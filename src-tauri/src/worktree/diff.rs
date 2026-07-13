//! Diff local da sessão ("PR review" sem GitHub) — leva B da Fase 3.
//!
//! Three-dot semantics: sempre `base_ref..HEAD` com o `base_ref` gravado
//! na criação do worktree (princípio #7). Saída de git com `-z` e parsing
//! NUL-separated (princípio #8). Hunks são lazy por arquivo: lockfiles
//! geram dezenas de milhares de linhas e nunca entram no payload da lista.

use std::path::Path;

use serde::Serialize;

use super::{git_in, run_git};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub subject: String,
    pub authored_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Other,
}

impl FileStatus {
    fn from_letter(letter: u8) -> Self {
        match letter {
            b'A' => Self::Added,
            b'M' => Self::Modified,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// `None` = binário (numstat emite `-`).
    pub insertions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionDiff {
    /// Raiz do repo que gerou este diff — a sessão troca de repo com `cd`,
    /// e o front precisa invalidar cache/estado quando o root muda.
    pub root: String,
    pub base_ref: String,
    pub commits: Vec<CommitInfo>,
    pub files: Vec<FileDiff>,
    /// Index vs HEAD (`git diff --cached`).
    pub staged_files: Vec<FileDiff>,
    /// Worktree vs index (`git diff`) + untracked.
    pub unstaged_files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileHunks {
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

fn nul_fields(raw: &[u8]) -> impl Iterator<Item = String> + '_ {
    raw.split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
}

fn parse_count(field: &str) -> Option<u32> {
    if field == "-" {
        None
    } else {
        field.parse().ok()
    }
}

pub struct NumstatEntry {
    pub old_path: Option<String>,
    pub path: String,
    pub insertions: Option<u32>,
    pub deletions: Option<u32>,
}

/// `git diff --numstat -z -M`: registros `ins\tdel\tpath\0`; renames vêm
/// como `ins\tdel\t\0old\0new\0` (path vazio no registro, dois campos extra).
pub fn parse_numstat_z(raw: &[u8]) -> Vec<NumstatEntry> {
    let mut out = Vec::new();
    let mut fields = raw.split(|b| *b == 0).filter(|f| !f.is_empty());
    let mut pending: Option<(Option<u32>, Option<u32>, bool)> = None;
    let mut pending_old: Option<String> = None;

    for field in fields.by_ref() {
        let field = String::from_utf8_lossy(field).into_owned();
        match (&mut pending, &mut pending_old) {
            (None, _) => {
                let mut parts = field.splitn(3, '\t');
                let ins = parse_count(parts.next().unwrap_or_default());
                let del = parse_count(parts.next().unwrap_or_default());
                match parts.next() {
                    Some("") | None => {
                        pending = Some((ins, del, true));
                    }
                    Some(path) => out.push(NumstatEntry {
                        old_path: None,
                        path: path.to_string(),
                        insertions: ins,
                        deletions: del,
                    }),
                }
            }
            (Some(_), None) => {
                pending_old = Some(field);
            }
            (Some((ins, del, _)), Some(old)) => {
                out.push(NumstatEntry {
                    old_path: Some(old.clone()),
                    path: field,
                    insertions: *ins,
                    deletions: *del,
                });
                pending = None;
                pending_old = None;
            }
        }
    }
    out
}

/// `git diff --name-status -z -M`: `S\0path\0` | `R<score>\0old\0new\0`.
pub fn parse_name_status_z(raw: &[u8]) -> Vec<(FileStatus, Option<String>, String)> {
    let mut out = Vec::new();
    let mut fields = nul_fields(raw);
    while let Some(status) = fields.next() {
        let letter = status.as_bytes().first().copied().unwrap_or(b'?');
        let status = FileStatus::from_letter(letter);
        match status {
            FileStatus::Renamed | FileStatus::Copied => {
                let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                    break;
                };
                out.push((status, Some(old), new));
            }
            _ => {
                let Some(path) = fields.next() else { break };
                out.push((status, None, path));
            }
        }
    }
    out
}

/// `git log -z --format=%H%x1f%s%x1f%aI`: registros por NUL, campos por US.
/// Subject pode conter qualquer byte exceto NUL/newline — inclusive o
/// separador; por isso sha é o primeiro campo, data o último, e o
/// subject é tudo que sobra no meio.
pub fn parse_log_z(raw: &[u8]) -> Vec<CommitInfo> {
    raw.split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let parts: Vec<&str> = text.split('\x1f').collect();
            if parts.len() < 3 {
                return None;
            }
            Some(CommitInfo {
                sha: parts[0].trim().to_string(),
                subject: parts[1..parts.len() - 1].join("\x1f"),
                authored_at: parts[parts.len() - 1].to_string(),
            })
        })
        .collect()
}

/// Parser do unified diff de UM arquivo (`git diff <range> -- <path>`).
pub fn parse_unified_hunks(text: &str) -> FileHunks {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut binary = false;

    let mut past_first_file = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            if past_first_file {
                break;
            }
            past_first_file = true;
            continue;
        }
        if line.starts_with("Binary files ") && line.ends_with(" differ") {
            binary = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@") {
            let header = line.to_string();
            let ranges = rest.split("@@").next().unwrap_or_default().trim();
            let mut old = (0u32, 1u32);
            let mut new = (0u32, 1u32);
            for range in ranges.split_whitespace() {
                let (sign, body) = range.split_at(1);
                let mut nums = body.splitn(2, ',');
                let start = nums.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                let count = nums.next().and_then(|n| n.parse().ok()).unwrap_or(1);
                match sign {
                    "-" => old = (start, count),
                    "+" => new = (start, count),
                    _ => {}
                }
            }
            hunks.push(Hunk {
                old_start: old.0,
                old_lines: old.1,
                new_start: new.0,
                new_lines: new.1,
                header,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            continue;
        };
        let kind = match line.as_bytes().first() {
            Some(b'+') => LineKind::Add,
            Some(b'-') => LineKind::Del,
            Some(b' ') => LineKind::Context,
            Some(b'\\') => continue,
            _ => continue,
        };
        hunk.lines.push(DiffLine {
            kind,
            text: line[1..].to_string(),
        });
    }

    FileHunks { binary, hunks }
}

fn diff_files(worktree: &Path, range_args: &[&str]) -> Result<Vec<FileDiff>, String> {
    let numstat = run_git(
        {
            let mut c = git_in(worktree);
            c.args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--numstat",
                "-z",
                "-M",
            ]);
            c.args(range_args);
            c
        },
        "git diff --numstat",
    )?;
    let name_status = run_git(
        {
            let mut c = git_in(worktree);
            c.args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--name-status",
                "-z",
                "-M",
            ]);
            c.args(range_args);
            c
        },
        "git diff --name-status",
    )?;

    let counts: std::collections::HashMap<String, (Option<u32>, Option<u32>)> =
        parse_numstat_z(&numstat)
            .into_iter()
            .map(|e| (e.path, (e.insertions, e.deletions)))
            .collect();

    Ok(parse_name_status_z(&name_status)
        .into_iter()
        .map(|(status, old_path, path)| {
            let (insertions, deletions) = counts.get(&path).copied().unwrap_or((Some(0), Some(0)));
            FileDiff {
                path,
                old_path,
                status,
                insertions,
                deletions,
            }
        })
        .collect())
}

