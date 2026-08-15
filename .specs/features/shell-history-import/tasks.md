# Import de histórico de shell — Tasks

## Execution Protocol (MANDATORY -- do not skip)

Implement these tasks with the `spec-driven` skill: **activate it by name and follow its Execute flow and Critical Rules.** Do not search for skill files by filesystem path. The skill is the source of truth for the full flow (per-task cycle, sub-agent delegation, adequacy review, Verifier, discrimination sensor).

**If the skill cannot be activated, STOP and tell the user - do not proceed without it.**

---

**Design**: `.specs/features/shell-history-import/design.md`
**Status**: Draft

Worktree: `.claude/worktrees/shell-history-import`, branch `feat/shell-history-import`. Nada disto nasce na `main`.

---

## Test Coverage Matrix

> Gerada do código, das diretrizes do projeto e da spec — confirmar antes do Execute. Diretrizes encontradas: `CLAUDE.md` ("testes unit para parsers são obrigatórios — são a parte mais frágil"), `.github/workflows/gates.yml`, `package.json`.

| Code Layer | Required Test Type | Coverage Expectation | Location Pattern | Run Command |
| ---------- | ------------------ | -------------------- | ---------------- | ----------- |
| Rust — lógica pura (parser, frecência, chave de import) | unit | Todos os ramos; 1:1 com os AC da spec; toda edge case listada tem teste | `#[cfg(test)] mod tests` no próprio arquivo | `cargo test` (em `src-tauri/`) |
| Rust — store / SQLite | unit | Caminhos de query e de erro; migration idempotente; segue os testes já existentes em `store.rs` | `#[cfg(test)] mod tests` em `session/store.rs` | `cargo test` |
| Rust — runner / orquestração | unit | Lote, import concorrente recusado, fonte quebrada, relatório | `#[cfg(test)] mod tests` em `history/import/mod.rs` | `cargo test` |
| Rust — comando Tauri / fronteira | unit | A fronteira (import não alcançável por agente) tem teste; o resto é build gate | `#[cfg(test)] mod tests` | `cargo test` |
| Frontend — lógica testável (decisão de exibir convite) | unit | Todos os ramos | `src/lib/*.test.ts` | `bun test` |
| Frontend — wrapper de IPC, i18n, componente | none | — (build gate) | — | `bun run typecheck` |

## Gate Check Commands

> Gerados do código — confirmar antes do Execute.

| Gate Level | When to Use | Command |
| ---------- | ----------- | ------- |
| Quick | Depois de task só com teste unit de Rust | `cd src-tauri && cargo test` |
| Full | Depois de task que toca core e front | `cd src-tauri && cargo test` + `bun test` |
| Build | Fim de fase, ou task de config/componente | `cd src-tauri && cargo fmt --check && cargo clippy -- -D warnings && cargo test` + `bun run typecheck && bun test` |

---

## Execution Plan

### Phase 1: Corrigir o que anula o import

Vem antes de qualquer parser: sem estas três, importar enche o banco e não muda nada para o usuário — ou pior, apaga o histórico vivo.

```
T1 → T2 → T3
```

### Phase 2: Parsers

Lógica pura, um formato por task. É a parte frágil, e cada uma é testável sozinha.

```
T3 → T4 → T5 → T6
```

### Phase 3: Pipeline de import

```
T6 → T7 → T8 → T9 → T10
```

### Phase 4: Superfície humana

```
T10 → T11 → T12
```

### Phase 5: Convite no primeiro uso (fatia 2)

```
T12 → T13
```

### Phase 6: Fonte atuin (fatia 3)

```
T13 → T14
```

---

## Task Breakdown

### T1: Demérito de fracasso só com exit code conhecido ✅

**What**: `HistoryCandidate` ganha `known_exit_codes`, a consulta de candidatos passa a agregá-lo e `frecency` só aplica o corte de 0,5 quando há código conhecido e nenhum é zero.
**Where**: `src-tauri/src/history/mod.rs`
**Depends on**: None
**Reuses**: `history::frecency` e seus testes existentes; a agregação de `Store::history_candidates`
**Requirement**: HIMP-11

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [x] `frecency` distingue as três situações: só falhou, nenhum código conhecido, misto
- [x] Teste novo para cada uma das três; os testes de frecência existentes continuam passando
- [x] A consulta em `store.rs` preenche o campo com `SUM(CASE WHEN exit_code IS NOT NULL THEN 1 ELSE 0 END)`
- [x] Gate check passa: `cd src-tauri && cargo test` — 1174 passaram, 0 falharam
- [x] Contagem de testes: 3 novos em `history`, 1 novo em `store` (nenhum apagado)

