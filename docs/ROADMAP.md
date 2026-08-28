# Roadmap

Cada fase é usável sozinha e entrega valor antes da próxima começar. Ordem pensada para dogfooding: a partir da fase 4, o próprio tyba orquestra as sessões de Claude Code que o constroem.

## Fase 0 — Fundação do repo ✅ (exceto itens anotados)

- [x] Scaffold Tauri 2 + React + TS + Tailwind v4 + shadcn/ui (bun)
- [x] Licença Apache-2.0, SECURITY.md, CONTRIBUTING.md
- [x] CI: clippy `-D warnings` + fmt + testes Rust (macOS + Ubuntu) + testes frontend — Windows entrou nos gates (#162), mas **compila os testes sem executá-los** até o [#163](https://github.com/tybadev/tyba-terminal/issues/163)
- [x] Estrutura de módulos do src-tauri conforme CLAUDE.md (traits de Sandbox e AgentRunner criadas)

## Fase 1 — Terminal single-session ✅

- [x] PtyPool com portable-pty spawando o shell do usuário
- [x] Batching de output (8-16ms) → evento IPC → xterm.js
- [x] Teclado, resize, working directory (cwd lógico vs físico, kernel como fonte)
- [x] Bracketed paste + preview/sanitização de paste multilinha
- [x] Tema base + gerenciador de temas (import, claro/escuro/sistema)

**Critério de saída**: usar como terminal do dia a dia sem fricção (vim/htop/lazygit funcionando). ✅ — em dogfooding diário.

## Fase 2 — Multi-session ✅ **concluída**

- [x] SessionManager + tabs/splits + workspaces
- [x] SQLite store (sessões + scrollback com redação de secrets)
- [x] Sessões sobrevivem a fechar a janela (attach/detach no core, refcount por janela)
- [x] Reconexão no reopen do app — [#50](https://github.com/tybadev/tyba-terminal/issues/50), entregue no #119 (`resume_startup`, preferência em Settings)
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
- [ ] Runner Custom — **bloqueado por design, não por esforço**: um binário arbitrário não tem hooks, logo não tem `PreToolUse`, logo **não tem gate de aprovação** — o agente rodaria o que quisesse sem o inbox ver. Este item listava duas condições, e **a do sandbox já foi cumprida**: Seatbelt (#87), bwrap/seccomp (#116) e a Camada A do Windows estão no produto, e `platform_sandbox()` recusa fail-closed onde não há jaula — não há implementação passthrough no código. Sobra a segunda, que é a que segura: o inbox precisa aprovar **escalada** — pedido que nasce da negação do SO, não de um hook —, porque é só isso que devolve o gate a um processo que não tem como avisar antes de agir. O botão foi removido do dialog: UI morta é dívida. Aí sim entra o fallback OSC/heurística de status.
- [~] Sandbox real — **Seatbelt macOS concluído (PR #87)**: envolve o processo do agente Claude, leitura de conteúdo deny-por-default com allowlist, escrita granular (worktree + `.git` sem hooks/config, refs só `tyba/`), fail-closed, denylist final autoritativa imune ao `read_allow` do usuário; Codex fica no Seatbelt nativo dele (aninhar quebra `sandbox_apply`). **Falta Linux** (bubblewrap/seccomp, validado em Docker) — _spec/ADR: `tyba/features/sandbox`_
- [x] Sandbox Linux (bubblewrap/seccomp) — PR #116; mesma `SandboxSpec` do Seatbelt traduzida para bind mounts; validado em Docker
- [~] Sandbox Windows — **shippou como Camada A parcial** (`feat(windows)`, v0.1.2): `SandboxSpec` traduzida para token `WRITE_RESTRICTED` + SID sintético por sessão + Integrity Level Low; spawn enjaulado com ConPTY sob o token, Job Object `KILL_ON_JOB_CLOSE` (paridade killpg #9), env por allowlist, deny de leitura dos segredos **nomeados** por rótulo IL, hook/gate por named pipe herdado (o gate atravessa a jaula), fail-closed — _spec: `tyba/features/sandbox/windows-tech-spec`; ADR: `tyba/decisions/2026-07-14-windows-token-restrito-nao-appcontainer` (token restrito, **não** AppContainer)_. **A spec v1 era Camada A + B juntas; o release saiu com A.** Cortes registrados no [TODO](TODO.md): (1) **smoke no app real** nunca exercido ponta a ponta (só headless + spike isolada de isTTY); (2) **shim de git** só validado em spike, git roda dentro da jaula em vez de roteado pelo core; (3) **Camada B** (usuário dedicado + rede por WFP + read default-deny) inteira medida em spike, zero no produto. **Consequência**: o Windows publicado é mais fraco que mac/Linux — **sem jaula de rede** (alcança loopback/RFC1918/internet) e **sem read default-deny** (só segredos nomeados)
- [ ] LSP para contexto de agente (tsserver/rust-analyzer)
- [x] **Shell integration própria (OSC 133) com blocos de comando** (#230–#236, #238) — _spec: `tyba/features/terminal-blocks/rules`, `tyba/features/command-line/rules`; ADR: `2026-07-10-blocos-linhas-logicas-nao-bytes-crus`_. O comando vira **linha lógica**, não byte cru; o bloco guarda comando, saída, código de saída, cwd e duração, com redação antes de gravar. Em volta dele: barra de menu nativa no macOS (#230), histórico e snippets na paleta (#231), a linha de comando como editor do TYBA (#232), os cartões com ações e seleção (#233). A **emenda** entre a saída ao vivo e o cartão (#235, #236) é onde estava a dificuldade real, e o que ela ensinou vale para quem mexer ali:
  - O recorte da faixa ao vivo usa `top` + `clip-path`, **nunca `transform`** — os três preservam o tamanho (que é o que o `ResizeObserver` observa e o que viraria `resizeSession`), mas `transform` promove o elemento a camada composta e o canvas WebGL passa a ser rasterizado ignorando o pixel ratio.
  - A entrelinha do cartão é **medida** de `.xterm-screen`, nunca calculada: o CSS multiplica pelo `font-size` e o xterm pela altura do glifo, então o mesmo `1.35` dá números diferentes.
  - A seta só é engolida com `ECHO` ligado no termios do PTY, e **só a seta**: o `y` de um `Ok to proceed?` também é canônico com eco, e segurar o teclado inteiro impediria responder ao prompt.
  - `DECCKM` não distingue quem quer as setas — medido: o menu do `npm create vite` navega com ele desligado.
  - **Não coberto**: `htop`, resize de janela e reattach de aba sob o recorte; p10k contra o repaint assíncrono. Ver [TODO](TODO.md).
- [ ] OSC 52 com confirmação, sanitização OSC 8

### Paridade de histórico de shell (2026-08-15)

O motor já existe (`history/mod.rs`): captura por `OSC 633;E`/`133;D`, frecência com escopo de cwd/repo e demérito para comando que só falhou, `ignorespace` respeitado, redação antes do SQLite, writer fora do caminho quente. O que falta não é motor — é **dado de entrada** e **cobertura de shell**. Os três itens saíram de comparar o TYBA com o [atuin](https://github.com/atuinsh/atuin), e a ordem é por impacto sobre custo, não por vontade.

- [x] **Import do histórico existente** — entregue na **0.6.0** (PRs #260, #266), lendo zsh, bash e fish.
  **Falta o `history.db` do atuin**, que este item pedia e não entrou: quem migra do atuin ainda chega com a paleta vazia.
  _Contexto original:_ a paleta nascia vazia: a frecência leva semanas para ter o que ranquear enquanto o usuário já tem anos em `~/.zsh_history`. Importar zsh (com e sem `EXTENDED_HISTORY`), bash, fish e o `history.db` do atuin quando existir. Armadilhas conhecidas: o formato estendido do zsh (`: <epoch>:<dur>;<cmd>`) e a continuação por `\` são o que quebra parser ingênuo; a **redação de secrets precisa rodar no import**, não só na gravação ao vivo, senão o arquivo do usuário entra cru no SQLite; e o import tem de ser idempotente, ou duplica a cada execução. Parser com teste unitário é obrigatório (CLAUDE.md).
- [ ] **Hook de shell para fish e PowerShell** — a integração OSC só existe para zsh e bash (`session/tyba-zsh-rc.sh`, `session/tyba-bash-rc.sh`). No Windows o core **lança** `pwsh`/`powershell` (`session/mod.rs:851`) e não injeta hook nenhum: sem OSC 133/633 não há histórico, bloco de comando, cwd lógico nem status de sessão — o Windows publicado é um terminal, não o TYBA. Fish é o mesmo buraco no macOS/Linux, com público maior. **A 0.6.0 é o primeiro release que publica
`.exe` e `.msi`**, então a partir dela esse buraco tem usuário do outro lado. PowerShell não tem `precmd`/`preexec`: a emissão sai de `prompt` + `PSConsoleHostReadLine`, e a injeção precisa sobreviver a perfil de usuário que redefine `prompt`. Fish usa os eventos `fish_preexec`/`fish_postexec`.
- [ ] **Stats de uso** — vitrine, não capacidade. Só depois dos dois acima, e só se houver folga.
  **Não confundir com o painel de estatísticas que saiu na 0.6.0** (PR #269): aquele mede agentes e
  aprovações — quanto o agente custa de atenção humana —, e o próprio PR diz que não é "qual comando
  eu mais uso". Este item continua aberto.

## Fase 6 — Distribuição

- [x] Pipeline de release: matriz macOS (Apple Silicon + Intel), Linux e Windows, gerando `.dmg`, `.deb`, `.rpm`, AppImage, `.exe`/`.msi` + SHA256SUMS; release sai como **rascunho** para conferência humana (macOS builda mas não publica até o certificado — ver abaixo)
- [x] Gates definidos num lugar só (`gates.yml`) e chamados pelo PR, pela main e **pela tag** — a tag não sai da main por definição, então a release roda os gates antes de publicar em vez de confiar que alguém já rodou
- [x] Tag amarrada à versão dos manifestos: `v0.2.0` com `tauri.conf.json` em `0.1.0` publicaria um `Tyba_0.1.0.dmg` dentro do release errado — o job recusa
- [x] Gate de secrets no histórico completo (gitleaks na CI) — pré-requisito de abrir o repo
- [x] CONTRIBUTING.md, README com instalação por plataforma
- [ ] Codesign + notarização (macOS) — **falta o certificado Developer ID nos secrets**; o workflow já verifica com `codesign` + `spctl` e recusa publicar artefato não assinado (build ad-hoc é rejeitado pelo Gatekeeper e o usuário vê "app danificado")
- [ ] Codesign Windows (SmartScreen) — o release já builda e publica Windows **unsigned** (NSIS `.exe` + MSI); sem certificado, o SmartScreen avisa "app não reconhecido". Mesma classe do bloqueador da Apple: aquisição de certificado, não código
- [ ] QA de desktop em Linux real (webkitgtk + xterm.js, notificações, window-state, PTY)
- [x] Notificação de versão nova + what's new — entregue na v0.1.1 (#140): `update/` no core compara com a última release, toast acionável + botão em Settings apontando para o changelog do site. What's new **dentro** da app (sem sair pro site) fica como polish futuro.
- [ ] Auto-update assinado — **fora da v0.1 de propósito**: a chave privada de update é o secret mais perigoso do projeto (quem a tiver publica um "update" que a máquina do usuário instala sozinha). Entra na v0.2, com calma. Ver [TODO](TODO.md).
- [ ] Flatpak — **fora da v0.1**: o TYBA é um sandbox, e bwrap aninhado dentro do bwrap do Flatpak não sobe. O caminho é `flatpak-spawn --host` (precedente Ptyxis/Black Box), com PTY, socket de hook e a jaula atravessando a fronteira do container: projeto próprio, não um manifesto.
- [ ] Site/docs, onboarding

## Explicitamente fora de escopo (por ora)

- Renderer GPU próprio (xterm.js aguenta o escopo)
- Shell próprio (o produto é o terminal/orquestrador; shell é o do usuário)
- OAuth/GitHub App próprio (gh CLI resolve)
- Sync em nuvem / telemetria — inclui **sync de histórico entre máquinas**, que é o que o atuin faz melhor do que nós: exige servidor, e é o "app desktop sem backend" que hoje permite dizer que o projeto não tem segredo para vazar. Decisão, não dívida.
- Runbook executável, cliente de banco embutido, gráfico de observabilidade — o território do [Atuin Desktop](https://github.com/atuinsh/desktop). Nosso bloco é **registro do que rodou**; o deles é documento para rodar depois. São produtos diferentes, e perseguir esse custa um backend e dilui o nosso.
- Neovim embed via msgpack-RPC (fase 7+, depois de LSP)

## Entregue fora do roadmap (refinamento do shell, 2026-07)

Trabalho que não estava listado mas aproximou o shell dos concorrentes e preparou a Fase 4: barra de chips com editor drag-and-drop, Rich Input (composer multiline pra agentes, `@arquivo` com cache), watcher de git no core (notify + reconcile), detecção de editores (`$EDITOR`/`$VISUAL`), Settings completo (temas, atalhos, preferências, i18n pt-BR/en), attach/detach multi-janela no core, liveness de processo por start time.

## Entregue fora do roadmap (SSH, Docker e forge, 2026-07)

Dois eixos inteiros que o plano original não previa, mais um item de forge — specs e ADRs no cofre (`tyba/features/ssh`, decisões `2026-07-15/16-ssh-*`):

- **SSH como feature humana** (o agente remoto é projeto separado, por decisão): gestor de conexões com grupos/cores (#156), credenciais por delegação zero-segredo (ssh-agent/1Password, o TYBA nunca vê chave), hosts materializados em `~/.ssh/config.d/tyba.conf` via Include (dividendo: `ssh`/`scp`/DBeaver ganham os hosts e túneis de graça), broadcast para N sessões com gate por risco (#157), persistência via tmux invisível no host (#158 — a SSH Session sobrevive a wifi, sleep e ⌘Q), túneis `-L`/`-R`/`-D` visuais com gate pela direção do risco e bind explícito (#159). **Atenção**: agente digitado num pane SSH roda cru na máquina remota — sem jaula nem inbox; documentado na doc pública, aviso na UI pendente.
- **Docker como sessão**: containers locais e do host remoto (via a conexão SSH) viram sessões de terminal com a mesma UX.
- **Painel de CI no forge** (#149): checks da PR visíveis no painel, sem sair do TYBA.