pub fn session_diff(worktree: &Path, base_ref: &str) -> Result<SessionDiff, String> {
    let range = format!("{base_ref}..HEAD");

    // As quatro consultas são independentes: rodam em paralelo em vez de ~8
    // processos git em série. Com a jaula, cada chamada carrega o custo do
    // sandbox-exec; paralelizar mantém o wall-clock no maior ramo, não na soma.
    let (log, files, staged_files, unstaged_files) = std::thread::scope(|s| {
        let log = s.spawn(|| {
            run_git(
                {
                    let mut c = git_in(worktree);
                    c.args(["log", "-z", "--format=%H%x1f%s%x1f%aI", &range]);
                    c
                },
                "git log",
            )
        });
        let files = s.spawn(|| diff_files(worktree, &[&range]));
        let staged = s.spawn(|| diff_files(worktree, &["--cached"]));
        let unstaged = s.spawn(|| {
            let mut f = diff_files(worktree, &[])?;
            f.extend(untracked_files(worktree)?);
            Ok::<_, String>(f)
        });
        (
            log.join().expect("log thread"),
            files.join().expect("files thread"),
            staged.join().expect("staged thread"),
            unstaged.join().expect("unstaged thread"),
        )
    });

    let (commits, files) = match (log, files) {
        (Ok(log), files) => (parse_log_z(&log), files?),
        (Err(_), _) if head_is_unborn(worktree) => (Vec::new(), Vec::new()),
        (Err(e), _) => return Err(e),
    };

    Ok(SessionDiff {
        root: worktree.to_string_lossy().into_owned(),
        base_ref: base_ref.to_string(),
        commits,
        files,
        staged_files: staged_files?,
        unstaged_files: unstaged_files?,
    })
}

