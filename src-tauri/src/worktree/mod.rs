//! Worktrees: boundary de escrita de cada sessão de agente (Fase 3).
//!
//! Regras dos docs:
//! - shell-out para o binário `git` (não git2/gitoxide no MVP)
//! - sempre `-z`, `--no-color`, `-c core.quotePath=false`
//! - three-dot semantics: diff contra `base_ref` salvo na criação
//! - `git stash` NUNCA na automação (compartilhado entre worktrees)

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    /// ~/.tyba/worktrees/<repo>/<branch>
    pub path: PathBuf,
    /// agent/<slug>-<sufixo curto>
    pub branch: String,
    /// sha da base no momento da criação (base do three-dot diff)
    pub base_ref: String,
    pub dirty: bool,
    pub ahead: u32,
}

// TODO(fase 3):
// - create(repo_root, task_title) -> Worktree
//   `git worktree add <path> -b <branch> <base_sha>` + hooks .tyba/setup.sh
// - remove(worktree, delete_branch: bool)
// - gc_orphans() no startup via `git worktree list --porcelain`
// - diff module: SessionDiff { commits, files, uncommitted } com hunks lazy
