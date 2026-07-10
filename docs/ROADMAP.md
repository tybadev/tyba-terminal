# Roadmap

Cada fase é usável sozinha e entrega valor antes da próxima começar. Ordem pensada para dogfooding: a partir da fase 4, o próprio tyba orquestra as sessões de Claude Code que o constroem.

## Fase 0 — Fundação do repo ✅ (exceto itens anotados)

- [x] Scaffold Tauri 2 + React + TS + Tailwind v4 + shadcn/ui (bun)
- [x] Licença Apache-2.0, SECURITY.md — _CONTRIBUTING.md ainda falta_
- [x] CI: clippy `-D warnings` + fmt + testes Rust (macOS + Ubuntu) + testes frontend — _Windows e build empacotado ficam para a Fase 6_
- [x] Estrutura de módulos do src-tauri conforme CLAUDE.md (traits de Sandbox e AgentRunner criadas)

## Fase 1 — Terminal single-session ✅

- [x] PtyPool com portable-pty spawando o shell do usuário
- [x] Batching de output (8-16ms) → evento IPC → xterm.js
- [x] Teclado, resize, working directory (cwd lógico vs físico, kernel como fonte)
- [x] Bracketed paste + preview/sanitização de paste multilinha
- [x] Tema base + gerenciador de temas (import, claro/escuro/sistema)

**Critério de saída**: usar como terminal do dia a dia sem fricção (vim/htop/lazygit funcionando). ✅ — em dogfooding diário.

## Fase 2 — Multi-session ✅ (exceto reopen)

- [x] SessionManager + tabs/splits + workspaces
- [x] SQLite store (sessões + scrollback com redação de secrets)
- [x] Sessões sobrevivem a fechar a janela (attach/detach no core, refcount por janela)
- [ ] Reconexão no reopen do app — [#50](https://github.com/tybadev/tyba-terminal/issues/50), spec pendente; blocos persistidos (ADR 2026-07-10) já nascem como fonte de dado
- [x] Kill com killpg

## Fase 3 — Worktree lifecycle ← **próxima obrigatória** (pré-requisito da Fase 4)

- [ ] WorktreeManager: create (branch slug + sufixo), remove, GC de órfãos no startup — _hoje só existe o plumbing `git_in`_
- [ ] Hooks de setup por repo (`.tyba/setup.sh`)
- [ ] Diff local: numstat/name-status com -z, hunks lazy por arquivo, dirty state
- [ ] DiffView React (sidebar de arquivos, Shiki, unified/split, colapso de gerados)

## Fase 4 — Agentes + inbox (o produto) — _decisões de escopo fechadas em grill 2026-07-10: TUI interativo no PTY + hooks pro inbox (não headless); 3 levas: sessão de agente → inbox de aprovações → merge flow_

- [ ] Trait AgentRunner + runner ClaudeCode — _trait e esqueleto existem; `build_command` muda de headless pra TUI com hooks injetados_
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
- [ ] Shell integration própria (OSC 133) com blocos de comando — _spec e ADR aceitos (linhas lógicas + persistência final, cofre 2026-07-10)_
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

## Entregue fora do roadmap (refinamento do shell, 2026-07)

Trabalho que não estava listado mas aproximou o shell dos concorrentes e preparou a Fase 4: barra de chips com editor drag-and-drop, Rich Input (composer multiline pra agentes, `@arquivo` com cache), watcher de git no core (notify + reconcile), detecção de editores (`$EDITOR`/`$VISUAL`), Settings completo (temas, atalhos, preferências, i18n pt-BR/en), attach/detach multi-janela no core, liveness de processo por start time.
