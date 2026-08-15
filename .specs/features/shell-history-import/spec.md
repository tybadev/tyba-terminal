# Import de histórico de shell — Specification

Notação: a narrativa é pt-BR (convenção do repo); os critérios de aceite são EARS,
que é notação formal e fica em inglês, como código.

## Problem Statement

O motor de histórico do TYBA está pronto — captura por `OSC 633;E`/`133;D`, frecência com escopo de cwd/repo, redação antes do SQLite (`src-tauri/src/history/mod.rs`) — e nasce sem dado. Quem instala o TYBA já tem anos de comando em `~/.zsh_history`, mas a paleta abre vazia e leva semanas até ranquear qualquer coisa. O motor existe e não tem o que ordenar; é o dado de entrada que falta, não o algoritmo.

## Goals

- [ ] Buscar na paleta, na primeira sessão depois do import, um comando digitado meses atrás fora do TYBA.
- [ ] Reimportar não altera a contagem de linhas de `command_history`.
- [ ] Nenhum secret conhecido em texto claro no banco depois de importar uma fixture sintética que contenha um.
- [ ] Import de 100 000 entradas sem segurar o flush de output do PTY.

## Out of Scope

| Feature | Reason |
| ------- | ------ |
| Sync de histórico entre máquinas | Exige servidor; é decisão registrada de roadmap, não dívida (`docs/ROADMAP.md`, "Explicitamente fora de escopo") |
| Import do `history.db` do atuin | Leitor SQLite com schema de terceiro; entra como P3 sem segurar as três fontes-texto |
| Import de PowerShell e nushell | Sem hook de shell nesses ainda; o import entra junto com o hook, não antes |
| Export do histórico do TYBA | Problema oposto ao desta entrega |
| Import de blocos (comando + saída) | Arquivo de histórico de shell não guarda saída; não há o que importar |
| Import de alias, env e dotfiles | Território do atuin; não é histórico |

---

## Assumptions & Open Questions

| Assumption / decision | Chosen default | Rationale | Confirmed? |
| --------------------- | -------------- | --------- | ---------- |
| Gatilho do import | Botão explícito em Settings; convite no primeiro uso como P2 | Copiar arquivo pessoal para o banco do TYBA sem clique é o que a gente cobra dos outros | y |
| Teto de `command_history` | Sobe de 20 000 para 100 000 linhas | 100k linhas de comando é da ordem de poucos MB; com o teto atual cada comando novo expulsaria um importado (eviction é FIFO por `id`) | y |
| Fontes no P1 | zsh, bash e fish | Os três são arquivo-texto e compartilham o pipeline; muda só o parser de linha | y |
| `exit_code` ausente não é falha | `frecency` só aplica o demérito de fracasso quando existe ao menos um exit code conhecido para aquele comando | Hoje `successes` conta `exit_code = 0` (`store.rs:810`) e bash/fish não gravam exit code: sem isso, **todo o corpus importado nasce com peso 0,5**, como se só tivesse falhado | n |
| Entrada sem timestamp (bash sem `HISTTIMEFORMAT`) | Distribuir para trás a partir do `mtime` do arquivo, 1 s por entrada, preservando a ordem do arquivo | `started_at_ms` é NOT NULL e a frecência é recência × frequência: zerar joga tudo para 1970 e o import vira invisível; carimbar tudo com "agora" faz o importado atropelar o comando real de ontem | n |
| `cwd` de entrada importada | NULL | zsh, bash e fish não gravam diretório; inventar um faria o escopo de cwd/repo mentir, e é ele que mais pesa no ranking | n |
| `session_id` de entrada importada | NULL | A entrada não pertence a nenhuma sessão do TYBA; a coluna já é nullable | n |
| Mesmo comando em duas fontes | Duas linhas, não uma | zsh e bash são dois eventos reais; frequência é o sinal que a frecência usa | n |
| Idempotência | Marca de origem persistida por fonte + posição/timestamp da entrada | Não existe constraint de unicidade em `command_history` hoje; a marca é o que permite reimportar para pegar o que foi digitado desde a última vez sem duplicar o resto | n |
| Redação no import | A mesma `session::redact::redact` da captura ao vivo, sem caminho alternativo | Duas implementações de redação divergem, e a que diverge é a que vaza | n |

**Open questions:** none — all resolved or logged above.

---

## User Stories

### P1: Importar zsh, bash e fish ⭐ MVP

**User Story**: Como usuário novo do TYBA, quero trazer o histórico de comando que já tenho no shell, para que a paleta e a sugestão sejam úteis na primeira sessão em vez de daqui a semanas.

**Why P1**: Sem isso o motor de frecência não tem o que ranquear, e a fatia inteira não entrega valor.

**Acceptance Criteria**:

