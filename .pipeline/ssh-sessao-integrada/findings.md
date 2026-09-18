---
project: tyba
feature: ssh-sessao-integrada
type: review
status: liberado
created: 2026-09-18
updated: 2026-09-18
branch: feat/ssh-sessao-integrada
---

# Review — ssh-sessao-integrada

Rodada 2 de no máximo 2. Spec: `.pipeline/ssh-sessao-integrada/spec.md`

Duas rodadas, cada uma com as duas lentes (`reviewer` e `verifier`) em paralelo.
A rodada 2 rodou com instâncias novas, sem a lista da rodada 1 — foi ela que
pegou a lacuna de cobertura da completação remota, que a rodada 1 não viu.

## Cobertura

Medida contra o host real do dono (Ubuntu, bash 5, zsh 5.9, tmux) e na tela do
app de desenvolvimento. **Sete casos de host real, todos verdes.**

| Critério | Estado | Onde se prova |
|---|---|---|
| Sessão nova com bash mostra blocos com código de saída | coberto | `tests/ssh_integrated_real_host.rs` — `a_sessao_integrada_sobe_e_fala_o_protocolo_de_controle` (fecha em `133;D;0`) · na tela: `ls -la` virou bloco com o corpo do servidor, e um comando que falhou virou bloco de borda vermelha com o chip `1` |
| O mesmo vale para zsh | coberto | `a_sessao_integrada_com_zsh_sobe_e_fala_o_protocolo_de_controle` |
| Linha de comando do TYBA, com as três situações de teclado | coberto | `src/lib/commandLine.test.ts` · na tela as três apareceram: "Digite um comando", "Comando rodando — o teclado é do terminal" (`top`) e "tmux está no controle — o teclado é dele" |
| `vim`/`top` funcionam e devolvem o teclado | coberto por observação | na tela: `top` rodou dentro de um bloco vivo e, no `q`, o bloco fechou com duração 18,8 s e a linha do TYBA voltou |
| Reatar redesenha sem reexecutar | coberto | `reatar_uma_sessao_viva_redesenha_sem_reexecutar_e_sem_deixar_pasta` — 2 voltas, a marca do processo remoto sobrevive, `screen` zerado antes do redesenho |
| Queda de rede: reconecta e o pane volta | coberto por observação | matei o `ssh` da sessão (`kill -9`); o app subiu outro sozinho, reanexou **a mesma** sessão tmux do servidor, e os comandos seguintes voltaram a virar bloco com saída e chip `/root` |
| Nenhum arquivo do TYBA fica no servidor | coberto | `o_rc_remoto_nao_deixa_rastro_depois_que_a_sessao_sobe` · `ssh::remote_rc` cobre as duas metades da janela de SIGHUP · conferido à mão: `ls -1d "${XDG_RUNTIME_DIR:-/tmp}"/tyba-*` vazio antes e depois de toda a bateria |
| Shell não suportado / integração desligada abre comum com a linha | coberto | `ssh/mod.rs` + `src/lib/remoteSession.test.ts` |
| Sessão viva de antes continua e diz por quê | coberto | `session/mod.rs` — `sessao_restaurada_sem_decisao_gravada_continua_comum` |
| Host sem tmux abre integrado, sem persistência, e diz isso | coberto | `a_sonda_do_host_devolve_shell_e_persistencia_e_o_plano_sai_integrado` |
| Comando remoto no histórico marcado com o Host | coberto | `session/store.rs` — `history_candidates_in`, `in_host` peso 2.0 |
| Autocomplete de comando remoto | coberto | `o_canal_devolve_os_nomes_de_comando_e_o_git_do_servidor` |
| Autocomplete completa caminho remoto | coberto | `a_completacao_de_caminho_traz_as_entradas_do_servidor` — nomes inventados no servidor, e a completação **local** sobre o mesmo caminho volta vazia; é isso que prova que veio de lá |
| Chips mostram pasta/branch/contagem do servidor | coberto | na tela: aba e chip mostram `/root` · `ssh::query::git_chips` · `sidebarChips` impede o chip local na barra lateral |
| Faixa de "sem jaula" para agente no servidor | coberto | `src/lib/remoteSession.test.ts` |
| Broadcast continua entregando | coberto por composição | `lib.rs::broadcast_write` chama `pty_pool.write`, e `pty/mod.rs::escrita_em_modo_de_controle_vira_send_keys` prova que isso vira `send-keys` no transporte de controle |
| Segredo remoto não aparece no histórico | coberto | `session/store.rs` — `segredo_que_vem_pelo_transporte_de_controle_nao_chega_ao_historico` |
| tmux do dono aninhado continua funcionando | coberto | `pty/mod.rs`, `mod alt_screen_tests` (replay de bytes capturados do VPS) + observação na tela: entra limpo e, ao sair, o bloco fecha e a linha do TYBA volta |
| E2E no VPS com bash | coberto | sete casos `--ignored` + a bateria de tela acima |

