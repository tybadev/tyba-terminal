//! Rich Input: injeção de prompt multiline em agentes CLI via bracketed paste.
//!
//! Regras da spec (swell-docs/tyba/features/rich-input/rules.md):
//! - payload sempre normalizado antes de tocar o PTY (anti terminal-injection)
//! - wrapper `ESC[200~ … ESC[201~` só com payload não-vazio e DECSET 2004 ativo
//! - o `\r` de submit vai num write SEPARADO, nunca concatenado ao payload
//! - `agent_match` é conveniência de UI, nunca autorização

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use parking_lot::RwLock;
use regex::Regex;
use serde::Serialize;

pub const DEFAULT_AGENT_PATTERN: &str = r"^(claude|codex|gemini)\b";
pub const SUBMIT_DELAY: Duration = Duration::from_millis(50);

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const PASTE_END_STR: &str = "\x1b[201~";

#[derive(Clone, Serialize)]
pub struct RichInputResult {
    pub injected_bytes: usize,
    pub stripped_control: bool,
    pub mentions_sensitive: bool,
}

pub fn normalize(text: &str) -> (String, bool) {
    let without_paste_end = text.replace(PASTE_END_STR, "");
    let without_esc = without_paste_end.replace('\x1b', "");
    let stripped = without_esc.len() != text.len();
    let normalized = without_esc.replace("\r\n", "\n").replace('\r', "\n");
    (normalized, stripped)
}

pub fn wrap(normalized: &str) -> Vec<u8> {
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut bytes = Vec::with_capacity(normalized.len() + PASTE_START.len() + PASTE_END.len());
    bytes.extend_from_slice(PASTE_START);
    bytes.extend_from_slice(normalized.as_bytes());
    bytes.extend_from_slice(PASTE_END);
    bytes
}

/// Heurística fraca de UX (badge amarelo + segundo clique). Nunca autoriza nem bloqueia.
const SENSITIVE_NEEDLES: &[&str] = &[
    "password",
    "senha",
    "secret",
    "token",
    "api key",
    "api_key",
    "apikey",
    "credential",
    "credencial",
    "private key",
    "chave privada",
    ".env",
];

pub fn mentions_sensitive(text: &str) -> bool {
    let lower = text.to_lowercase();
    SENSITIVE_NEEDLES.iter().any(|n| lower.contains(n))
}

pub struct AgentMatcher {
    regex: RwLock<Regex>,
}

impl AgentMatcher {
    pub fn new() -> Self {
        Self {
            regex: RwLock::new(
                Regex::new(DEFAULT_AGENT_PATTERN).expect("default agent pattern compiles"),
            ),
        }
    }

    pub fn set_pattern(&self, pattern: &str) -> bool {
        match Regex::new(pattern) {
            Ok(regex) => {
                *self.regex.write() = regex;
                true
            }
            Err(e) => {
                eprintln!("rich_input: pattern de agente inválido, mantendo o atual: {e}");
                false
            }
        }
    }

    pub fn matches(&self, cmdline: &str) -> bool {
        self.regex.read().is_match(cmdline)
    }
}

impl Default for AgentMatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn agent_matcher() -> &'static AgentMatcher {
    static MATCHER: OnceLock<AgentMatcher> = OnceLock::new();
    MATCHER.get_or_init(AgentMatcher::new)
}

pub fn plan_injection(normalized: &str, bracketed_paste: bool) -> Result<Vec<u8>, String> {
    if bracketed_paste {
        return Ok(wrap(normalized));
    }
    if normalized.contains('\n') {
        return Err(
            "multiline indisponível: a sessão não habilitou bracketed paste (DECSET 2004)".into(),
        );
    }
    Ok(normalized.as_bytes().to_vec())
}