1. WHEN the user triggers the import from Settings THEN the system SHALL read `$HISTFILE` or `~/.zsh_history`, `$HISTFILE` or `~/.bash_history`, and `$XDG_DATA_HOME/fish/fish_history` or `~/.local/share/fish/fish_history`, importing every source that exists.
2. WHEN a zsh entry is in extended format `: <epoch>:<duration>;<command>` THEN the system SHALL store `started_at_ms` from `<epoch>` and `duration_ms` from `<duration>`.
3. WHEN a zsh entry is in plain format (no `EXTENDED_HISTORY`) THEN the system SHALL store the command with a synthesized timestamp derived from the file mtime and the entry position, preserving file order.
4. WHEN a history entry continues across lines through a trailing backslash THEN the system SHALL store it as a single command.
5. WHEN a fish entry is read from its YAML-like `- cmd:` / `when:` record THEN the system SHALL store the command and the `when` epoch as `started_at_ms`.
6. WHEN an imported command matches a secret pattern THEN the system SHALL store it redacted through the same `redact` used by live capture.
7. WHEN an imported command starts with a space or a tab THEN the system SHALL discard it without storing.
8. WHEN the user runs the import a second time THEN the system SHALL NOT insert a second row for an entry already imported.
9. WHEN the user runs the import after new commands were appended to a source file THEN the system SHALL import only the new entries.
10. WHILE an import is running the system SHALL commit in batches of at most 1 000 entries per transaction, never holding the store lock for the whole file.
11. WHILE an import is running the system SHALL run it off the PTY emitter thread.
12. IF a source file is missing, unreadable, or its content is not valid UTF-8 THEN the system SHALL skip that source or entry, continue with the remaining ones, and report what was skipped and why.
13. IF an import is already running THEN the system SHALL refuse a second concurrent import and report that one is in flight.
14. WHEN the import finishes THEN the system SHALL report, per source, how many entries were read, imported and discarded.
15. WHERE an imported entry carries no exit code the system SHALL store `exit_code` as NULL, never as `0`.
16. The system SHALL cap `command_history` at 100 000 rows.
17. The system SHALL expose the import only to the human UI; no agent-facing command, hook or tool SHALL be able to trigger it.
18. The system SHALL NOT write imported command text to application logs.

**Independent Test**: com fixtures sintéticas de zsh (estendido e simples), bash e fish em um `$HOME` temporário, disparar o import e ver as entradas ranqueadas na paleta; rodar de novo e conferir que a contagem de linhas não muda.

---

### P1: Histórico importado ranqueia junto com o vivo

**User Story**: Como usuário que acabou de importar, quero que o comando importado apareça na busca como qualquer outro, para que o import não tenha sido um no-op silencioso.

**Why P1**: Duas armadilhas do código atual anulam o import sozinhas — o demérito de fracasso (`store.rs:810` + `history::frecency`) e o corte de 2 000 candidatos mais recentes (`HISTORY_CANDIDATES`, `store.rs:229`). Importar sem corrigi-las entrega um banco cheio e uma paleta que não mudou.

**Acceptance Criteria**:

1. WHEN candidates are ranked and every entry of a command has an unknown exit code THEN the system SHALL NOT apply the failure demotion to that command.
2. WHEN candidates are ranked and a command has at least one known exit code THEN the system SHALL apply the failure demotion exactly as today.
3. WHEN the palette searches with a non-empty query THEN the system SHALL consider every distinct matching command in `command_history`, not only the 2 000 most recent ones.
4. WHEN the user clears all command history THEN the system SHALL delete imported entries as well.
5. WHEN the user clears the history of a repository THEN the system SHALL keep imported entries that carry no `cwd`.

**Independent Test**: inserir 3 000 comandos vivos recentes mais um importado antigo e único; buscar o antigo pelo prefixo e vê-lo retornar; conferir que um comando importado sem exit code não fica atrás de um vivo equivalente com sucesso.

---

### P2: Convite no primeiro uso

**User Story**: Como usuário que acabou de instalar, quero ser avisado de que dá para trazer meu histórico, para não depender de encontrar a opção em Settings.

**Why P2**: É descoberta, não capacidade — o P1 já entrega o import inteiro por Settings.

**Acceptance Criteria**:

1. WHEN TYBA starts and no import has ever run and at least one source file exists THEN the system SHALL show a dismissible invitation stating the number of entries found per source.
2. The system SHALL NOT write any entry to `command_history` before the user accepts the invitation.
3. WHEN the user dismisses the invitation THEN the system SHALL NOT show it again.

**Independent Test**: primeiro boot com fixture presente mostra o convite com a contagem certa; recusar e reabrir não mostra de novo.

---

### P3: Importar o `history.db` do atuin

**User Story**: Como usuário de atuin, quero importar o banco dele, para trazer junto o cwd, o exit code e a duração que o arquivo-texto não tem.

**Why P3**: Público menor e leitor diferente (SQLite com schema de terceiro), mas é a única fonte com contexto completo.

**Acceptance Criteria**:

1. WHERE `~/.local/share/atuin/history.db` exists the system SHALL offer it as an import source.
2. WHEN the atuin database is imported THEN the system SHALL store command, `cwd`, `exit_code`, duration and timestamp from it.
3. IF the atuin database does not have the expected schema THEN the system SHALL skip the source and report the reason instead of failing the whole import.