**Tests**: unit
**Gate**: quick

**Commit**: `fix(history): exit code desconhecido deixa de contar como fracasso na frecência`

---

### T2: Eviction por idade e teto de 100 000 ✅

**What**: o corte do teto passa a apagar a entrada mais antiga por `started_at_ms` em vez do menor `id`, e `COMMAND_HISTORY_CAP` vai de 20 000 para 100 000.
**Where**: `src-tauri/src/session/store.rs`
**Depends on**: T1
**Reuses**: o `DELETE` de eviction já existente em `insert_command`
**Requirement**: HIMP-09

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [x] Teste que hoje não existe: passar do teto apaga a entrada **mais antiga**, não a de menor `id`
- [x] Teste com entrada de data velha inserida depois de entrada recente: a recente sobrevive
- [x] `COMMAND_HISTORY_CAP` = 100 000, fixado por teste
- [x] Gate check passa: `cd src-tauri && cargo test` — 1177 passaram, 0 falharam
- [x] Contagem de testes: 3 novos (nenhum apagado)

**Tests**: unit
**Gate**: quick

**Commit**: `fix(history): eviction por idade em vez de ordem de inserção`

---

### T3: Pré-filtro em SQL para a busca com query

**What**: `Store::history_candidates_matching` filtra por `LIKE` antes de agregar, e `search_command_history` passa a usá-lo quando a query não é vazia.
**Where**: `src-tauri/src/session/store.rs`
**Depends on**: T2
**Reuses**: a agregação de `history_candidates`; `escape_like`
**Requirement**: HIMP-12

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Teste: com mais candidatos recentes do que o teto da janela, um comando antigo que casa com a query ainda é retornado
- [ ] Query vazia continua indo pelo caminho de recência
- [ ] Medição registrada no commit: tempo da consulta com 100 000 linhas
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 2 novos (nenhum apagado)

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): busca com query considera todo o histórico, não só a janela recente`

---

### T4: Parser de zsh

**What**: parser de `~/.zsh_history` cobrindo formato estendido (`: <epoch>:<dur>;<cmd>`), formato simples e continuação por `\`.
**Where**: `src-tauri/src/history/import/parser/zsh.rs`
**Depends on**: T3
**Reuses**: —
**Requirement**: HIMP-02

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Itera sobre `BufRead`, sem `read_to_string`
- [ ] Testes com fixture **sintética**: estendido, simples, continuação por `\`, linha com UTF-8 inválido, arquivo vazio
- [ ] Entrada sem timestamp recebe data sintetizada para trás a partir do `mtime`, preservando a ordem
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 6 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): parser de histórico do zsh`

---

### T5: Parser de bash

**What**: parser de `~/.bash_history` com e sem linha `#<epoch>` de `HISTTIMEFORMAT`, e continuação por `\`.
**Where**: `src-tauri/src/history/import/parser/bash.rs`
**Depends on**: T4
**Reuses**: a síntese de timestamp de T4
**Requirement**: HIMP-02

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Testes com fixture sintética: sem timestamp, com `#<epoch>`, continuação, arquivo vazio
- [ ] Ordem do arquivo preservada na data sintetizada
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 5 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): parser de histórico do bash`

---

### T6: Parser de fish

**What**: parser do `fish_history`, precedido da verificação do formato na documentação oficial do fish.
**Where**: `src-tauri/src/history/import/parser/fish.rs`
**Depends on**: T5
**Reuses**: —
**Requirement**: HIMP-02

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Formato conferido na doc do fish **antes** de escrever o parser; o achado vai no comentário do módulo (escape de newline dentro de `cmd:` é o ponto)
- [ ] Fixture sintética derivada da doc, nunca de arquivo real
- [ ] Testes: registro simples, comando multilinha, `when` ausente, arquivo vazio
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 5 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): parser de histórico do fish`

---

### T7: Resolução e contagem de fontes

**What**: `import::source` resolve o caminho de cada fonte (`$HISTFILE` quando presente no ambiente do core, senão o padrão) e conta as entradas sem gravar nada.
**Where**: `src-tauri/src/history/import/source.rs`
**Depends on**: T6
**Reuses**: os parsers de T4–T6
**Requirement**: HIMP-01

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Testes com `$HOME` temporário: fonte presente, ausente, `$HISTFILE` apontando para outro caminho
- [ ] `scan` não escreve no banco (teste verifica contagem de linhas inalterada)
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 4 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): resolução e contagem das fontes de histórico`

---

### T8: Chave de import e gravação em lote

**What**: migration de `import_key` com índice UNIQUE e `Store::insert_imported_batch` com `INSERT OR IGNORE` numa transação por lote.
**Where**: `src-tauri/src/session/store.rs`
**Depends on**: T7
**Reuses**: o padrão de migration idempotente do `open`; `sha2`
**Requirement**: HIMP-04

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Migration roda duas vezes sem erro (idempotente), com teste
- [ ] Teste: inserir o mesmo lote duas vezes não muda a contagem de linhas
- [ ] Teste: linha viva (chave nula) não colide com outra linha viva
- [ ] Não usa `insert_command`; a eviction roda uma vez, fora do laço
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 4 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): chave de import com índice único e gravação em lote`

