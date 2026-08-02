use std::path::{Path, PathBuf};
use std::sync::Arc;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use parking_lot::Mutex;
use serde::Serialize;

use super::Context;

const WALK_ENTRY_CAP: usize = 20_000;
const GIT_INDEX_CAP: usize = 50_000;
const WALK_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "vendor",
];

struct IndexData {
    names: Vec<String>,
    truncated: bool,
}

#[derive(Serialize)]
pub struct SearchOutcome {
    pub paths: Vec<String>,
    pub truncated: bool,
}

/// Índice de nomes de arquivo sob a raiz do painel. Em contexto git o índice
/// nasce do `git ls-files` (gitignorados fora, `.git` fora, sem descer em
/// `node_modules`); fora de repo, um walk limitado com denylist. Ambos com teto
/// nomeado e flag de truncado. Invalidado pelo watcher — a próxima busca
/// reconstrói.
#[derive(Default)]
pub struct FuzzyIndex {
    data: Mutex<Option<Arc<IndexData>>>,
}

impl FuzzyIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invalidate(&self) {
        *self.data.lock() = None;
    }

    fn ensure(&self, root: &Path, context: &Context) -> Arc<IndexData> {
        let mut guard = self.data.lock();
        if let Some(data) = guard.as_ref() {
            return Arc::clone(data);
        }
        let built = Arc::new(build(root, context));
        *guard = Some(Arc::clone(&built));
        built
    }

    pub fn search(
        &self,
        root: &Path,
        context: &Context,
        query: &str,
        limit: usize,
    ) -> SearchOutcome {
        let data = self.ensure(root, context);
        SearchOutcome {
            paths: rank(&data.names, query, limit),
            truncated: data.truncated,
        }
    }
}

fn build(root: &Path, context: &Context) -> IndexData {
    let (mut names, mut truncated) = match context {
        Context::AgentWorktree { .. } | Context::Repo => {
            git_tracked(root).unwrap_or((Vec::new(), false))
        }
        Context::OutsideRepo => (Vec::new(), false),
    };
    if names.is_empty() && matches!(context, Context::OutsideRepo) {
        let (walked, walked_truncated) = walk(root);
        names = walked;
        truncated = walked_truncated;
    }
    names.sort_unstable();
    names.dedup();
    IndexData { names, truncated }
}

fn git_tracked(root: &Path) -> Option<(Vec<String>, bool)> {
    let out = crate::worktree::git_in(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut names = Vec::new();
    let mut truncated = false;
    for field in out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        if names.len() >= GIT_INDEX_CAP {
            truncated = true;
            break;
        }
        names.push(String::from_utf8_lossy(field).replace('\\', "/"));
    }
    Some((names, truncated))
}

fn walk(root: &Path) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= WALK_ENTRY_CAP {
            return (out, true);
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if name.starts_with('.') || WALK_SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push(entry.path());
            } else if let Some(rel) = super::rel_of(root, &entry.path()) {
                out.push(rel);
                if out.len() >= WALK_ENTRY_CAP {
                    return (out, true);
                }
            }
        }
    }
    (out, false)
}

pub(crate) fn rank(names: &[String], query: &str, limit: usize) -> Vec<String> {
    if query.is_empty() {
        return names.iter().take(limit).cloned().collect();
    }
    let matcher = SkimMatcherV2::default().smart_case();
    let mut scored: Vec<(i64, &String)> = names
        .iter()
        .filter_map(|name| matcher.fuzzy_match(name, query).map(|score| (score, name)))
        .collect();
    scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.truncate(limit);
    scored.into_iter().map(|(_, name)| name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn temp_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-fuzzy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // O teste afirma o que o índice ignora, e `core.excludesfile` global de
        // quem roda a suíte entra nessa conta: um padrão lá fora tiraria do
        // índice um arquivo que o teste espera encontrar, e a falha não teria
        // nada a ver com o código.
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(dir.join("main.rs"), "x").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "x").unwrap();
        std::fs::write(dir.join("untracked.txt"), "x").unwrap();
        std::fs::write(dir.join("secret.log"), "x").unwrap();
        std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
        git(&["add", "main.rs", "src/lib.rs", ".gitignore"]);
        std::fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn index_excludes_gitignored_and_finds_tracked_and_untracked() {
        let repo = temp_repo();
        let index = FuzzyIndex::new();
        let all = index.search(&repo, &Context::Repo, "", 100);
        assert!(all.paths.contains(&"main.rs".to_string()));
        assert!(all.paths.contains(&"src/lib.rs".to_string()));
        assert!(all.paths.contains(&"untracked.txt".to_string()));
        assert!(!all.truncated, "repo pequeno não trunca o índice");
        assert!(
            !all.paths.iter().any(|f| f.ends_with("secret.log")),
            "gitignorado não pode entrar no índice"
        );
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn fuzzy_query_ranks_the_match_first() {
        let repo = temp_repo();
        let index = FuzzyIndex::new();
        let hits = index.search(&repo, &Context::Repo, "librs", 5);
        assert_eq!(hits.paths.first().map(String::as_str), Some("src/lib.rs"));
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn watcher_invalidation_reveals_new_files() {
        let repo = temp_repo();
        let index = FuzzyIndex::new();
        let before = index.search(&repo, &Context::Repo, "", 100).paths;
        assert!(!before.iter().any(|f| f == "nascido-depois.txt"));

        std::fs::write(repo.join("nascido-depois.txt"), "x").unwrap();
        let stale = index.search(&repo, &Context::Repo, "", 100).paths;
        assert_eq!(
            before, stale,
            "sem invalidação a busca vem do índice cacheado"
        );

        index.invalidate();
        let fresh = index.search(&repo, &Context::Repo, "", 100).paths;
        assert!(
            fresh.iter().any(|f| f == "nascido-depois.txt"),
            "após invalidar, a reconstrução vê o arquivo novo"
        );
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn outside_repo_walk_skips_denylisted_dirs() {
        let dir = std::env::temp_dir().join(format!("tyba-walk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::write(dir.join("src/app.ts"), "x").unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        let index = FuzzyIndex::new();
        let hits = index.search(&root, &Context::OutsideRepo, "", 100).paths;
        assert!(hits.contains(&"src/app.ts".to_string()));
        assert!(
            !hits.iter().any(|f| f.contains("node_modules")),
            "node_modules fica fora do walk"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