## Bloqueantes

Nenhum aberto. Os quatro que apareceram foram corrigidos e confirmados.

- **`src-tauri/src/ssh/remote_rc.rs`** — a janela entre criar a pasta e armar a
  armadilha deixava rastro no servidor. O prólogo fazia `d=$(mktemp -d …)` e **só
  depois** `trap tyba_wipe HUP TERM INT`; SIGHUP entre as duas matava o `sh` pela
  ação padrão com a pasta já criada. Falhava em 2 de 4 execuções da suíte
  completa e passava 10 de 10 isolado — não era teste instável, era janela real
  que alarga sob carga.
  *Conserto:* o TYBA gera o sufixo aleatório (token próprio, **não** o nonce dos
  marcos — esse autentica os marcos e não pode virar nome de pasta legível por
  outro usuário do host), a armadilha é armada com o nome já conhecido, e só
  então a pasta nasce com `umask 077`. `mkdir` falha se o caminho existir, o que
  preserva a proteção contra symlink que o `mktemp` dava, e some a dependência do
  `chmod` no PATH remoto. Suíte completa verde em 4 rodadas seguidas.

- **`src-tauri/src/pty/mod.rs`** — sair do tmux aninhado matava o pane: ficava em
  branco, o teclado continuava entregue "ao app", digitar não ecoava. No servidor
  o shell estava vivo.
  *Causa raiz* (a primeira hipótese, região de rolagem, foi medida e descartada —
  o `\033[1;24r` vem depois do `?1049h` e cai no grid alternativo): em
  `apply_screen`, braço `capture::Action::ResetScreen`, `ingest_chunk` dava o
  chunk inteiro ao `vt100` do core, que voltava à tela normal; no **mesmo chunk**
  a máquina de captura achava o `133;D` do prompt e emitia `ResetScreen`; o braço
  fazia `state.pending.clear()` — e a fila esvaziada era a que levava a sequência
  de restauração ao webview, `?1049l` incluído. Core na tela normal, xterm.js
  preso no buffer alternativo.
  *Por que `top` passava:* coalescência. A thread leitora junta todos os
  `ControlEvent::Output` de uma leitura num chunk só, então no transporte de
  controle "restauração + `[exited]` + prompt + `133;D`" no mesmo chunk é a
  regra. Com `top`, saída e prompt tendem a cair em leituras separadas.
  *O conserto é geral*, não específico do tmux aninhado: vale para qualquer app
  de tela cheia cujo `?1049l` caia no mesmo chunk do `133;D`, inclusive em sessão
  local.

- **`src/components/TerminalView.tsx` / `src/lib/remoteSession.ts`** — o xterm.js
  respondia às consultas de terminal dentro do modo de controle. O tmux responde
  às consultas **e** encaminha os bytes crus; o xterm.js respondia de novo, e a
  resposta voltava como digitação via `send-keys` — o lixo `1;2c0;276;0c` no
  prompt do servidor. *Conserto:* silenciar DA1, DA2, DA3, DSR 5/6, DECXCPR 6,
  DECRQM nos dois sabores, XTWINOPS 14/16/18, DECRQSS e OSC 4/10/11/12 com `?`,
  só quando o transporte é `tmux_control`, devolvendo `false` para final ambíguo
  que não é relatório. Guiado pelo evento novo `session://transport/<id>`.

- **`src-tauri/tests/ssh_integrated_real_host.rs`** — falso verde. A conferência
  de rastro usava o glob `tyba-??????`, o molde do `mktemp`. Com o nome novo
  (`tyba-` + 32 hex) o glob deixou de casar qualquer coisa e a asserção passaria
  **sem ter medido nada**. Corrigido para `tyba-*`, e provado no servidor: o glob
  encontra uma pasta que criei e some quando removo.

## Deveria

- **Sem comando de consulta do transporte.** `session://transport/<id>` é
  anunciado no spawn, antes de o front assinar; quem assina depois só vê a
  transição. Hoje é latente — o único ponto de produção que monta
  `Transport::TmuxControl` sempre passa `control_marker: Some(…)`, então a
  transição sai bem depois do login. Uma sessão que nascesse já em modo de
  controle nunca seria anunciada e o front responderia consulta. O padrão já
  existe para integração e modo prompt; falta `session_transport(id)`.
