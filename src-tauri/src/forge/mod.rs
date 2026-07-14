pub mod cli_hosts;
pub mod exec;
pub mod github;
pub mod gitlab;
pub mod remote;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use remote::Remote;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    GitHub,
    GitLab,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForgeStatus {
    pub kind: ForgeKind,
    pub cli: String,
    pub installed: bool,
    pub authenticated: bool,
    pub web_create_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub checks: Vec<CheckRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

/// Um run de workflow. Nasce separado do `CheckRun` de propósito: o CheckRun é
/// pobre (nome, status, conclusão) e é consumido pelo tom agregado do PR —
/// esticá-lo pra caber `id`, `url` e `head_branch` quebraria dois consumidores
/// pra servir um terceiro.
///
/// `head_branch` guarda branch OU tag. É ele que responde "cortei a tag, em que
/// pé está o release?" — a pergunta que o painel de PR não tem como responder,
/// porque um run de tag não tem pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowRun {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub head_branch: String,
    pub event: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowJob {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub url: Option<String>,
}

fn encode_branch(branch: &str) -> String {
    utf8_percent_encode(branch, NON_ALPHANUMERIC)
        .to_string()
        .replace("%2D", "-")
        .replace("%2E", ".")
        .replace("%5F", "_")
        .replace("%7E", "~")
}

fn remote_of(repo_root: &Path) -> Option<Remote> {
    let out = exec::run(
        "git",
        &["remote", "get-url", "origin"],
        repo_root,
        None,
        exec::LOCAL_TIMEOUT,
    )
    .ok()?;
    if !out.ok {
        return None;
    }
    remote::parse(String::from_utf8_lossy(&out.stdout).trim())
}

fn current_branch(repo_root: &Path) -> Result<String, AppError> {
    let out = exec::run(
        "git",
        &["rev-parse", "--abbrev-ref", "HEAD"],
        repo_root,
        None,
        exec::LOCAL_TIMEOUT,
    )
    .map_err(|detail| AppError::new("push.failed").with("detail", detail))?;
    if !out.ok {
        return Err(AppError::new("push.failed").with("detail", out.stderr_message()));
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Err(AppError::new("push.detached_head"));
    }
    Ok(branch)
}

fn resolve_kind(remote: &Remote) -> Option<ForgeKind> {
    remote
        .kind()
        .or_else(|| cli_hosts::kind_for_host(&remote.host))
}

pub fn detect(repo_root: &Path) -> Option<ForgeKind> {
    let remote = remote_of(repo_root)?;
    resolve_kind(&remote)
}

pub fn status(repo_root: &Path, branch: Option<&str>) -> Option<ForgeStatus> {
    let remote = remote_of(repo_root)?;
    let kind = resolve_kind(&remote)?;

    let (cli, installed, authenticated, web_create_url) = match kind {
        ForgeKind::GitHub => {
            let installed = github::installed();
            let authenticated = installed && github::authenticated(repo_root, &remote);
            (
                github::CLI,
                installed,
                authenticated,
                branch.map(|b| github::web_create_url(&remote, b)),
            )
        }
        ForgeKind::GitLab => {
            let installed = gitlab::installed();
            let authenticated = installed && gitlab::authenticated(repo_root, &remote);
            (
                gitlab::CLI,
                installed,
                authenticated,
                branch.map(|b| gitlab::web_create_url(&remote, b)),
            )
        }
    };

    Some(ForgeStatus {
        kind,
        cli: cli.into(),
        installed,
        authenticated,
        web_create_url,
    })
}

fn kind_of(worktree: &Path) -> Result<ForgeKind, AppError> {
    detect(worktree).ok_or_else(|| AppError::new("forge.no_forge"))
}

fn cli_installed(kind: ForgeKind) -> bool {
    match kind {
        ForgeKind::GitHub => github::installed(),
        ForgeKind::GitLab => gitlab::installed(),
    }
}

/// Consulta de leitura sem a CLI: **não há o que consultar** — status de PR, checks
/// e comentários são API da forja, e sem `gh`/`glab` não existe caminho até ela.
/// Isso não é erro do usuário: a maioria das pessoas não instala `gh`. Antes, o
/// ENOENT do binário subia até a UI como "falha ao executar gh: arquivo ou
/// diretório inexistente" — mensagem que não diz nada e parece defeito do Tyba.
/// Agora a informação simplesmente não aparece.
fn cli_missing(kind: ForgeKind) -> bool {
    !cli_installed(kind)
}

fn is_auth_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    [
        "not logged in",
        "authentication",
        "auth login",
        "unauthorized",
        "http 401",
        "401 unauthorized",
        "requires authentication",
    ]
    .iter()
    .any(|marker| detail.contains(marker))
}

