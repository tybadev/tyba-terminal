---
project: tyba
feature: ssh-conexao-confiavel
type: review
status: liberado
created: 2026-09-17
updated: 2026-09-17
branch: feat/ssh-conexao-confiavel
---

# Review — ssh-conexao-confiavel

Rodada 2 de no máximo 2, com duas lentes (`reviewer` e `verifier`), instâncias
novas na rodada 2. Spec: `.pipeline/ssh-conexao-confiavel/spec.md` · nota
`[[tyba/features/ssh/tech-spec-04-conexao-confiavel]]` · ADR
`[[tyba/decisions/2026-09-16-ssh-conexao-confiavel]]`.

Perguntas dos `impl-block`: **0** em todos os blocos e correções (3 blocos + 3
correções). O corte e a spec bastaram.

## Evidência de execução

| Portão | Resultado |
|---|---|
| `cargo fmt --check` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test` (10 testes antigos de shell pulados — ver "Fora da entrega") | 1824 passed, 0 failed, 18 ignored |
| `bun run typecheck` | exit 0 |
| `bun test` | 959 pass, 1 fail — `src/lib/keys.test.ts:257`, **preexistente**: falha idêntica num worktree limpo do `main` |
| `cargo test --test ssh_real_host -- --ignored --test-threads=1` contra o VPS do dono | 3/3 ok |
| E2E de tela (`bun run tauri dev`, banco real do dono com backup) | ver abaixo |

E2E de tela, 17/09:

- Migração 8 no banco real: `user_version` 8; host legado carregou como `auto`;
  `tyba.conf` regenerado ganhou só as duas linhas de keepalive.
- O dono conectou o host com `User Root`: o servidor registrou
  `Invalid user Root`, e o pane mostrou o cartão de falha ("Não autenticou",
  Tentar de novo / Editar host). O dono editou para `root` e conectou: sessão
  `running`, `ssh_logged_in=1`, `last_connected_at` gravado no login;
  servidor registrou `Accepted publickey for root`.
- Queda: a sessão principal matou só o `ssh` local do pane. Em 0,3 s o overlay
  "reconectando… a sessão continua viva no host" apareceu (por evento); ~1 s
  depois um Cano novo subiu, o marco atualizou `last_connected_at` e, em 4,5 s,
  o pane mostrava o prompt do host de novo.
- Cinco reinícios seguidos do `tauri dev` durante a correção da rodada 2 + um
  reinício controlado: nenhum Cano órfão, nenhuma sessão tmux escondida no
  host, linha de sessão sem pane descartada no boot.
- Não observado em tela: a faixa de `connecting` com prompt de senha (nenhum
  host de senha no teste), o cartão `host_key_changed`, e o subtítulo da barra
  lateral atualizando depois da correção da rodada 2.

## Cobertura

28 de 30 critérios cobertos, 2 parciais, 0 não cobertos.

| Critério | Estado | Onde se prova |
|---|---|---|
| Render por método (regras 2–5), inclusive `auto` legado | coberto | `ssh/config.rs` `metodo_agente_…`, `metodo_arquivo_…`, `metodo_senha_…`, `auto_legado_so_ganha_keepalive` |
| Nunca `UseKeychain` nem `IgnoreUnknown` | coberto | `ssh/config.rs` `nenhuma_combinacao_escreve_usekeychain_nem_ignoreunknown` |
| Keepalive na ordem da regra 9 | coberto | `ssh/config.rs` `keepalive_vem_depois_dos_forwards_e_antes_do_multiplex` + E2E |
| `.pub` 0600/0700, nome pela digital, órfãos removidos | coberto | `ssh/config.rs` `pub_das_chaves_de_agente_nasce_privado…` |
| `agent_key` forjada recusada | coberto | `ssh/agent_keys.rs` `digital_forjada_e_recusada`, `chave_em_mais_de_uma_linha…` |
| Migração 7 → 8 com backfill | coberto | `session/store.rs` teste v7→v8 + E2E no banco real |
| Render inválido não grava linha nem `tyba.conf` | coberto | `ssh/config.rs` `validar_nao_instala_nada…`; `lib.rs` `host_que_o_ssh_recusa_nao_chega_ao_banco_nem_ao_tyba_conf` |
| Classificador da regra 15 | coberto | `ssh/classify.rs` (um caso por motivo, precedência, corte em 300) |
| Observador: marco partido, nonce errado, 16 KiB, para de guardar | coberto | `session/cano.rs` testes de `CanoWatch` |
| Marco antes do tmux e do shell; csh | coberto | `ssh/tmux.rs` |
| Saída antes do login fora de queda → `failed`, sem sonda nem respawn | coberto | `session/cano.rs` `primeira_conexao_que_sai_antes_do_login_falha_sem_probe_nem_respawn` + E2E |
| Backoff 1/2/4/8/16/30 e `dropped` aos 300 s | coberto | `session/cano.rs` |
| Login < 30 s não zera a queda; ≥ 30 s zera | coberto | `session/cano.rs` |
| Dentro da queda só `no_route`/`host_unresolved` insistem | coberto | `session/cano.rs` `dentro_da_queda_so_rede_ausente_continua_tentando` |
| Nenhuma transição fora da tabela | coberto | `session/cano.rs` `so_as_transicoes_da_regra_13_existem` + exploração exaustiva |
| Respawn herda attachers e tamanho | coberto | `pty/mod.rs` `religar_o_cano_no_mesmo_id_herda_janelas_e_tamanho` + E2E |
| Marco → `live`, `ssh_logged_in`, `last_connected_at`; spawn não toca | coberto | `session/mod.rs` `marco_de_login_poe_no_ar_e_grava_login_e_ultimo_acesso` + E2E |
| Boot esquece sem login e religa os outros como queda | **parcial** | decisão coberta (`session/cano.rs` `boot_religa_sessao_com_login_como_queda_que_comecou_no_boot`, `boot_esquece_…`, `boot_so_religa_ssh_quando_a_pref_religa`; `session/mod.rs` `boot_ssh`); a cola `resume_startup` → `spawn_cano` (`lib.rs`) só por leitura e pelos reinícios do E2E |
| `mergeSessionUpdate` propaga `connection` e `connection_failure` | coberto | `src/lib/sessionStatus.test.ts` + E2E (overlay por evento) |
| Argv do teste de conexão + limpeza em sucesso/falha/prazo | coberto | `ssh/test_conn.rs` `argv_leva_todas_as_opcoes_da_regra_22`, `falha_limpa_tudo…`, `prazo_estourado_mata_o_ssh…` |
| Desfechos da regra 23 | coberto | `ssh/test_conn.rs` |
| `passphrase_required` | coberto | `ssh/test_conn.rs` `chave_cifrada_fora_do_agente_pede_passphrase` (chave real gerada no teste) |
| Lista do agente (64 chaves, `ssh-agent` descartável, sem socket) | coberto | `ssh/agent_keys.rs` `listagem_para_em_64_chaves`, `agente_descartavel_…`, `sem_socket_resolvido_e_lista_vazia_sem_socket` |
| Env do login shell numa invocação, com fallback | coberto | `shell_path.rs` |
| Guarda: nenhum `new("ssh")` fora de `ssh/command.rs` | coberto | `ssh/command.rs` `nenhum_ssh_nasce_fora_do_construtor` |
| Host de senha sem master → `ssh.password_needs_session` na hora | coberto | `ssh/command.rs` `host_de_senha_sem_master_falha_na_hora` |
| `Input`/`Textarea` sem autocorreção + prosa religada | coberto | `src/components/ui/textFieldDefaults.test.tsx` |
| Aviso de maiúscula não bloqueia | coberto | `src/lib/hostForm.test.ts` `usuário com maiúscula é enviado como digitado` |
| Cartão de falha; `host_key_changed` só com comando copiável | **parcial** | lógica em `src/lib/canoFailure.test.ts`; `auth_refused` visto pelo dono em tela; `host_key_changed` só por leitura de `CanoStatus.tsx` |
| E2E no VPS: `root` conecta pelo gestor; `Root` falha sem reconectar; queda mostra reconexão e o pane volta | coberto | E2E acima |

## Bloqueantes

Nenhum, nas duas rodadas e nas duas lentes.

## Deveria

Todos resolvidos nesta entrega, por decisão do dono:

- **Condutor do Cano sem teste de corrida** (reviewer, rodada 1) — o condutor
  saiu do `lib.rs` para `session/cano.rs` com portas injetadas (`CanoPorts`,
  `conduct`); `lib.rs` só implementa as portas. Testes: aba fechada durante a
  espera ou com a sonda em voo não religa; `dispose` esconde a sessão antes de
  soltar o ciclo (prova por mutação: inverter a ordem em `dispose` quebra o
  teste).
- **Boot religando sem teste** (verifier, rodada 1) — decisão extraída para
  `ssh_boot`/`boot_ssh`, com testes. A cola final segue sem teste (parcial
  acima).
- **Desvios do contrato** (verifier, rodada 1) — aceitos e declarados: o teste
  de conexão usa `StrictHostKeyChecking=yes` e `-T` (sem eles um `accept-new`
  do usuário gravaria `known_hosts` e `host_unknown` não aparece), afirmados em
  teste com comentário; `test_host_connection` pode devolver
  `ssh.auth_fields_conflict`, com teste.
- **Agente sem socket mostrava erro genérico** (reviewer, rodada 2) —
  `list_agent_keys` sem socket resolvido agora devolve `socket: null` e lista
  vazia; o front mostra a orientação de configurar o agente.
- **Subtítulo `user@host` desatualizado depois de editar o host** (E2E) —
  comportamento anterior à entrega, que ela deixou mais visível. O core passou
  a emitir `ssh://hosts-changed` depois de toda gravação de host/grupo; App e
  tela de conexões releem. Um teste amarra o nome do evento entre Rust e TS.
  Não verificado em tela.