- **O cwd ainda vaza na janela de reconexão** — outra superfície, não o git.
  `displayDir = workspaceCwd(w) ?? w.repo_root` (`src/App.tsx`) alimenta o título
  da aba, o marco de partida do bloco, o nome automático da sessão (`basename`),
  o `SessionHoverCard` e o "copiar diretório". Durante a reconexão isso é `~` /
  `/Users/guilherme`, e o nome da sessão chega a virar "guilherme". Mexe em nome
  de sessão, que tem regra própria (`name_locked`) e testes próprios.
- **A despedida do `ssh` vira linha não reconhecida.** No encerramento o log traz
  `modo de controle do tmux com linha não reconhecida (a sessão segue): Shared
  connection to <host> closed.` É a mensagem do próprio cliente `ssh` chegando
  pelo canal. Não muda comportamento; é ruído que o decodificador deveria
  reconhecer.
- **Os casos de host real exercitam a costura, não a cola de produção.** Nenhum
  chama `SessionManager::spawn_ssh`, `PtyPool::spawn_with_transport` ou
  `Store::insert_block`. O mecanismo (protocolo, rc, canal próprio) está provado
  contra host vivo; o fio que liga isso ao app está provado com mock. O mesmo
  vale para `resume_startup`, sem teste direto.
- **`o_rc_remoto_nao_deixa_rastro…` confere de forma absoluta**, enquanto o caso
  de reatar usa linha de base por diferença. Com a sessão real do dono viva no
  host, o primeiro pode falhar por rastro de outra sessão e apontar o dedo para a
  errada.
- **Modo prompt no reatar segue a preferência global**, não a da sessão.
- **Segredo no bloco gravado é herdado por dedução**, não por teste: existe teste
  para o histórico (`segredo_que_vem_pelo_transporte_de_controle_nao_chega_ao_historico`),
  não para `capture::CaptureMachine` → `Store::insert_block` no cenário composto.

## Nits

- `src-tauri/src/pty/mod.rs:527` — `left_alt_screen` lê só o estado atual do
  parser, não uma comparação antes/depois do chunk, então o `?1049l` é reemitido
  em praticamente todo fim de bloco, não só no caso estreito que o comentário
  descreve. Inofensivo hoje: o `CLEAR_SCREEN` logo em seguida sobrescreve o
  DECRC. Passaria a importar se a ordem dos dois `extend_from_slice` fosse
  invertida ou o `CLEAR_SCREEN` saísse do ramo.
- `HostFormDialog.tsx` usa dez ícones marcados como `deprecated` no Phosphor —
  pré-existente.
- `src/lib/keys.test.ts:257` falha na `main` limpa também; não é desta entrega.

## Escopo não combinado

- **`sessions.ssh_integration TEXT`** entrou na migração 9 sem constar do
  contrato da spec, que lista só `host.integration_enabled` e
  `command_history.host_id` (+ índice). É necessária para distinguir "sessão de
  antes" de "decisão gravada" e está justificada no comentário do código, mas é
  uma terceira mudança de schema que não virou emenda registrada como as outras
  duas (`Transport::TmuxControl`, campo `persistence`).
- **`session://transport/<id>`** é evento novo que a spec não nomeia.
- **A correção do `ResetScreen`** mexe em `ScreenState`, compartilhado por
  **todas** as sessões. Foi descoberta pelo tmux aninhado numa sessão SSH, mas o
  defeito e o conserto valem também para uma sessão local rodando `tmux attach`.
  Correção geral pegando carona na entrega.
- **`Cano::kill` ganhou o campo `morto`** em `tests/ssh_integrated_real_host.rs`:
  o `killpg` podia alcançar um pid já colhido e, com pid reusado, mandar SIGKILL
  para um grupo alheio. Conserto de armadilha pré-existente no arquivo de teste.

## Portões

| | |
|---|---|
| Suíte do core (`--lib`, dez testes de shim pulados) | 1918 passed, 0 failed, 19 ignored |
| Front (`bun test`) | 1024 pass, 1 fail — a pré-existente `keys.test.ts:257` |
| `bun run typecheck` | exit 0 |
| `cargo clippy --lib --tests -- -D warnings` | exit 0 |
| `cargo fmt --check` | exit 0 |
| Host real (`--ignored --test-threads=1`) | 7 passed, 0 failed |
| Servidor depois de toda a bateria | zero pastas órfãs |

## Relacionadas

- `[[tyba/features/ssh/tech-spec-05-sessao-integrada]]`
- `[[tyba/decisions/2026-09-17-ssh-sessao-integrada]]`
- `[[tyba/features/ssh/validation-04-conexao-confiavel]]`