pub fn worktree_files(root: &Path, query: &str, limit: usize) -> Result<Vec<String>, String> {
    let out = crate::worktree::git_in(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files falhou: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let files = out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Ok(rank_files(files, query, limit))
}

fn rank_files(mut files: Vec<String>, query: &str, limit: usize) -> Vec<String> {
    if query.is_empty() {
        files.sort_unstable();
        files.truncate(limit);
        return files;
    }
    let matcher = SkimMatcherV2::default().smart_case();
    let mut scored: Vec<(i64, String)> = files
        .into_iter()
        .filter_map(|f| matcher.fuzzy_match(&f, query).map(|score| (score, f)))
        .collect();
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(limit);
    scored.into_iter().map(|(_, f)| f).collect()
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize;

    #[test]
    fn texto_limpo_passa_intacto_e_nao_marca_strip() {
        assert_eq!(normalize("hello world"), ("hello world".into(), false));
    }

    #[test]
    fn remove_paste_end_embutido_no_payload() {
        let (out, stripped) = normalize("evil\x1b[201~\rrm -rf /");
        assert!(!out.contains('\x1b'));
        assert_eq!(out, "evil\nrm -rf /");
        assert!(stripped);
    }

    #[test]
    fn remove_todo_esc_restante() {
        let (out, stripped) = normalize("a\x1b[31mred");
        assert_eq!(out, "a[31mred");
        assert!(stripped);
    }

    #[test]
    fn paste_end_remontado_pela_remocao_nao_sobrevive_aos_dois_passos() {
        let (out, _) = normalize("\x1b\x1b[201~[201~x");
        assert!(!out.contains("\x1b[201~"));
        assert!(!out.contains('\x1b'));
    }

    #[test]
    fn normaliza_crlf_e_cr_para_lf_sem_marcar_strip() {
        assert_eq!(normalize("a\r\nb\rc"), ("a\nb\nc".into(), false));
    }

    #[test]
    fn cr_nunca_sobrevive_dentro_do_payload() {
        let (out, _) = normalize("linha1\rlinha2\r\n\r");
        assert!(!out.contains('\r'));
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::wrap;

    #[test]
    fn payload_vazio_nao_emite_wrapper() {
        assert!(wrap("").is_empty());
    }

    #[test]
    fn payload_e_embrulhado_em_bracketed_paste() {
        assert_eq!(wrap("hi\nthere"), b"\x1b[200~hi\nthere\x1b[201~".to_vec());
    }
}

#[cfg(test)]
mod plan_injection_tests {
    use super::plan_injection;

    #[test]
    fn com_bracketed_paste_o_payload_vai_embrulhado() {
        assert_eq!(
            plan_injection("a\nb", true).unwrap(),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
    }

    #[test]
    fn sem_bracketed_paste_single_line_vai_como_texto_puro() {
        assert_eq!(plan_injection("ls -la", false).unwrap(), b"ls -la".to_vec());
    }

    #[test]
    fn sem_bracketed_paste_multiline_e_recusado() {
        let err = plan_injection("a\nb", false).unwrap_err();
        assert!(err.contains("multiline"));
    }

    #[test]
    fn payload_vazio_nao_gera_bytes_em_nenhum_modo() {
        assert!(plan_injection("", true).unwrap().is_empty());
        assert!(plan_injection("", false).unwrap().is_empty());
    }
}

#[cfg(test)]
mod agent_matcher_tests {
    use super::{AgentMatcher, DEFAULT_AGENT_PATTERN};

    #[test]
    fn default_casa_agentes_conhecidos_por_fronteira_de_palavra() {
        let m = AgentMatcher::new();
        assert!(m.matches("claude"));
        assert!(m.matches("codex --resume abc"));
        assert!(m.matches("gemini chat"));
        assert!(!m.matches("codexx"));
        assert!(!m.matches("myclaude"));
        assert!(!m.matches("vim"));
    }

    #[test]
    fn pattern_invalido_e_rejeitado_e_matcher_anterior_continua_valendo() {
        let m = AgentMatcher::new();
        assert!(!m.set_pattern("(("));
        assert!(m.matches("codex"));
        assert_eq!(DEFAULT_AGENT_PATTERN, r"^(claude|codex|gemini)\b");
    }

    #[test]
    fn pattern_valido_substitui_o_default() {
        let m = AgentMatcher::new();
        assert!(m.set_pattern(r"^(aider)\b"));
        assert!(m.matches("aider --model x"));
        assert!(!m.matches("codex"));
    }
}

#[cfg(test)]
mod mentions_sensitive_tests {
    use super::mentions_sensitive;

    #[test]
    fn detecta_termos_sensiveis_sem_case() {
        assert!(mentions_sensitive("my PassWord is hunter2"));
        assert!(mentions_sensitive("cole o API key aqui"));
        assert!(mentions_sensitive("leia o .env e me diga"));
        assert!(mentions_sensitive("minha senha do banco"));
    }

    #[test]
    fn texto_comum_nao_dispara() {
        assert!(!mentions_sensitive("refatora o parser de diff"));
    }
}

#[cfg(test)]
mod worktree_files_tests {
    use super::worktree_files;
    use std::process::Command;

    fn temp_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-richinput-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join("main.rs"), "x").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "x").unwrap();
        std::fs::write(dir.join("untracked.txt"), "x").unwrap();
        std::fs::write(dir.join("ignored.log"), "x").unwrap();
        std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
        git(&["add", "main.rs", "src/lib.rs", ".gitignore"]);
        dir
    }

    #[test]
    fn lista_rastreados_e_untracked_mas_nunca_ignorados() {
        let repo = temp_repo();

        let files = worktree_files(&repo, "", 50).unwrap();

        assert!(files.contains(&"main.rs".to_string()));
        assert!(files.contains(&"src/lib.rs".to_string()));
        assert!(files.contains(&"untracked.txt".to_string()));
        assert!(!files.iter().any(|f| f.ends_with("ignored.log")));
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn query_fuzzy_prioriza_o_match_e_respeita_o_limite() {
        let repo = temp_repo();

        let files = worktree_files(&repo, "mainrs", 1).unwrap();

        assert_eq!(files, vec!["main.rs".to_string()]);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn fora_de_repo_git_retorna_erro() {
        let dir = std::env::temp_dir().join(format!("tyba-notrepo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let result = worktree_files(&dir, "", 10);

        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