---

### T9: Runner do import

**What**: orquestra leitura → parse → `should_record` + `redact` → lote de 1 000, um import por vez, com relatório por fonte.
**Where**: `src-tauri/src/history/import/mod.rs`
**Depends on**: T8
**Reuses**: `redact`, `should_record`, os parsers, `insert_imported_batch`
**Requirement**: HIMP-03, HIMP-05, HIMP-06, HIMP-07, HIMP-08

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Teste: fixture com `export TOKEN=sk-…` entra redigida — o segredo não aparece em texto claro no banco
- [ ] Teste: comando iniciado por espaço não é gravado
- [ ] Teste: fonte ilegível é pulada com motivo e as demais seguem
- [ ] Teste: segundo import concorrente é recusado
- [ ] Teste: relatório traz lidas, importadas e descartadas por fonte
- [ ] Texto de comando não aparece em log
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 6 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): runner do import com redação, lote e relatório por fonte`

---

### T10: Comandos Tauri e fronteira de agente

**What**: `scan_shell_history_sources` e `import_shell_history` registrados, com evento de progresso, e o teste de fronteira que trava o import fora do alcance de sessão de agente.
**Where**: `src-tauri/src/lib.rs`
**Depends on**: T9
**Reuses**: o padrão de progresso de `lsp/managed`; `AppError { code, params }`
**Requirement**: HIMP-10

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Teste de fronteira: a superfície alcançável por sessão de agente (`HookAction`) não expõe import
- [ ] Progresso emitido por evento, não por retorno
- [ ] Gate check passa: `cd src-tauri && cargo fmt --check && cargo clippy -- -D warnings && cargo test`
- [ ] Contagem de testes: 2 novos

**Tests**: unit
**Gate**: build

**Commit**: `feat(history): comandos de import de histórico`

---

### T11: IPC e i18n do import

**What**: wrappers `scanShellHistorySources` / `importShellHistory`, tipos do relatório e chaves de texto pt/en.
**Where**: `src/lib/ipc.ts`
**Depends on**: T10
**Reuses**: o padrão dos wrappers de histórico já existentes; o dicionário de i18n
**Requirement**: HIMP-08

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Tipos espelham `ImportReport` e `SourceScan` do core
- [ ] Chaves pt e en presentes nas duas línguas
- [ ] Gate check passa: `bun run typecheck && bun test`

**Tests**: none
**Gate**: build

**Commit**: `feat(history): ipc e textos do import`

---

### T12: Seção de import em Configurações

**What**: botão de importar, lista de fontes com contagem, barra de progresso e relatório ao fim.
**Where**: `src/components/ShellSettings.tsx`
**Depends on**: T11
**Reuses**: a seção de histórico existente (toggle e limpar), o padrão de toast
**Requirement**: HIMP-01, HIMP-08

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Fonte ausente não aparece; fonte pulada aparece com o motivo
- [ ] Import em andamento desabilita o botão
- [ ] Gate check passa: `bun run typecheck && bun test`

**Tests**: none
**Gate**: build

**Commit**: `feat(history): seção de import de histórico nas configurações`

---

### T13: Convite no primeiro uso

**What**: a decisão de exibir o convite (nunca exibido antes, nenhuma importação feita, ao menos uma fonte com entradas) e a dispensa persistida.
**Where**: `src/lib/historyImportInvite.ts`
**Depends on**: T12
**Reuses**: a preferência persistida do core; `scanShellHistorySources`
**Requirement**: HIMP-14

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Testes de todos os ramos da decisão, incluindo dispensado e já importado
- [ ] Nada é gravado em `command_history` antes do aceite (teste no core)
- [ ] Gate check passa: `bun test` e `cd src-tauri && cargo test`
- [ ] Contagem de testes: 5 novos no front, 1 novo no core

**Tests**: unit
**Gate**: full

**Commit**: `feat(history): convite de import no primeiro uso`

---

### T14: Fonte atuin

**What**: leitor do `history.db` do atuin, trazendo comando, cwd, exit code, duração e data.
**Where**: `src-tauri/src/history/import/parser/atuin.rs`
**Depends on**: T13
**Reuses**: o pipeline de T9; `rusqlite`
**Requirement**: HIMP-15

**Tools**:

- MCP: NONE
- Skill: NONE

**Done when**:

- [ ] Teste com `history.db` sintético no schema esperado
- [ ] Teste: schema inesperado é pulado com motivo, sem derrubar o import
- [ ] Entradas trazem cwd, e o escopo de diretório passa a valer para elas
- [ ] Gate check passa: `cd src-tauri && cargo test`
- [ ] Contagem de testes: 4 novos

**Tests**: unit
**Gate**: quick

**Commit**: `feat(history): import do banco do atuin`

---

## Phase Execution Map

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6

Phase 1:  T1 ------→ T2 ------→ T3
Phase 2:  T4 ------→ T5 ------→ T6
Phase 3:  T7 ------→ T8 ------→ T9 ------→ T10
Phase 4:  T11 -----→ T12
Phase 5:  T13
Phase 6:  T14
```

