//! "Buscar no projeto" da tela de Arquivos: texto dentro dos arquivos
//! rastreados, via `git grep` (só leitura, na jaula de leitura do git).
//!
//! Arquivo de segredo (`.env`, chave, credencial) nunca aparece no resultado,
//! e o trecho mostrado passa pela redação de segredos antes de sair do core.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

use serde::Serialize;

/// Menos que isso casa meio repo e não ajuda ninguém.
pub const MIN_QUERY: usize = 2;
/// Trecho de linha que vai para a tela.
const MAX_TEXT: usize = 240;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrepHit {
    pub path: String,
    /// 1-based.
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GrepOutcome {
    pub hits: Vec<GrepHit>,
    pub truncated: bool,
}

/// Mesmo critério do `lib/sensitivePaths` do webview: casa pelo nome.
pub fn is_sensitive(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.starts_with("id_rsa")
        || name.contains("credentials")
        || name.ends_with(".p12")
        || name == ".npmrc"
        || name == ".pypirc"
}

/// Uma linha de `git grep -n -z`: `caminho\0linha\0texto`.
pub fn parse_line(raw: &str) -> Option<GrepHit> {
    let mut parts = raw.splitn(3, '\0');
    let path = parts.next()?;
    let line = parts.next()?.parse().ok()?;
    let text = parts.next()?;
    if path.is_empty() || is_sensitive(path) {
        return None;
    }
    let text = crate::session::redact::redact(text.trim_end_matches('\r'));
    let text: String = text.chars().take(MAX_TEXT).collect();
    Some(GrepHit {
        path: path.to_string(),
        line,
        text,
    })
}

/// Busca `query` (texto literal, sem caixa) nos arquivos rastreados de `root`.
/// Para de ler em `limit` resultados.
pub fn grep(root: &Path, query: &str, limit: usize) -> Result<GrepOutcome, String> {
    if query.trim().chars().count() < MIN_QUERY {
        return Ok(GrepOutcome {
            hits: Vec::new(),
            truncated: false,
        });
    }
    let mut child = crate::worktree::git_in(root)
        .args(["grep", "-n", "-I", "-z", "--no-color", "-F", "-i", "-e"])
        .arg(query)
        .args(["--", "."])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git grep: {e}"))?;
    let stdout = child.stdout.take().ok_or("git grep sem saída")?;
    let mut hits = Vec::new();
    let mut truncated = false;
    let mut read_all = true;
    for line in BufReader::new(stdout).split(b'\n') {
        let Ok(line) = line else {
            read_all = false;
            break;
        };
        let Some(hit) = parse_line(&String::from_utf8_lossy(&line)) else {
            continue;
        };
        if hits.len() == limit {
            truncated = true;
            read_all = false;
            break;
        }
        hits.push(hit);
    }
    if !read_all {
        // Saiu antes do fim: não espera o resto do repo.
        let _ = child.kill();
        let _ = child.wait();
        return Ok(GrepOutcome { hits, truncated });
    }
    let status = child.wait().map_err(|e| format!("git grep: {e}"))?;
    exit_outcome(status.code())?;
    Ok(GrepOutcome { hits, truncated })
}

/// Status de saída do `git grep` lido até o fim: 0 achou, 1 não casou nada;
/// 128 é "não é repositório git"; o resto é falha.
fn exit_outcome(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(0 | 1) => Ok(()),
        Some(128) => Err("a busca no projeto exige um repositório git".into()),
        Some(c) => Err(format!("git grep falhou (status {c})")),
        None => Err("git grep interrompido".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nul_separated_line() {
        let hit = parse_line("src/server.js\x008\x00  res.json({ ok: true });").unwrap();
        assert_eq!(hit.path, "src/server.js");
        assert_eq!(hit.line, 8);
        assert_eq!(hit.text, "  res.json({ ok: true });");
    }

    #[test]
    fn keeps_colons_and_nul_free_text_intact() {
        let hit = parse_line("a b/ç.rs\x0012\x00let x: u8 = 1; // a:b").unwrap();
        assert_eq!(hit.path, "a b/ç.rs");
        assert_eq!(hit.text, "let x: u8 = 1; // a:b");
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(parse_line("").is_none());
        assert!(parse_line("only-path").is_none());
        assert!(parse_line("p\x00notanumber\x00x").is_none());
    }

    #[test]
    fn sensitive_files_never_show_up() {
        assert!(parse_line(".env\x001\x00API_KEY=abc").is_none());
        assert!(parse_line("config/.env.local\x001\x00X=1").is_none());
        assert!(parse_line("certs/server.pem\x001\x00---").is_none());
        assert!(parse_line("aws/credentials\x001\x00x").is_none());
        assert!(!is_sensitive("src/environment.ts"));
    }

    #[test]
    fn secrets_in_matched_text_are_redacted() {
        let hit = parse_line("src/a.ts\x003\x00const k = \"AKIAIOSFODNN7EXAMPLE\";").unwrap();
        assert!(!hit.text.contains("AKIAIOSFODNN7EXAMPLE"), "{}", hit.text);
    }

    #[test]
    fn long_text_is_cut() {
        let long = "x".repeat(1000);
        let hit = parse_line(&format!("a.rs\x001\x00{long}")).unwrap();
        assert_eq!(hit.text.chars().count(), MAX_TEXT);
    }

    #[test]
    fn short_query_returns_nothing_without_running_git() {
        let out = grep(Path::new("/nonexistent"), "a", 10).unwrap();
        assert!(out.hits.is_empty());
    }

    #[test]
    fn exit_status_maps_to_outcome() {
        assert!(exit_outcome(Some(0)).is_ok());
        assert!(exit_outcome(Some(1)).is_ok());
        assert!(exit_outcome(Some(128))
            .unwrap_err()
            .contains("repositório git"));
        assert!(exit_outcome(Some(2)).is_err());
        assert!(exit_outcome(None).is_err());
    }

    #[test]
    fn outside_a_git_repo_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alfa\n").unwrap();
        let err = grep(dir.path(), "alfa", 10).unwrap_err();
        assert!(err.contains("repositório git"), "{err}");
    }

    #[test]
    fn no_match_in_repo_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("a.txt"), "beta\n").unwrap();
        git(&["add", "a.txt"]);
        let out = grep(root, "alfa", 10).unwrap();
        assert!(out.hits.is_empty());
    }

    #[test]
    fn greps_a_real_repo_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("a.txt"), "Alfa\nbeta alfa\n").unwrap();
        std::fs::write(root.join(".env"), "ALFA=1\n").unwrap();
        git(&["add", "-f", "a.txt", ".env"]);
        let out = grep(root, "alfa", 10).unwrap();
        assert_eq!(out.hits.len(), 2, "{:?}", out.hits);
        assert!(out.hits.iter().all(|h| h.path == "a.txt"));
        assert!(!out.truncated);
        let capped = grep(root, "alfa", 1).unwrap();
        assert_eq!(capped.hits.len(), 1);
        assert!(capped.truncated);
    }
}
