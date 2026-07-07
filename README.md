# TYBA

> Seu ambiente. Tudo conectado.

Terminal opensource orientado a agentes de IA: orquestra múltiplas sessões (Claude Code, Codex) em git worktrees isolados, com inbox de aprovações, review de diff local e segurança como diferencial.

**Status**: pré-alpha — Fase 1 do [roadmap](docs/ROADMAP.md) em construção.

## Stack

Tauri 2 (Rust) · React + TypeScript · Tailwind v4 · xterm.js · **Bun**

## Desenvolvimento

Pré-requisitos: [Bun](https://bun.sh), [Rust](https://rustup.rs) e os [pré-requisitos do Tauri 2](https://v2.tauri.app/start/prerequisites/) para o seu OS.

```bash
bun install
bun tauri dev
```

## Documentação

- [Arquitetura](docs/ARCHITECTURE.md) — modelo de dados, IPC, ciclo de vida de sessão
- [Segurança](docs/SECURITY.md) — modelo de ameaça e regras não negociáveis
- [Roadmap](docs/ROADMAP.md) — fases e critérios de saída
- [CLAUDE.md](CLAUDE.md) — contexto para desenvolvimento com Claude Code

## Licença

[Apache-2.0](LICENSE)