Fases 1 a 4 são a fatia 1 e entregam valor sozinhas. Fase 5 é a fatia 2 e fase 6 a fatia 3 — podem ser cortadas sem quebrar nada.

---

## Task Granularity Check

| Task | Scope | Status |
| ---- | ----- | ------ |
| T1: demérito com código conhecido | 1 função pura + a agregação que a alimenta | ✅ Granular |
| T2: eviction por idade e teto | 1 conceito (capacidade da tabela), 1 arquivo | ✅ Granular |
| T3: pré-filtro em SQL | 1 método de store | ✅ Granular |
| T4: parser zsh | 1 parser | ✅ Granular |
| T5: parser bash | 1 parser | ✅ Granular |
| T6: parser fish | 1 parser | ✅ Granular |
| T7: resolução e contagem de fontes | 1 módulo | ✅ Granular |
| T8: chave e lote | 1 migration + 1 método coeso no mesmo arquivo | ✅ Granular |
| T9: runner | 1 orquestrador | ✅ Granular |
| T10: comandos Tauri | 2 comandos coesos no mesmo arquivo | ✅ Granular |
| T11: IPC e i18n | 1 wrapper + chaves | ✅ Granular |
| T12: seção de Configurações | 1 componente | ✅ Granular |
| T13: convite | 1 módulo de decisão | ✅ Granular |
| T14: fonte atuin | 1 parser | ✅ Granular |

---

## Diagram-Definition Cross-Check

| Task | Depends On (task body) | Diagram Shows | Status |
| ---- | ---------------------- | ------------- | ------ |
| T1 | None | — | ✅ Match |
| T2 | T1 | T1 → T2 | ✅ Match |
| T3 | T2 | T2 → T3 | ✅ Match |
| T4 | T3 | T3 → T4 (fronteira de fase) | ✅ Match |
| T5 | T4 | T4 → T5 | ✅ Match |
| T6 | T5 | T5 → T6 | ✅ Match |
| T7 | T6 | T6 → T7 (fronteira de fase) | ✅ Match |
| T8 | T7 | T7 → T8 | ✅ Match |
| T9 | T8 | T8 → T9 | ✅ Match |
| T10 | T9 | T9 → T10 | ✅ Match |
| T11 | T10 | T10 → T11 (fronteira de fase) | ✅ Match |
| T12 | T11 | T11 → T12 | ✅ Match |
| T13 | T12 | T12 → T13 (fronteira de fase) | ✅ Match |
| T14 | T13 | T13 → T14 (fronteira de fase) | ✅ Match |

Nenhuma dependência aponta para fase posterior.

---

## Test Co-location Validation

| Task | Code Layer Created/Modified | Matrix Requires | Task Says | Status |
| ---- | --------------------------- | --------------- | --------- | ------ |
| T1 | Rust lógica pura + store | unit | unit | ✅ OK |
| T2 | Rust store | unit | unit | ✅ OK |
| T3 | Rust store | unit | unit | ✅ OK |
| T4 | Rust lógica pura | unit | unit | ✅ OK |
| T5 | Rust lógica pura | unit | unit | ✅ OK |
| T6 | Rust lógica pura | unit | unit | ✅ OK |
| T7 | Rust lógica pura | unit | unit | ✅ OK |
| T8 | Rust store | unit | unit | ✅ OK |
| T9 | Rust runner | unit | unit | ✅ OK |
| T10 | Rust comando/fronteira | unit | unit | ✅ OK |
| T11 | Frontend wrapper/i18n | none | none | ✅ OK |
| T12 | Frontend componente | none | none | ✅ OK |
| T13 | Frontend lógica testável + core | unit | unit | ✅ OK |
| T14 | Rust lógica pura | unit | unit | ✅ OK |