/// Diff read-only de uma branch contra a base (modo explorar do painel):
/// só o committed do range — staged/unstaged são do working tree, não da
/// branch olhada.
pub fn branch_diff(worktree: &Path, base_ref: &str, branch: &str) -> Result<SessionDiff, String> {
    let range = format!("{base_ref}..{branch}");
    let (log, files) = std::thread::scope(|s| {
        let log = s.spawn(|| {
            run_git(
                {
                    let mut c = git_in(worktree);
                    c.args(["log", "-z", "--format=%H%x1f%s%x1f%aI", &range]);
                    c
                },
                "git log",
            )
        });
        let files = s.spawn(|| diff_files(worktree, &[&range]));
        (
            log.join().expect("log thread"),
            files.join().expect("files thread"),
        )
    });

    Ok(SessionDiff {
        root: worktree.to_string_lossy().into_owned(),
        base_ref: base_ref.to_string(),
        commits: parse_log_z(&log?),
        files: files?,
        staged_files: Vec::new(),
        unstaged_files: Vec::new(),
    })
}

pub fn range_file_hunks(
    worktree: &Path,
    range: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileHunks, String> {
    let out = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["diff", "--no-ext-diff", "--no-color", "-M"]);
            c.arg(range);
            c.arg("--").arg(format!(":(literal){path}"));
            if let Some(old) = old_path {
                c.arg(format!(":(literal){old}"));
            }
            c
        },
        "git diff",
    )?;
    Ok(parse_unified_hunks(&String::from_utf8_lossy(&out)))
}

fn head_is_unborn(worktree: &Path) -> bool {
    run_git(
        {
            let mut c = git_in(worktree);
            c.args(["rev-parse", "--verify", "--quiet", "HEAD"]);
            c
        },
        "git rev-parse",
    )
    .is_err()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    Committed,
    Staged,
    Unstaged,
}

pub fn file_hunks(
    worktree: &Path,
    base_ref: &str,
    scope: DiffScope,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileHunks, String> {
    let range = format!("{base_ref}..HEAD");
    let out = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["diff", "--no-ext-diff", "--no-color", "-M"]);
            match scope {
                DiffScope::Committed => {
                    c.arg(&range);
                }
                DiffScope::Staged => {
                    c.arg("--cached");
                }
                DiffScope::Unstaged => {}
            };
            c.arg("--").arg(format!(":(literal){path}"));
            if let Some(old) = old_path {
                c.arg(format!(":(literal){old}"));
            }
            c
        },
        "git diff",
    )?;
    // Diff vazio no escopo unstaged pode ser untracked (git diff não
    // enxerga) — mas só cai no --no-index se o arquivo é untracked mesmo,
    // senão um tracked limpo voltaria inteiro como adição.
    if out.is_empty() && scope == DiffScope::Unstaged && is_untracked(worktree, path)? {
        return untracked_hunks(worktree, path);
    }
    Ok(parse_unified_hunks(&String::from_utf8_lossy(&out)))
}

