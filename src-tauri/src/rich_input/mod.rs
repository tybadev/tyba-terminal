use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use parking_lot::{Mutex, RwLock};
use regex::Regex;

pub const DEFAULT_AGENT_PATTERN: &str = r"^(claude|codex|gemini)\b";
pub const SUBMIT_DELAY: Duration = Duration::from_millis(50);

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";
const PASTE_END_STR: &str = "\x1b[201~";

fn is_allowed(c: char) -> bool {
    match c {
        '\n' | '\t' => true,
        c if c.is_control() => false,
        _ => true,
    }
}

pub fn normalize(text: &str) -> (String, bool) {
    let unwrapped = text.replace(PASTE_END_STR, "");
    let newlines = unwrapped.replace("\r\n", "\n").replace('\r', "\n");
    let normalized: String = newlines.chars().filter(|c| is_allowed(*c)).collect();
    let stripped = newlines.chars().any(|c| !is_allowed(c)) || unwrapped.len() != text.len();
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
        let pattern = if pattern.trim().is_empty() {
            DEFAULT_AGENT_PATTERN
        } else {
            pattern
        };
        match Regex::new(pattern) {
            Ok(regex) => {
                *self.regex.write() = regex;
                true
            }
            Err(_) => false,
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

pub fn rel_path(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

const FILES_CACHE_TTL: Duration = Duration::from_secs(5);

fn load_worktree_files(root: &Path, base: &Path) -> Result<Vec<String>, String> {
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
    let mut files: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let absolute: PathBuf = root.join(String::from_utf8_lossy(s).as_ref());
            rel_path(&absolute, base)
        })
        .collect();
    files.sort_unstable();
    Ok(files)
}

struct CachedFiles {
    files: Arc<Vec<String>>,
    at: Instant,
}

#[derive(Default)]
pub struct FilesCache {
    lists: Mutex<HashMap<PathBuf, CachedFiles>>,
}

impl FilesCache {
    pub fn files_for(&self, cwd: &Path, query: &str, limit: usize) -> Result<Vec<String>, String> {
        let cwd = crate::repo::canonicalize_or(cwd);
        let cached = {
            let mut lists = self.lists.lock();
            lists.retain(|_, entry| entry.at.elapsed() < FILES_CACHE_TTL);
            lists.get(&cwd).map(|hit| Arc::clone(&hit.files))
        };
        let files = match cached {
            Some(files) => files,
            None => {
                let root = crate::repo::toplevel(&cwd)
                    .ok_or_else(|| "sessão fora de repositório git".to_string())?;
                let files = Arc::new(load_worktree_files(&root, &cwd)?);
                self.lists.lock().insert(
                    cwd,
                    CachedFiles {
                        files: Arc::clone(&files),
                        at: Instant::now(),
                    },
                );
                files
            }
        };
        Ok(rank_files(&files, query, limit))
    }
}

fn rank_files(files: &[String], query: &str, limit: usize) -> Vec<String> {
    if query.is_empty() {
        return files.iter().take(limit).cloned().collect();
    }
    let matcher = SkimMatcherV2::default().smart_case();
    let mut scored: Vec<(i64, &String)> = files
        .iter()
        .filter_map(|f| matcher.fuzzy_match(f, query).map(|score| (score, f)))
        .collect();
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.truncate(limit);
    scored.into_iter().map(|(_, f)| f.clone()).collect()
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize;

    #[test]
    fn clean_text_passes_through_without_marking_strip() {
        assert_eq!(normalize("hello world"), ("hello world".into(), false));
    }

    #[test]
    fn strips_paste_end_embedded_in_the_payload() {
        let (out, stripped) = normalize("evil\x1b[201~\rrm -rf /");
        assert!(!out.contains('\x1b'));
        assert_eq!(out, "evil\nrm -rf /");
        assert!(stripped);
    }

    #[test]
    fn strips_every_remaining_esc() {
        let (out, stripped) = normalize("a\x1b[31mred");
        assert_eq!(out, "a[31mred");
        assert!(stripped);
    }

    #[test]
    fn paste_end_reassembled_by_stripping_does_not_survive_both_passes() {
        let (out, _) = normalize("\x1b\x1b[201~[201~x");
        assert!(!out.contains("\x1b[201~"));
        assert!(!out.contains('\x1b'));
    }

    #[test]
    fn eight_bit_csi_cannot_reassemble_the_paste_end() {
        let (out, stripped) = normalize("x\u{9b}201~rm -rf /");
        assert_eq!(out, "x201~rm -rf /");
        assert!(!out.contains('\u{9b}'));
        assert!(stripped);
    }

    #[test]
    fn c0_controls_and_del_never_reach_the_pty() {
        let (out, stripped) = normalize("ls\u{4}\u{3}\u{7f}x\u{7}");
        assert_eq!(out, "lsx");
        assert!(stripped);
    }

    #[test]
    fn newline_and_tab_survive_as_content() {
        let (out, stripped) = normalize("a\n\tb");
        assert_eq!(out, "a\n\tb");
        assert!(!stripped);
    }

    #[test]
    fn normalizes_crlf_and_cr_to_lf_without_marking_strip() {
        assert_eq!(normalize("a\r\nb\rc"), ("a\nb\nc".into(), false));
    }

    #[test]
    fn cr_never_survives_inside_the_payload() {
        let (out, _) = normalize("linha1\rlinha2\r\n\r");
        assert!(!out.contains('\r'));
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::wrap;

    #[test]
    fn empty_payload_emits_no_wrapper() {
        assert!(wrap("").is_empty());
    }

    #[test]
    fn payload_is_wrapped_in_bracketed_paste() {
        assert_eq!(wrap("hi\nthere"), b"\x1b[200~hi\nthere\x1b[201~".to_vec());
    }
}

#[cfg(test)]
mod plan_injection_tests {
    use super::plan_injection;

    #[test]
    fn with_bracketed_paste_the_payload_is_wrapped() {
        assert_eq!(
            plan_injection("a\nb", true).unwrap(),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
    }

    #[test]
    fn without_bracketed_paste_single_line_goes_as_plain_text() {
        assert_eq!(plan_injection("ls -la", false).unwrap(), b"ls -la".to_vec());
    }

    #[test]
    fn without_bracketed_paste_multiline_is_refused() {
        let err = plan_injection("a\nb", false).unwrap_err();
        assert!(err.contains("multiline"));
    }

    #[test]
    fn empty_payload_yields_no_bytes_in_either_mode() {
        assert!(plan_injection("", true).unwrap().is_empty());
        assert!(plan_injection("", false).unwrap().is_empty());
    }
}

#[cfg(test)]
mod agent_matcher_tests {
    use super::{AgentMatcher, DEFAULT_AGENT_PATTERN};

    #[test]
    fn default_matches_known_agents_on_word_boundary() {
        let m = AgentMatcher::new();
        assert!(m.matches("claude"));
        assert!(m.matches("codex --resume abc"));
        assert!(m.matches("gemini chat"));
        assert!(!m.matches("codexx"));
        assert!(!m.matches("myclaude"));
        assert!(!m.matches("vim"));
    }

    #[test]
    fn invalid_pattern_is_rejected_and_the_previous_one_stays() {
        let m = AgentMatcher::new();
        assert!(!m.set_pattern("(("));
        assert!(m.matches("codex"));
        assert_eq!(DEFAULT_AGENT_PATTERN, r"^(claude|codex|gemini)\b");
    }

    #[test]
    fn valid_pattern_replaces_the_default() {
        let m = AgentMatcher::new();
        assert!(m.set_pattern(r"^(aider)\b"));
        assert!(m.matches("aider --model x"));
        assert!(!m.matches("codex"));
    }

    #[test]
    fn empty_pattern_resets_to_the_default_instead_of_matching_everything() {
        let m = AgentMatcher::new();
        assert!(m.set_pattern(r"^(aider)\b"));
        assert!(m.set_pattern("  "));
        assert!(m.matches("codex"));
        assert!(!m.matches("vim"));
    }
}

#[cfg(test)]
mod mentions_sensitive_tests {
    use super::mentions_sensitive;

    #[test]
    fn detects_sensitive_terms_case_insensitively() {
        assert!(mentions_sensitive("my PassWord is hunter2"));
        assert!(mentions_sensitive("cole o API key aqui"));
        assert!(mentions_sensitive("leia o .env e me diga"));
        assert!(mentions_sensitive("minha senha do banco"));
    }

    #[test]
    fn ordinary_text_does_not_trigger() {
        assert!(!mentions_sensitive("refatora o parser de diff"));
    }
}

#[cfg(test)]
mod rel_path_tests {
    use super::rel_path;
    use std::path::Path;

    #[test]
    fn path_inside_the_base_becomes_relative() {
        assert_eq!(
            rel_path(Path::new("/repo/src/lib.rs"), Path::new("/repo")),
            "src/lib.rs"
        );
    }

    #[test]
    fn path_outside_the_base_stays_absolute() {
        assert_eq!(
            rel_path(Path::new("/other/x.rs"), Path::new("/repo")),
            "/other/x.rs"
        );
    }
}

#[cfg(test)]
mod worktree_files_tests {
    use super::FilesCache;
    use std::process::Command;

    fn worktree_files(
        base: &std::path::Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, String> {
        FilesCache::default().files_for(base, query, limit)
    }

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
    fn lists_tracked_and_untracked_but_never_ignored() {
        let repo = temp_repo();

        let files = worktree_files(&repo, "", 50).unwrap();

        assert!(files.contains(&"main.rs".to_string()));
        assert!(files.contains(&"src/lib.rs".to_string()));
        assert!(files.contains(&"untracked.txt".to_string()));
        assert!(!files.iter().any(|f| f.ends_with("ignored.log")));
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn paths_are_relative_to_the_session_cwd_not_the_repo_root() {
        let repo = temp_repo();
        let base = repo.join("src");

        let files = worktree_files(&base, "lib", 10).unwrap();

        assert_eq!(files.first().map(String::as_str), Some("lib.rs"));
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn fuzzy_query_ranks_the_match_first_and_respects_the_limit() {
        let repo = temp_repo();

        let files = worktree_files(&repo, "mainrs", 1).unwrap();

        assert_eq!(files, vec!["main.rs".to_string()]);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn outside_a_git_repo_returns_an_error() {
        let dir = std::env::temp_dir().join(format!("tyba-notrepo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let result = worktree_files(&dir, "", 10);

        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn within_the_ttl_the_list_is_served_from_cache_without_git() {
        let repo = temp_repo();
        let cache = FilesCache::default();

        let before = cache.files_for(&repo, "", 50).unwrap();
        assert!(!before.contains(&"nascido-depois.txt".to_string()));

        std::fs::write(repo.join("nascido-depois.txt"), "x").unwrap();
        let cached = cache.files_for(&repo, "", 50).unwrap();
        assert_eq!(
            before, cached,
            "dentro do TTL a lista devia vir do cache, sem novo ls-files"
        );
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn an_expired_entry_reloads_and_sees_new_files() {
        let repo = temp_repo();
        let cache = FilesCache::default();

        cache.files_for(&repo, "", 50).unwrap();
        std::fs::write(repo.join("nascido-depois.txt"), "x").unwrap();

        let Some(expired) = std::time::Instant::now().checked_sub(super::FILES_CACHE_TTL * 2)
        else {
            return;
        };
        for entry in cache.lists.lock().values_mut() {
            entry.at = expired;
        }

        let reloaded = cache.files_for(&repo, "", 50).unwrap();
        assert!(
            reloaded.contains(&"nascido-depois.txt".to_string()),
            "entrada expirada devia recarregar do git"
        );
        std::fs::remove_dir_all(&repo).ok();
    }
}
