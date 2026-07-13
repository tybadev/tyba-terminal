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

## Fase 3 — Worktree lifecycle ✅ **concluída** (PRs #71, #72)

- [x] WorktreeManager: create (branch slug + sufixo), remove, GC de órfãos no startup
- [x] Hooks de setup por repo (`.tyba/setup.sh`) com consent por hash
- [x] Diff local: numstat/name-status com -z, hunks lazy por arquivo, dirty state
- [x] DiffView React (sidebar de arquivos, Shiki, unified/split, colapso de gerados) — evoluiu para painel de git do workspace (staging, commit, push, comentários pro agente)

## Fase 4 — Agentes + inbox (o produto) ✅ **concluída** (PRs #73–#77) — _spec e decisões no cofre: `tyba/features/agents/`_

- [x] Trait AgentRunner + runner ClaudeCode — TUI interativo com hooks injetados via `--settings`; bypass de permissões proibido (teste trava)
- [x] StatusDetector: eventos de hook (SessionStart/Stop/Notification/SessionEnd) → SessionStatus; fallback OSC/heurística fica pra F5 (só runners sem hooks)
- [x] Inbox: sidebar com status, notificação nativa em AwaitingInput, toast acionável, opções numeradas 1/2/3 como no TUI
- [x] approveAction com classificação de risco — hook `PreToolUse` bloqueia até a decisão humana (unix socket + `tyba _hook`, deny fail-closed); verde auto-aprovado; "sempre permitir" por comando+sessão (nunca vermelho); recusar com feedback volta pro agente
- [x] Env filtrado por allowlist (`.tyba/config.toml`) com consent por hash e baseline inviolável
- [x] **Leva 3 — merge flow** (#76, #77): review no painel → Abrir PR via `gh`/`glab` (push automático da branch antes) | merge local (ação vermelha, `merge-tree`+ff-only) | PR view com checks e comentários encaminhados pro agente
- [x] Regras hard: recusa de push para main (core), push sempre com aprovação humana, spawn atrás da trait Sandbox

**Critério de saída**: rodar 3+ sessões de Claude Code em paralelo construindo o próprio tyba. ✅ — a leva 3 foi construída assim.

### Pós-Fase 4 (profundidade/polish já entregue)

- [x] Logo/marca Tyba + set de ícones (#78)
- [x] Códigos de erro bilíngues pt/en (`AppError {code,params}` + `translateError`) (#79)
- [x] Polish do painel de diff (accordion, rolagem contida, "Ver PR"), notificações via toast, error boundary por região (#79, #80)
- [x] **Painel de git pra TODO repo** (não só worktree) + ícones contextuais de git/PR no header (#81) — _spec: `tyba/features/git-panel/rules`_
- [x] **Status de sessão visível** (#83) — cores por estado no sidebar (azul in progress / âmbar aguardando-bloqueado / verde concluído / vermelho falhou), motivo estruturado da pausa (`AwaitingInput.reason`), atenção não-vista no core (`session_mark_seen`), toast no topo com resumo do turno (transcript) + atalho ⌘⇧O, notificação nativa com settle cancelável, spinner braille no título da janela, tab com path + glifo do agente — _spec: `tyba/features/session-status`_
- [x] Ícone do app em squircle macOS com margens transparentes (Dock sem quadrado full-bleed) (#83)
- [ ] Seletor de base/branch no painel de diff (three-dot/merge-base) — _grill pendente; comparar a branch contra o alvo de merge_

## Fase 5 — Profundidade

- [x] **Runner Codex** — o Codex CLI tem hooks no formato do Claude Code, então o gate de aprovação, o StatusDetector e o resumo do turno reaproveitam o pipeline inteiro: hooks injetados por `-c` com **trust computado** (o Tyba replica o fingerprint sha256 que o Codex exige, sem `--dangerously-bypass-hook-trust`), sandbox nativo `workspace-write` ligado como segunda camada, `-a on-request` mantendo o prompt do TUI como backstop do fail-open do Codex, classificação de risco por `ToolAction` canônico (Bash/apply_patch/web_search), resumo do turno lendo o rollout JSONL — _spec: `tyba/features/codex-runner`; ADR: autoridade única com sandbox nativo ligado_
- [ ] Runner Custom — **bloqueado por design, não por esforço**: um binário arbitrário não tem hooks, logo não tem `PreToolUse`, logo **não tem gate de aprovação** — o agente rodaria o que quisesse sem o inbox ver, e a trait `Sandbox` ainda é passthrough. Só faz sentido depois do sandbox real (Seatbelt/Landlock), quando a contenção vem do SO e o inbox aprova escaladas. O botão foi removido do dialog: UI morta é dívida. Aí sim entra o fallback OSC/heurística de status.
- [~] Sandbox real — **Seatbelt macOS concluído (PR #87)**: envolve o processo do agente Claude, leitura de conteúdo deny-por-default com allowlist, escrita granular (worktree + `.git` sem hooks/config, refs só `tyba/`), fail-closed, denylist final autoritativa imune ao `read_allow` do usuário; Codex fica no Seatbelt nativo dele (aninhar quebra `sandbox_apply`). **Falta Linux** (bubblewrap/seccomp, validado em Docker) — _spec/ADR: `tyba/features/sandbox`_
- [ ] Sandbox Linux (bubblewrap/seccomp) — spawn de agente recusado na plataforma até existir
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