pub fn create_pr(worktree: &Path, title: &str, body: &str) -> Result<PullRequest, AppError> {
    if title.trim().is_empty() {
        return Err(AppError::new("pr.title_empty"));
    }
    let branch = current_branch(worktree)?;
    if matches!(branch.as_str(), "main" | "master") {
        return Err(AppError::new("push.protected_branch").with("branch", branch));
    }
    let kind = kind_of(worktree)?;
    // Ação (diferente de consulta) precisa dizer o que fazer. O diálogo já oferece
    // "push + abrir no navegador", que funciona sem CLI nenhuma — mas se alguém
    // chegar aqui sem a CLI, o erro tem que ser acionável, não um ENOENT cru.
    if cli_missing(kind) {
        let cli = match kind {
            ForgeKind::GitHub => github::CLI,
            ForgeKind::GitLab => gitlab::CLI,
        };
        return Err(AppError::new("forge.cli_missing").with("cli", cli));
    }
    crate::worktree::ops::push(worktree)?;
    match kind {
        ForgeKind::GitHub => github::create_pr(worktree, title, body),
        ForgeKind::GitLab => gitlab::create_pr(worktree, title, body),
    }
}

pub fn pr_for_branch(worktree: &Path, branch: &str) -> Result<Option<PullRequest>, AppError> {
    if branch.trim().is_empty() {
        return Err(AppError::new("pr.branch_empty"));
    }
    let kind = kind_of(worktree)?;
    if cli_missing(kind) {
        return Ok(None);
    }
    match kind {
        ForgeKind::GitHub => github::pr_for_branch(worktree, branch),
        ForgeKind::GitLab => gitlab::pr_for_branch(worktree, branch),
    }
}

pub fn pr_comments(worktree: &Path, number: u64) -> Result<Vec<ReviewComment>, AppError> {
    let kind = kind_of(worktree)?;
    if cli_missing(kind) {
        return Ok(Vec::new());
    }
    match kind {
        ForgeKind::GitHub => github::pr_comments(worktree, number),
        ForgeKind::GitLab => gitlab::pr_comments(worktree, number),
    }
}

pub fn pr_list(repo_root: &Path) -> Result<Vec<PullRequest>, AppError> {
    let kind = kind_of(repo_root)?;
    if cli_missing(kind) {
        return Ok(Vec::new());
    }
    match kind {
        ForgeKind::GitHub => github::pr_list(repo_root),
        ForgeKind::GitLab => gitlab::pr_list(repo_root),
    }
}

/// `None` e lista vazia dizem coisas diferentes, e a UI depende disso: vazio é
/// "não há run"; `None` é "não sei olhar aqui". O GitLab devolve `None` porque
/// não há repositório GitLab para exercitar o parser — escrever parser que
/// ninguém roda é a mesma armadilha do teste de sandbox que passa com a jaula
/// quebrada.
/// Segurar o resultado por 30s tem um propósito único: o painel abre INSTANTÂNEO
/// em vez de mostrar "carregando". Sem cache, pré-carregar ao focar a sessão
/// custaria um processo `gh` por troca de aba — e o core divide máquina com os
/// agentes.
const CACHE_TTL: Duration = Duration::from_secs(30);

type RunCache = HashMap<PathBuf, (Instant, Option<Vec<WorkflowRun>>)>;

fn cache() -> &'static Mutex<RunCache> {
    static CACHE: std::sync::OnceLock<Mutex<RunCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_runs(repo_root: &Path) -> Option<Option<Vec<WorkflowRun>>> {
    let cache = cache().lock();
    let (at, runs) = cache.get(repo_root)?;
    if at.elapsed() > CACHE_TTL {
        return None;
    }
    Some(runs.clone())
}

