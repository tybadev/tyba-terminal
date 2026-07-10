# TYBA

> Your environment. Everything connected.

Open-source, AI-agent-oriented terminal: orchestrates multiple agent sessions (Claude Code, Codex) in isolated git worktrees, with an approvals inbox, local diff review, and security as a product differentiator.

**Status**: pre-alpha — Phase 1 of the [roadmap](docs/ROADMAP.md) under construction.

![TYBA](docs/assets/tyba-home.png)

## Stack

Tauri 2 (Rust) · React + TypeScript · Tailwind v4 · xterm.js · **Bun**

## Development

Prerequisites: [Bun](https://bun.sh), [Rust](https://rustup.rs), and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
bun install
bun tauri dev
```

## Screenshots

Chip bar layout editor (Settings → Preferences):

![Settings — chip bar editor](docs/assets/tyba-settings-chips.png)

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — data model, IPC, session lifecycle
- [Security](docs/SECURITY.md) — threat model and non-negotiable rules
- [Roadmap](docs/ROADMAP.md) — phases and exit criteria
- [CLAUDE.md](CLAUDE.md) — context for development with Claude Code

## License

[Apache-2.0](LICENSE)
