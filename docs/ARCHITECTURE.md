# Arquitetura

## Visão

O tyba é um **orquestrador de sessões de agentes com terminal como interface**, não um emulador de terminal tradicional. O terminal core (xterm.js + PTY) é commodity; o produto é o gerenciamento de múltiplas sessões de agente em worktrees isolados, com inbox de status e review local de mudanças.

## Topologia de processos

```
┌─────────────────────────────────────────┐
│  Webview (React + xterm.js)             │
│  UI, inbox, renderização dos terminais  │
└──────────────┬──────────────────────────┘
               │ Tauri IPC (commands + events)
┌──────────────┴──────────────────────────┐
│  Rust core (processo Tauri)             │
│  SessionManager · WorktreeManager       │
│  PtyPool · StatusDetector · Store       │
└──┬────────┬────────┬────────────────────┘
   │ PTY    │ PTY    │ PTY
 zsh      claude   claude
(manual) (agente) (agente)
```

Estado vive no core. Fechar a janela não mata sessões — reabrir o app reconecta à inbox com os agentes ainda rodando.

## Modelo de dados

```rust
struct Session {
    id: Uuid,
    kind: SessionKind,          // Shell | Agent { runner: AgentRunner }
    title: String,
    repo_root: Option<PathBuf>,
    worktree: Option<Worktree>,
    pty_id: PtyId,
    status: SessionStatus,
    created_at: DateTime<Utc>,
}

enum AgentRunner { ClaudeCode, Codex, Custom(String) }

enum SessionStatus {
    Running,
    AwaitingInput { hint: Option<String> },
    Idle,
    Exited { code: i32 },
    Failed { reason: String },
}

struct Worktree {
    path: PathBuf,        // ~/.tyba/worktrees/<repo>/<branch>
    branch: String,       // agent/<slug>-<sufixo-curto>
    base_ref: String,     // sha da main no momento da criação
    dirty: bool,
    ahead: u32,
}
```

`SessionStatus` é o dado central do produto — alimenta a inbox. Persistir sessões (metadados + últimas N linhas de scrollback, com redação de secrets) em SQLite.

## IPC

**Commands (UI → core):**

```typescript
createSession(opts: { kind, repoRoot?, prompt?, baseBranch? }): SessionId
writeToSession(id, data: string)
resizeSession(id, cols, rows)
approveAction(id)
getSessionDiff(id): SessionDiff
getFileHunks(id, path): Hunk[]        // lazy-load por arquivo
mergeSession(id, strategy: 'merge' | 'squash' | 'pr')
disposeSession(id, { removeWorktree: boolean })
```

**Events (core → UI):**

```typescript
`pty://output/${id}`       // chunks batched para xterm.js
`session://status/${id}`   // mudanças de SessionStatus
`session://progress/${id}` // eventos estruturados do stream-json
```

**Batching obrigatório**: acumular output de PTY em buffer no Rust e flushar a cada ~8-16ms. `cargo build`/`bun install` geram output rápido demais para um emit por chunk (serialização JSON do IPC vira gargalo). Evolução futura: `tauri::ipc::Channel` binário.

## StatusDetector

Dois modos, por confiabilidade:

1. **Estruturado (preferido)**: Claude Code com `--output-format stream-json --include-partial-messages`. Cada linha do stdout é um evento JSON — tool use pendente de aprovação mapeia direto para `AwaitingInput`. Zero scraping de ANSI.
2. **Heurístico (fallback)**: shell integration OSC 133 (`A` = prompt, `C` = executando, `D` = terminou) + timeout de silêncio com último frame terminando em padrão de pergunta (`? `, `[y/n]`, `❯`).

## Ciclo de vida da sessão de agente

```
create → worktree → spawn → monitor → review → merge → cleanup
```

1. **Create**: resolver `repo_root` (`git rev-parse`), gerar branch a partir da task (slug + sufixo aleatório curto — evita colisão de branch entre worktrees).
2. **Worktree**: `git worktree add <path> -b <branch> <base_ref>` + hooks de setup por repo (`.tyba/setup.sh`): symlink de `.env`, `bun install`, database por worktree.
3. **Spawn**: PTY no worktree com o runner do agente. PID e PGID guardados.
4. **Monitor**: StatusDetector → inbox. `AwaitingInput` → notificação nativa. `approveAction` escreve a resposta no PTY sem trocar de foco.
5. **Review**: diff local (ver abaixo).
6. **Merge**: merge local | squash | `gh pr create` (via gh CLI do usuário — sem OAuth próprio).
7. **Cleanup**: `git worktree remove` + delete de branch mergeado. GC no startup lista órfãos via `git worktree list --porcelain`.

Armadilhas conhecidas: `git stash` é compartilhado entre worktrees (nunca usar em automação — sempre commit em branch); dois worktrees não podem ter o mesmo branch checked out.

## Diff local ("PR review" sem GitHub)

Toda a informação vem do `.git` local. Semântica three-dot: comparar com o ponto de partida, não com a main atual.

```bash
# sumário (sidebar)
git diff --numstat -z <base_sha>..HEAD
git diff --name-status -z -M <base_sha>..HEAD

# hunks de um arquivo (lazy, ao clicar)
git diff <base_sha>..HEAD -- <path>

# timeline de commits da sessão
git log <base_sha>..HEAD --format='%H%x00%s%x00%aI'

# estado não commitado (agente no meio do trabalho)
git status --porcelain=v2
git diff && git diff --cached
```

Sempre `-z`, `--no-color`, `-c core.quotePath=false`.

```rust
struct SessionDiff { commits: Vec<CommitInfo>, files: Vec<FileDiff>, uncommitted: bool }
struct FileDiff {
    path: PathBuf, old_path: Option<PathBuf>,
    status: FileStatus, additions: u32, deletions: u32,
    hunks: Vec<Hunk>,   // vazio até lazy-load
}
struct Hunk { old_start: u32, old_lines: u32, new_start: u32, new_lines: u32, lines: Vec<DiffLine> }
```

Lazy-load de hunks é obrigatório (lockfiles geram diffs de dezenas de milhares de linhas; colapsar arquivos gerados por default).

**UI**: DiffView é componente React comum (fora do xterm.js). Sidebar de arquivos + viewer com Shiki para highlight, toggle unified/split. O botão de aprovar push mora dentro da tela de diff — review local é passo obrigatório antes de qualquer push.

## Futuro (não implementar agora, não bloquear)

- **LSP**: agente consultando tsserver/rust-analyzer para contexto (definições, type errors) — diferencial técnico principal da fase 2.
- **Neovim embed**: `nvim --embed` via msgpack-RPC para abrir diffs em buffer nativo.
- **Sandbox real**: Seatbelt (macOS) / Landlock (Linux) na trait `Sandbox`.
- Integração GitHub via OAuth/GitHub App (por ora, gh CLI resolve tudo).