/// O painel aberto precisa ver o run que acabou de mudar de estado — se ele
/// lesse o cache, o poll de 15s mostraria o mesmo retrato por 30s.
pub fn workflow_runs_fresh(
    repo_root: &Path,
    limit: u32,
) -> Result<Option<Vec<WorkflowRun>>, AppError> {
    let runs = workflow_runs(repo_root, limit)?;
    cache()
        .lock()
        .insert(repo_root.to_path_buf(), (Instant::now(), runs.clone()));
    Ok(runs)
}

/// Usado pelo pré-carregamento: se o cache está fresco, não gasta processo.
pub fn workflow_runs_cached(
    repo_root: &Path,
    limit: u32,
) -> Result<Option<Vec<WorkflowRun>>, AppError> {
    if let Some(runs) = cached_runs(repo_root) {
        return Ok(runs);
    }
    workflow_runs_fresh(repo_root, limit)
}

pub fn workflow_runs(repo_root: &Path, limit: u32) -> Result<Option<Vec<WorkflowRun>>, AppError> {
    let kind = kind_of(repo_root)?;
    if cli_missing(kind) {
        return Ok(Some(Vec::new()));
    }
    match kind {
        ForgeKind::GitHub => github::run_list(repo_root, limit).map(Some),
        ForgeKind::GitLab => Ok(None),
    }
}

pub fn workflow_jobs(repo_root: &Path, run_id: u64) -> Result<Vec<WorkflowJob>, AppError> {
    let kind = kind_of(repo_root)?;
    if cli_missing(kind) {
        return Ok(Vec::new());
    }
    match kind {
        ForgeKind::GitHub => github::run_jobs(repo_root, run_id),
        ForgeKind::GitLab => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(origin: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        for args in [vec!["init", "-q"], vec!["remote", "add", "origin", origin]] {
            let out = std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .output()
                .expect("git");
            assert!(out.status.success(), "{args:?}");
        }
        dir
    }

    #[test]
    fn detects_github_from_the_origin_remote() {
        let repo = init_repo("git@github.com:tybadev/tyba-terminal.git");
        assert_eq!(detect(repo.path()), Some(ForgeKind::GitHub));
    }

    #[test]
    fn detects_gitlab_from_the_origin_remote() {
        let repo = init_repo("https://gitlab.com/group/sub/app.git");
        assert_eq!(detect(repo.path()), Some(ForgeKind::GitLab));
    }

    #[test]
    fn detects_selfhosted_gitlab_via_subgroups() {
        let repo = init_repo("git@git.acme.com:group/sub/infra/app.git");
        assert_eq!(detect(repo.path()), Some(ForgeKind::GitLab));
    }

    #[test]
    fn detects_nothing_on_an_unknown_forge() {
        let repo = init_repo("https://bitbucket.org/team/app.git");
        assert_eq!(detect(repo.path()), None);
        assert!(status(repo.path(), Some("main")).is_none());
        assert!(create_pr(repo.path(), "t", "b").is_err());
        assert!(pr_for_branch(repo.path(), "b").is_err());
        assert!(pr_comments(repo.path(), 1).is_err());
    }

    #[test]
    fn status_always_offers_the_web_fallback_url() {
        let repo = init_repo("git@github.com:tybadev/tyba-terminal.git");
        let status = status(repo.path(), Some("feat/forge")).expect("status");
        assert_eq!(status.kind, ForgeKind::GitHub);
        assert_eq!(status.cli, "gh");
        assert_eq!(
            status.web_create_url.as_deref(),
            Some("https://github.com/tybadev/tyba-terminal/compare/feat%2Fforge?expand=1")
        );
    }

    #[test]
    fn status_without_a_branch_has_no_fallback_url() {
        let repo = init_repo("https://gitlab.com/group/app.git");
        let status = status(repo.path(), None).expect("status");
        assert_eq!(status.cli, "glab");
        assert_eq!(status.web_create_url, None);
    }

    #[test]
    fn rejects_an_empty_title_or_branch_before_touching_the_network() {
        let repo = init_repo("git@github.com:tybadev/tyba-terminal.git");
        assert!(create_pr(repo.path(), "  ", "body").is_err());
        assert!(pr_for_branch(repo.path(), " ").is_err());
    }

    #[test]
    fn keeps_url_safe_characters_readable_in_the_branch() {
        assert_eq!(encode_branch("feat/forge-1.2_x~y"), "feat%2Fforge-1.2_x~y");
    }
}