fn is_untracked(worktree: &Path, path: &str) -> Result<bool, String> {
    let out = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["ls-files", "--others", "--exclude-standard", "-z", "--"]);
            c.arg(format!(":(literal){path}"));
            c
        },
        "git ls-files",
    )?;
    Ok(!out.is_empty())
}

/// `git diff HEAD` não enxerga arquivo untracked; o conteúdo dele é
/// exatamente o artefato mais arriscado de um agente. `--no-index` contra
/// /dev/null devolve o arquivo inteiro como hunk de adição (exit 1 =
/// "tem diferença", não erro).
fn untracked_hunks(worktree: &Path, path: &str) -> Result<FileHunks, String> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let out = {
        let mut c = git_in(worktree);
        c.args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--no-index",
            "--",
            null,
        ]);
        c.arg(path);
        c.output()
            .map_err(|e| format!("git diff --no-index: {e}"))?
    };
    if !out.status.success() && out.status.code() != Some(1) {
        return Err(format!(
            "git diff --no-index: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_unified_hunks(&String::from_utf8_lossy(&out.stdout)))
}

const UNTRACKED_COUNT_MAX_BYTES: u64 = 1024 * 1024;

fn untracked_files(worktree: &Path) -> Result<Vec<FileDiff>, String> {
    let raw = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["ls-files", "--others", "--exclude-standard", "-z"]);
            c
        },
        "git ls-files",
    )?;
    Ok(nul_fields(&raw)
        .map(|path| {
            let full = worktree.join(&path);
            let insertions = std::fs::metadata(&full)
                .ok()
                .filter(|m| m.is_file() && m.len() <= UNTRACKED_COUNT_MAX_BYTES)
                .and_then(|_| std::fs::read(&full).ok())
                .filter(|content| !content.contains(&0))
                .map(|content| {
                    let mut count = content.iter().filter(|b| **b == b'\n').count() as u32;
                    if !content.is_empty() && content.last() != Some(&b'\n') {
                        count += 1;
                    }
                    count
                });
            FileDiff {
                path,
                old_path: None,
                status: FileStatus::Added,
                insertions,
                deletions: insertions.map(|_| 0),
            }
        })
        .collect())
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn numstat_parses_plain_and_binary_entries() {
        let raw = b"3\t1\tsrc/main.rs\0-\t-\tlogo.png\0";
        let parsed = parse_numstat_z(raw);
        assert_eq!(parsed[0].path, "src/main.rs");
        assert_eq!(
            (parsed[0].insertions, parsed[0].deletions),
            (Some(3), Some(1))
        );
        assert_eq!(parsed[1].path, "logo.png");
        assert_eq!((parsed[1].insertions, parsed[1].deletions), (None, None));
    }

    #[test]
    fn numstat_parses_rename_records() {
        let raw = b"5\t0\t\0old name.rs\0new name.rs\0";
        let parsed = parse_numstat_z(raw);
        assert_eq!(parsed[0].old_path.as_deref(), Some("old name.rs"));
        assert_eq!(parsed[0].path, "new name.rs");
        assert_eq!(
            (parsed[0].insertions, parsed[0].deletions),
            (Some(5), Some(0))
        );
    }

    #[test]
    fn name_status_parses_rename_with_score() {
        let raw = b"M\0a.rs\0R100\0antigo.rs\0novo.rs\0A\0c om espa\xc3\xa7o.txt\0";
        let parsed = parse_name_status_z(raw);
        assert_eq!(parsed[0], (FileStatus::Modified, None, "a.rs".into()));
        assert_eq!(
            parsed[1],
            (
                FileStatus::Renamed,
                Some("antigo.rs".into()),
                "novo.rs".into()
            )
        );
        assert_eq!(
            parsed[2],
            (FileStatus::Added, None, "c om espaço.txt".into())
        );
    }

    #[test]
    fn log_parses_records_with_unit_separator() {
        let raw = b"abc123\x1ffeat: algo\x1f2026-07-10T10:00:00-03:00\0def456\x1ffix: outra \x1f coisa\x1f2026-07-09T09:00:00-03:00\0";
        let commits = parse_log_z(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "abc123");
        assert_eq!(commits[0].subject, "feat: algo");
        assert_eq!(
            commits[1].subject, "fix: outra \x1f coisa",
            "subject com o separador embutido não pode engolir a data"
        );
        assert_eq!(commits[1].authored_at, "2026-07-09T09:00:00-03:00");
    }

    #[test]
    fn unified_hunks_parse_ranges_lines_and_no_newline_marker() {
        let text = "diff --git a/f.txt b/f.txt\nindex 000..111 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,4 @@ contexto\n linha1\n-removida\n+adicionada\n+outra\n\\ No newline at end of file\n";
        let parsed = parse_unified_hunks(text);
        assert!(!parsed.binary);
        assert_eq!(parsed.hunks.len(), 1);
        let hunk = &parsed.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
        assert_eq!((hunk.new_start, hunk.new_lines), (1, 4));
        assert_eq!(hunk.lines.len(), 4);
        assert_eq!(hunk.lines[1].kind, LineKind::Del);
        assert_eq!(hunk.lines[2].text, "adicionada");
    }

    #[test]
    fn second_file_headers_never_leak_into_the_previous_hunk() {
        let text = "diff --git a/um.txt b/um.txt\n--- a/um.txt\n+++ b/um.txt\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/dois.txt b/dois.txt\n--- a/dois.txt\n+++ b/dois.txt\n@@ -1 +1 @@\n-c\n+d\n";
        let parsed = parse_unified_hunks(text);
        assert_eq!(parsed.hunks.len(), 1, "só o primeiro arquivo entra");
        let texts: Vec<&str> = parsed.hunks[0]
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(texts, vec!["a", "b"]);
    }

    #[test]
    fn unified_hunks_flag_binary_files() {
        let parsed = parse_unified_hunks("Binary files a/logo.png and b/logo.png differ\n");
        assert!(parsed.binary);
        assert!(parsed.hunks.is_empty());
    }

    #[test]
    fn single_line_hunk_omits_the_count() {
        let text = "@@ -5 +5 @@\n-a\n+b\n";
        let parsed = parse_unified_hunks(text);
        let hunk = &parsed.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (5, 1));
        assert_eq!((hunk.new_start, hunk.new_lines), (5, 1));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn temp_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-diff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@t.com"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "um\ndois\ntres\n").unwrap();
        std::fs::write(dir.join("renomeia.txt"), "conteudo estavel\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn session_diff_on_unborn_head_shows_only_working_tree_state() {
        let dir = std::env::temp_dir().join(format!("tyba-diff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@t.com"]);
        git(&dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("staged.txt"), "conteudo\n").unwrap();
        git(&dir, &["add", "staged.txt"]);
        std::fs::write(dir.join("solto.txt"), "linha\n").unwrap();

        let diff = session_diff(&dir, "HEAD").expect("session_diff em repo sem commit");
        assert!(diff.commits.is_empty());
        assert!(diff.files.is_empty());
        assert_eq!(diff.staged_files.len(), 1);
        assert_eq!(diff.staged_files[0].path, "staged.txt");
        assert!(diff.unstaged_files.iter().any(|f| f.path == "solto.txt"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_diff_covers_commits_files_renames_and_dirty_state() {
        let repo = temp_repo();
        let base = super::super::head_sha(&repo).unwrap();

        std::fs::write(repo.join("a.txt"), "um\ndois alterado\ntres\nquatro\n").unwrap();
        git(&repo, &["mv", "renomeia.txt", "novo-nome.txt"]);
        std::fs::write(repo.join("novo.txt"), "novinho\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "feat: mudancas"]);
        std::fs::write(repo.join("sujo.txt"), "não commitado\n").unwrap();
        git(&repo, &["add", "sujo.txt"]);

        let diff = session_diff(&repo, &base).expect("session_diff");

        assert_eq!(diff.commits.len(), 1);
        assert_eq!(diff.commits[0].subject, "feat: mudancas");
        // sujo.txt foi addado (staged), nada unstaged sobrou.
        assert_eq!(diff.staged_files.len(), 1);
        assert_eq!(diff.staged_files[0].path, "sujo.txt");
        assert!(diff.unstaged_files.is_empty());

        let by_path: std::collections::HashMap<&str, &FileDiff> =
            diff.files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(by_path["a.txt"].status, FileStatus::Modified);
        assert_eq!(by_path["a.txt"].insertions, Some(2));
        assert_eq!(by_path["novo.txt"].status, FileStatus::Added);
        let renamed = by_path["novo-nome.txt"];
        assert_eq!(renamed.status, FileStatus::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some("renomeia.txt"));

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn untracked_file_appears_in_uncommitted_and_has_reviewable_hunks() {
        let repo = temp_repo();
        let base = super::super::head_sha(&repo).unwrap();
        std::fs::write(repo.join("nunca-addado.sh"), "linha um\nlinha dois\n").unwrap();

        let diff = session_diff(&repo, &base).expect("session_diff");
        assert!(diff.staged_files.is_empty());
        let untracked = diff
            .unstaged_files
            .iter()
            .find(|f| f.path == "nunca-addado.sh")
            .expect("untracked na lista");
        assert_eq!(untracked.status, FileStatus::Added);
        assert_eq!(untracked.insertions, Some(2));

        let hunks = file_hunks(&repo, &base, DiffScope::Unstaged, "nunca-addado.sh", None).unwrap();
        assert_eq!(hunks.hunks.len(), 1);
        assert!(hunks.hunks[0].lines.iter().all(|l| l.kind == LineKind::Add));
        assert_eq!(hunks.hunks[0].lines.len(), 2);

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn glob_named_file_does_not_leak_sibling_hunks() {
        let repo = temp_repo();
        let base = super::super::head_sha(&repo).unwrap();
        std::fs::write(repo.join("a0.txt"), "sib\n").unwrap();
        std::fs::write(repo.join("a[0].txt"), "glob\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "dois arquivos"]);

        let hunks = file_hunks(&repo, &base, DiffScope::Committed, "a[0].txt", None).unwrap();
        let added: Vec<&str> = hunks
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Add)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            added,
            vec!["glob"],
            "pathspec com glob vazou arquivo vizinho"
        );

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn renamed_file_hunks_show_the_edit_not_the_whole_file() {
        let repo = temp_repo();
        let base = super::super::head_sha(&repo).unwrap();
        git(&repo, &["mv", "renomeia.txt", "novo-nome.txt"]);
        std::fs::write(repo.join("novo-nome.txt"), "conteudo estavel\neditado\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "rename com edit"]);

        let hunks = file_hunks(
            &repo,
            &base,
            DiffScope::Committed,
            "novo-nome.txt",
            Some("renomeia.txt"),
        )
        .unwrap();
        let adds = hunks
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Add)
            .count();
        assert_eq!(
            adds, 1,
            "rename devia mostrar só o edit, não o arquivo inteiro"
        );

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn file_hunks_are_lazy_per_file_and_scope() {
        let repo = temp_repo();
        let base = super::super::head_sha(&repo).unwrap();

        std::fs::write(repo.join("a.txt"), "um\ndois alterado\ntres\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "muda a"]);
        std::fs::write(repo.join("a.txt"), "um\ndois alterado\ntres\nsujo\n").unwrap();

        let committed = file_hunks(&repo, &base, DiffScope::Committed, "a.txt", None).unwrap();
        assert_eq!(committed.hunks.len(), 1);
        assert!(committed.hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Add && l.text == "dois alterado"));
        assert!(!committed.hunks[0].lines.iter().any(|l| l.text == "sujo"));

        let unstaged = file_hunks(&repo, &base, DiffScope::Unstaged, "a.txt", None).unwrap();
        assert!(unstaged.hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Add && l.text == "sujo"));

        git(&repo, &["add", "a.txt"]);
        let staged = file_hunks(&repo, &base, DiffScope::Staged, "a.txt", None).unwrap();
        assert!(staged.hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Add && l.text == "sujo"));
        let unstaged = file_hunks(&repo, &base, DiffScope::Unstaged, "a.txt", None).unwrap();
        assert!(unstaged.hunks.is_empty());

        std::fs::remove_dir_all(&repo).ok();
    }
}
