use std::collections::HashSet;

use super::fs::RemoteFs;
use crate::files::gutter::{markers_from_hunks, GutterMarker};
use crate::files::{parse_status_z, Decoration};
use crate::worktree::diff::parse_unified_hunks;

const GIT_INDEX_CAP: usize = 50_000;

/// Monta o argv de um git remoto com as flags do princípio #8 (parsing
/// NUL-separated, sem cor, sem quoting do path). `-C <root>` ancora no repo.
fn git_argv<'a>(root: &'a str, sub: &[&'a str]) -> Vec<&'a str> {
    let mut argv = vec!["git", "-C", root, "-c", "core.quotePath=false"];
    argv.extend_from_slice(sub);
    argv
}

/// Resolve o repo root remoto: `git rev-parse --show-toplevel` no cwd. `None`
/// quando o cwd não está num repositório (a raiz cai no próprio cwd).
pub fn rev_parse_toplevel(fs: &dyn RemoteFs, cwd: &str) -> Option<String> {
    let argv = git_argv(cwd, &["rev-parse", "--show-toplevel"]);
    let out = fs.exec(&argv).ok()?;
    if !out.ok() {
        return None;
    }
    let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if top.is_empty() {
        None
    } else {
        Some(top)
    }
}

/// Decorações de working tree remotas: um `git status -z` por refresh.
pub fn remote_decorations(fs: &dyn RemoteFs, root: &str) -> Vec<Decoration> {
    let argv = git_argv(root, &["status", "--porcelain", "-z"]);
    match fs.exec(&argv) {
        Ok(out) if out.ok() => parse_status_z(&out.stdout),
        _ => Vec::new(),
    }
}

/// Índice de fuzzy remoto via `git ls-files` (rastreados + não rastreados, sem
/// gitignorados), com o mesmo teto de 50k e flag de truncado da fatia 2.
pub fn remote_ls_files(fs: &dyn RemoteFs, root: &str) -> (Vec<String>, bool) {
    let argv = git_argv(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    );
    let out = match fs.exec(&argv) {
        Ok(out) if out.ok() => out,
        _ => return (Vec::new(), false),
    };
    parse_ls_files(&out.stdout)
}

/// Parser NUL-separated do `git ls-files -z`. Nomes hostis (espaços, controle,
/// unicode) chegam crus — sem shell, sem parsing de texto. Aplica o teto.
pub fn parse_ls_files(raw: &[u8]) -> (Vec<String>, bool) {
    let mut names = Vec::new();
    let mut truncated = false;
    for field in raw.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        if names.len() >= GIT_INDEX_CAP {
            truncated = true;
            break;
        }
        names.push(String::from_utf8_lossy(field).replace('\\', "/"));
    }
    (names, truncated)
}

/// Subconjunto gitignorado de uma listagem, via um `git check-ignore` por dir
/// (só em contexto de repo). `--literal-pathspecs` trata nomes hostis como
/// literais. Exit 0 = há ignorados (stdout NUL-separated); 1 = nenhum; senão vazio.
pub fn remote_check_ignore(fs: &dyn RemoteFs, root: &str, rels: &[String]) -> HashSet<String> {
    if rels.is_empty() {
        return HashSet::new();
    }
    let mut argv: Vec<&str> = vec![
        "git",
        "-C",
        root,
        "-c",
        "core.quotePath=false",
        "--literal-pathspecs",
        "check-ignore",
        "-z",
        "--",
    ];
    for rel in rels {
        argv.push(rel.as_str());
    }
    match fs.exec(&argv) {
        Ok(out) if out.code == 0 => out
            .stdout
            .split(|b| *b == 0)
            .filter(|f| !f.is_empty())
            .map(|f| String::from_utf8_lossy(f).replace('\\', "/"))
            .collect(),
        _ => HashSet::new(),
    }
}

/// Gutter remoto: recalculado no save via `git diff HEAD -- <rel>`, mesmo
/// invocação da `range_file_hunks` local, executada no host.
pub fn remote_gutter(fs: &dyn RemoteFs, root: &str, rel: &str) -> Vec<GutterMarker> {
    let pathspec = format!(":(literal){rel}");
    let argv = git_argv(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-color",
            "-M",
            "HEAD",
            "--",
            &pathspec,
        ],
    );
    match fs.exec(&argv) {
        Ok(out) if out.ok() => {
            let hunks = parse_unified_hunks(&String::from_utf8_lossy(&out.stdout));
            markers_from_hunks(&hunks)
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::DecoStatus;

    #[test]
    fn parse_ls_files_splits_nul_and_normalizes_backslash() {
        let raw = b"src/lib.rs\0a b/c.txt\0weird\\name\0";
        let (names, truncated) = parse_ls_files(raw);
        assert_eq!(names, vec!["src/lib.rs", "a b/c.txt", "weird/name"]);
        assert!(!truncated);
    }

    #[test]
    fn parse_ls_files_tolerates_control_chars_in_names() {
        let raw = b"na\x01me\0file\twith\ttab\0";
        let (names, _) = parse_ls_files(raw);
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "na\x01me");
        assert_eq!(names[1], "file\twith\ttab");
    }

    #[test]
    fn parse_ls_files_applies_the_50k_cap() {
        let mut raw = Vec::new();
        for i in 0..(GIT_INDEX_CAP + 10) {
            raw.extend_from_slice(format!("f{i}").as_bytes());
            raw.push(0);
        }
        let (names, truncated) = parse_ls_files(&raw);
        assert_eq!(names.len(), GIT_INDEX_CAP);
        assert!(truncated);
    }

    #[test]
    fn remote_status_parse_matches_the_shared_nul_parser() {
        // Reusa o mesmo parser da fatia 1/2: entrada hostil com rename e untracked.
        let raw =
            "R  novo name.rs\0antigo.rs\0?? arquivo com espaço.txt\0 M src/keep.rs\0".as_bytes();
        let decos = parse_status_z(raw);
        let by: std::collections::HashMap<&str, DecoStatus> =
            decos.iter().map(|d| (d.path.as_str(), d.status)).collect();
        assert_eq!(by["novo name.rs"], DecoStatus::Renamed);
        assert_eq!(by["arquivo com espaço.txt"], DecoStatus::Untracked);
        assert_eq!(by["src/keep.rs"], DecoStatus::Modified);
        assert!(!by.contains_key("antigo.rs"), "origem do rename não decora");
    }
}
