# Contributing to TYBA

Thanks for wanting to help. This document is short on ceremony and long on the two or three things that will actually get your PR merged.

## Setup

Prerequisites: [Bun](https://bun.sh), [Rust](https://rustup.rs), the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS, and `bubblewrap` on Linux.

```bash
bun install
bun tauri dev
```

## Gates

CI runs exactly these. Run them locally first — it is faster than a round trip.

```bash
cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
cargo test

cd ..
bun run typecheck
bun test
```

If you touched the Linux sandbox, also run its execution tests in a container:

```bash
./scripts/sandbox-linux-docker.sh
```

Conventional commits (`feat:`, `fix:`, `refactor:`, `docs:`). Code and identifiers in English; UI strings and docs in pt-BR (i18n is on the roadmap).

## The rules that are not up for negotiation

These are load-bearing. A PR that weakens one of them will be turned down regardless of how good the rest is — see [docs/SECURITY.md](docs/SECURITY.md) for the reasoning.

1. **All state lives in the Rust core.** The webview renders what it receives and emits intentions. No session, git, or status logic in React.
2. **Agent processes are spawned only through the `Sandbox` trait.** Never a bare `Command::new` for an agent.
3. **Red actions are never automatic**: `git push`, `gh pr create`, `sudo`, network, writes outside the worktree, `rm -rf`. Hard-coded in the runner — not configurable as "always allow".
4. **Pushing to main/master from an agent session is refused by the core.** Always.
5. **The sandbox fails closed.** If it cannot be applied, the session is refused. Security that switches itself off when it hits trouble is not security.
6. **Secrets never reach logs or persisted scrollback.** Redaction happens before the write to SQLite.

## Tests

Parser tests (diff, OSC 133, stream-json) are mandatory — they are the most fragile part of the codebase.

For anything touching the sandbox or the approval gate, a test that only proves the *negative* is not enough. A broken sandbox makes every deny test pass vacuously: the command fails, the protected file is untouched, and the assertion goes green without any policy having been applied. Always include the **positive pair** (the worktree *is* writable, the gate *does* connect) and assert that the cage actually came up.

**Test fixtures are always synthetic.** Never copy a real session from `~/.claude/projects` or `~/.codex/sessions` to reproduce a bug. A terminal records everything its owner typed; the fixture you added "just for the test" carries their token with it. Write scrollback, transcripts, and terminal output by hand.

## Reporting a vulnerability

Do not open a public issue. See [SECURITY.md](SECURITY.md).