**Independent Test**: com um `history.db` sintético no schema esperado, importar e conferir que o escopo de cwd passa a valer para as entradas importadas.

---

## Edge Cases

- IF a source file exceeds 50 MB THEN the system SHALL still import it, reading it as a stream instead of loading it into memory at once.
- IF a source file is empty THEN the system SHALL report zero entries and finish successfully.
- IF an entry decodes to an empty or whitespace-only command THEN the system SHALL discard it.
- IF the same command text appears in two different sources THEN the system SHALL store both, counting them as two uses.
- IF the import is interrupted by app shutdown THEN the system SHALL keep the batches already committed and resume from the last imported position on the next run.
- IF a `$HISTFILE` points outside the user's home THEN the system SHALL still read it, since it is the shell's own configured location.

---

## Requirement Traceability

| Requirement ID | Story | Phase | Status |
| -------------- | ----- | ----- | ------ |
| HIMP-01 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-02 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-03 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-04 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-05 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-06 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-07 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-08 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-09 | P1: Importar zsh, bash e fish | Tasks | Implementing |
| HIMP-10 | P1: Importar zsh, bash e fish | Design | Pending |
| HIMP-11 | P1: Histórico importado ranqueia junto com o vivo | Tasks | Implementing |
| HIMP-12 | P1: Histórico importado ranqueia junto com o vivo | Design | Pending |
| HIMP-13 | P1: Histórico importado ranqueia junto com o vivo | Design | Pending |
| HIMP-14 | P2: Convite no primeiro uso | - | Pending |
| HIMP-15 | P3: Importar o history.db do atuin | - | Pending |

Mapa dos IDs para os critérios:

- **HIMP-01** leitura das três fontes e resolução de caminho (P1-A 1)
- **HIMP-02** parsers de zsh estendido, zsh simples, bash e fish (P1-A 2, 3, 4, 5)
- **HIMP-03** redação e `ignorespace` no import (P1-A 6, 7, 18)
- **HIMP-04** idempotência e import incremental (P1-A 8, 9)
- **HIMP-05** lote e execução fora do caminho quente (P1-A 10, 11)
- **HIMP-06** fonte com defeito não derruba o import (P1-A 12)
- **HIMP-07** import concorrente recusado (P1-A 13)
- **HIMP-08** relatório por fonte (P1-A 14)
- **HIMP-09** `exit_code` desconhecido é NULL e teto de 100 000 (P1-A 15, 16)
- **HIMP-10** import é superfície humana, nunca de agente (P1-A 17)
- **HIMP-11** demérito de fracasso só com exit code conhecido (P1-B 1, 2)
- **HIMP-12** candidato importado elegível apesar do corte por recência (P1-B 3)
- **HIMP-13** limpeza de histórico cobre o importado (P1-B 4, 5)
- **HIMP-14** convite no primeiro uso (P2 inteiro)
- **HIMP-15** fonte atuin (P3 inteiro)

**Coverage:** 15 total, 0 mapeados para tasks, 15 pendentes ⚠️

---

## Implicit-Requirement Dimensions Sweep

| Dimension | Resolution |
| --------- | ---------- |
| Input validation & bounds | HIMP-02, HIMP-06, HIMP-09; arquivo > 50 MB e arquivo vazio nas edge cases |
| Failure / partial-failure states | HIMP-06 (fonte pulada não derruba as outras), HIMP-05 (lote commitado sobrevive a shutdown) |
| Idempotency / retry / duplicate handling | HIMP-04 |
| Auth boundaries & rate limits | HIMP-10 — o import é do humano; agente nunca dispara. Rate limit N/A porque é ação local, manual e sem rede |
| Concurrency / ordering | HIMP-07 (import concorrente recusado), HIMP-05 (lote não segura o lock), ordem do arquivo preservada em HIMP-02 |
| Data lifecycle / expiry | HIMP-09 (teto de 100 000), HIMP-13 (limpeza cobre importado) |
| Observability | HIMP-08 relatório por fonte; HIMP-03 proíbe texto de comando no log |
| External-dependency failure | N/A no P1 — nenhuma rede, nenhum serviço; no P3 o schema do atuin é tratado em HIMP-15 |
| State-transition integrity | HIMP-07: ocioso → rodando → concluído/falhou, com o segundo disparo recusado enquanto roda |

---

## Success Criteria

- [ ] Buscar na paleta, logo depois do import, um comando digitado meses atrás fora do TYBA e vê-lo retornar.
- [ ] Rodar o import duas vezes seguidas deixa `SELECT COUNT(*) FROM command_history` idêntico.
- [ ] Fixture sintética com `export TOKEN=sk-…` entra no banco redigida — nenhuma ocorrência do segredo em texto claro.
- [ ] Durante um import de 100 000 entradas o output do terminal continua fluindo, sem intervalo de flush acima de 100 ms.
- [ ] Comando importado sem exit code não fica atrás de um comando vivo equivalente com sucesso, a igual recência e frequência.
