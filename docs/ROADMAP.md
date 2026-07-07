# Roadmap

Cada fase é usável sozinha e entrega valor antes da próxima começar. Ordem pensada para dogfooding: a partir da fase 4, o próprio tyba orquestra as sessões de Claude Code que o constroem.

## Fase 0 — Fundação do repo

- [ ] Scaffold Tauri 2 + React + TS + Tailwind v4 + shadcn/ui (bun)
- [ ] Licença Apache-2.0, SECURITY.md, CONTRIBUTING.md básico
- [ ] CI: lint (clippy + eslint), testes, build nos 3 OS (GitHub Actions)
- [ ] Estrutura de módulos do src-tauri conforme CLAUDE.md (traits vazias de Sandbox e AgentRunner já criadas)

## Fase 1 — Terminal single-session

- [ ] PtyPool com portable-pty spawando o shell do usuário
- [ ] Batching de output (8-16ms) → evento IPC → xterm.js
- [ ] Teclado, resize, working directory
- [ ] Bracketed paste + preview de paste multilinha
- [ ] Tema base (aesthetic vinext: shadcn/ui, Phosphor)

**Critério de saída**: usar como terminal do dia a dia sem fricção (vim/htop/lazygit funcionando).

## Fase 2 — Multi-session

- [ ] SessionManager + tabs/splits
- [ ] SQLite store (sessões + scrollback com redação de secrets)
- [ ] Sessões sobrevivem a fechar a janela; reconexão no reopen
- [ ] Kill com killpg

## Fase 3 — Worktree lifecycle

- [ ] WorktreeManager: create (branch slug + sufixo), remove, GC de órfãos no startup
- [ ] Hooks de setup por repo (`.tyba/setup.sh`)
- [ ] Diff local: numstat/name-status com -z, hunks lazy por arquivo, dirty state
- [ ] DiffView React (sidebar de arquivos, Shiki, unified/split, colapso de gerados)

## Fase 4 — Agentes + inbox (o produto)

- [ ] Trait AgentRunner + runner ClaudeCode (stream-json)
- [ ] StatusDetector: eventos estruturados + fallback OSC 133/heurística
- [ ] Inbox: sidebar de sessões com status, notificação nativa em AwaitingInput
- [ ] approveAction com classificação de risco (verde/amarelo/vermelho)
- [ ] Env filtrado por allowlist (.tyba/config)
- [ ] Merge flow: review no DiffView → merge local | squash | gh pr create
- [ ] Regras hard: recusa de push para main, push sempre com aprovação humana

**Critério de saída**: rodar 3+ sessões de Claude Code em paralelo construindo o próprio tyba.

## Fase 5 — Profundidade

- [ ] Runner Codex + Custom
- [ ] Sandbox real (Seatbelt macOS, Landlock Linux)
- [ ] LSP para contexto de agente (tsserver/rust-analyzer)
- [ ] Shell integration própria (OSC 133) com blocos de comando
- [ ] OSC 52 com confirmação, sanitização OSC 8

## Fase 6 — Distribuição

- [ ] Codesign + notarização (macOS), releases assinados
- [ ] Auto-update assinado
- [ ] Site/docs, onboarding

## Explicitamente fora de escopo (por ora)

- Renderer GPU próprio (xterm.js aguenta o escopo)
- Shell próprio (o produto é o terminal/orquestrador; shell é o do usuário)
- OAuth/GitHub App próprio (gh CLI resolve)
- Sync em nuvem / telemetria
- Neovim embed via msgpack-RPC (fase 7+, depois de LSP)