## Nits

- A guarda de `new("ssh")` é textual e contornável por reformatação; é o que a
  spec pede ao pé da letra.
- Os três pontos que chamam `require_session_if_password` (docker, SFTP, túnel)
  não têm teste próprio; a função tem.
- `Host.auth_method` e `agent_key` são opcionais no TS, embora o core sempre
  envie `auth_method`.

## Escopo não combinado

- **Evento `ssh://hosts-changed`** — contrato novo, acrescentado na rodada 2
  com o aval do dono. Entra na nota do cofre no ship.
- **Redação de segredos no `detail`** da falha antes de ir à UI.
- **Notas do Grupo** também religam a autocorreção (a regra 29 citava só as
  notas do Host).
- **`forget_unlogged` em todos os modos de boot** — a regra 19 diz "no boot"
  sem condicionar a modo.
- **`.gitignore` e `CLAUDE.md`** fora dos write-sets: a entrada
  `.pipeline/*/state.json` veio do `pipeline-state.sh init` e o `vault_path`
  foi decidido pelo dono na fase-doc — os dois antes de qualquer bloco.
- **`PtyPool::terminate`** (mata o grupo e mantém o handle) substituiu `kill`
  no túnel de sessão sem master.

## Riscos abertos

- **`AddKeysToAgent yes` com o 1Password como `IdentityAgent` continua não
  medido.** O dono salvou o host como `auto`, então o método `arquivo` não foi
  exercitado. O desenho (§5) diz: se o `ssh` avisar ou falhar, a regra 3 volta
  ao `fase-arq`.
- A faixa de `connecting` com prompt de senha não foi vista em tela.
- O workspace SSH mostra o chip git do `$HOME` local — lacuna anterior, fica
  para a `tech-spec-05`.

## Fora da entrega (achados da sessão)

- **Testes antigos de shell usam o `HOME` real** (`session/mod.rs`,
  `run_interactive_shell` e afins): 10 testes abrem `bash -i`/`zsh -i`, sujam
  `~/.bash_history`/`~/.zsh_history` e, com o `.zshrc` do dono, sobem o
  `claude` de verdade, sem interação, no checkout — e o Stop hook do pipeline
  alcança essas sessões. Nesta entrega foram pulados com `--skip`. Correção
  pendente em outra entrega (worktree `fix/shell-tests-private-home` já criado
  por uma dessas sessões, limpo).
- `src/lib/keys.test.ts:257` falha no `main` também.

## Relacionadas

- `[[tyba/features/ssh/tech-spec-04-conexao-confiavel]]`
- `[[tyba/decisions/2026-09-16-ssh-conexao-confiavel]]`
- `[[tyba/decisions/2026-07-15-ssh-credenciais-delegacao-zero-segredo]]`
