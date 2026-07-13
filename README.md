# TYBA

> Your environment. Everything connected.

Open-source, AI-agent-oriented terminal: orchestrates multiple agent sessions (Claude Code, Codex) in isolated git worktrees, with an approvals inbox, local diff review, and security as a product differentiator.

Every agent runs inside a real OS sandbox — Seatbelt on macOS, bubblewrap + seccomp on Linux — with a deny-by-default read policy. Your `~/.ssh`, cloud credentials and other projects are not merely off-limits to the agent: they do not exist in its world. Every tool use goes through an approval gate; pushing to `main` from an agent session is refused by the core, always.

![TYBA](docs/assets/tyba-home.png)

## Install

**macOS 11+** — download the `.dmg` for your chip (Apple Silicon or Intel) from [Releases](https://github.com/tybadev/tyba-terminal/releases). Signed and notarized.

**Linux** — download the `.deb`, `.rpm` or `.AppImage` from [Releases](https://github.com/tybadev/tyba-terminal/releases):

```bash
sudo apt install ./Tyba_0.1.0_amd64.deb    # Debian / Ubuntu
sudo dnf install ./Tyba-0.1.0-1.x86_64.rpm # Fedora
yay -S tyba-bin                            # Arch (AUR)
```

`bubblewrap` is a hard dependency, not a suggestion: without it the core refuses to spawn agents (fail-closed) and TYBA is just a terminal. The `.deb`/`.rpm`/AUR packages pull it in for you; with the AppImage, install it yourself (`apt install bubblewrap`). If your distro ships unprivileged user namespaces disabled (some hardened kernels, Debian with `kernel.unprivileged_userns_clone=0`), agent sessions are refused with an actionable message — the sandbox never silently degrades.

Agents themselves are not bundled: install [Claude Code](https://claude.com/claude-code) or [Codex](https://developers.openai.com/codex/cli) separately and TYBA picks them up from your `PATH`.

## Stack

Tauri 2 (Rust) · React + TypeScript · Tailwind v4 · xterm.js · **Bun**

## Development

Prerequisites: [Bun](https://bun.sh), [Rust](https://rustup.rs), and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. On Linux, also `bubblewrap`.

```bash
bun install
bun tauri dev
```

Gates before opening a PR (CI runs the same):

```bash
cd src-tauri && cargo fmt --check && cargo clippy -- -D warnings && cargo test
bun run typecheck && bun test
./scripts/sandbox-linux-docker.sh   # exec tests for the Linux sandbox, in a container
```

## Screenshots

Chip bar layout editor (Settings → Preferences):

![Settings — chip bar editor](docs/assets/tyba-settings-chips.png)

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — data model, IPC, session lifecycle
- [Security](docs/SECURITY.md) — threat model and non-negotiable rules
- [Roadmap](docs/ROADMAP.md) — phases and exit criteria
- [TODO](docs/TODO.md) — what was deliberately left out of the launch, and why
- [CLAUDE.md](CLAUDE.md) — context for development with Claude Code

## License

[Apache-2.0](LICENSE)
