# Import de histórico de shell — Validation

**Date**: 2026-08-15
**Spec**: `.specs/features/shell-history-import/spec.md`
**Diff range**: `main..HEAD` (a215916..HEAD, 15 commits)
**Verifier**: passe independente na sessão principal (sem sub-agente, por escolha do dono na aprovação das tasks)
**Escopo**: fatia 1 — fases 1 a 4 (P1 completo). P2 (convite) e P3 (atuin) não foram executadas e ficam fora deste relatório.

**Result**: PASS

---

## Task Completion

| Task | Status | Notes |
| ---- | ------ | ----- |
| T1 | ✅ Done | `dad5895` |
| T2 | ✅ Done | `c5fa76b` |
| T3 | ✅ Done | `46306e9` — escopo estendido em uma linha (`HistoryHit.failed`), registrado no critério |
| T3b | ✅ Done | `d24e00f` — nasceu da medição do T3, aprovado pelo dono antes de executar |
| T4 | ✅ Done | `0870255` |
| T5 | ✅ Done | `2aa9f89` |
| T6 | ✅ Done | `46f59b1` |
| T7 | ✅ Done | `be73090` |
| T8 | ✅ Done | `5b67989` |
| T9 | ✅ Done | `913a5a0` |
| T10 | ✅ Done | `ad7cd7d` |
| T11 | ✅ Done | `6357338` |
| T12 | ✅ Done | `6fbe3c7` |
| T13, T14 | ⏭️ Fora da fatia | P2 e P3, cortadas na aprovação |

---

## Spec-Anchored Acceptance Criteria

### P1: Importar zsh, bash e fish

| Critério | Resultado esperado pela spec | `file:line` + asserção | Resultado |
| --- | --- | --- | --- |
| 1. Lê as três fontes, `$HISTFILE`/padrão e `$XDG_DATA_HOME` | toda fonte existente é importada | `history/import/source.rs:186` — `assert_eq!(found[0].source, ImportSource::Zsh)`; `:199` — `assert_eq!(found[0].source, ImportSource::Fish)` | ✅ |
| 2. zsh estendido traz início e duração | `epoch × 1000`, `duração × 1000` | `parser/zsh.rs:166` — `assert_eq!(found[0].started_at_ms, 1_786_820_227_000)`; `:167` — `assert_eq!(found[0].duration_ms, Some(12_000))` | ✅ |
| 3. zsh simples recebe data sintetizada preservando a ordem | data para trás a partir do `mtime`, 1 s por entrada | `parser/zsh.rs:178` — `assert_eq!(found[0].started_at_ms, mtime - 2_000)` | ✅ |
| 4. Continuação por `\` vira um comando só | uma entrada com a quebra dentro | `parser/zsh.rs:189` — `assert_eq!(found[0].command, "for i in 1 2\n  echo $i\nend")` | ✅ |
| 5. Registro do fish traz comando e `when` | `when × 1000` | `parser/fish.rs:154` — `assert_eq!(found[0].started_at_ms, 1_786_820_677_000)` | ✅ |
| 6. Comando com secret entra redigido | mesma `redact` da captura ao vivo | `history/import/mod.rs:301` — `assert!(found[0].contains("[REDACTED]"))` | ✅ |
| 7. Comando iniciado por espaço é descartado | não é gravado | `history/import/mod.rs:311` — `assert_eq!(commands(&store), vec!["pwd"])` | ✅ |
| 8. Reimportar não insere a mesma entrada de novo | contagem de linhas igual | `history/import/mod.rs:392` — `assert_eq!(second.sources[0].imported, 0)`; `session/store.rs:2216` — `assert_eq!(history_count(&store), 2)` | ✅ |
| 9. Reimportar traz só o que é novo | só as entradas novas entram | `history/import/mod.rs:392` — `assert_eq!(second.sources[0].read, 2)` e `assert_eq!(second.sources[0].imported, 1)` sobre arquivo que cresceu | ✅ |
| 10. Lote de no máximo 1 000 por transação | resultado independe do tamanho do lote | `history/import/mod.rs:378` — `assert_eq!(report.sources[0].imported, 5)` com lote 2 | ✅ |
| 11. Roda fora da thread emissora do PTY | — | comando Tauri, fora do caminho do PTY por construção | ⚠️ Garantia estrutural, sem asserção |
| 12. Fonte com defeito é pulada com motivo, as outras seguem | relatório traz o motivo | `history/import/mod.rs:356` — `assert!(report.sources[0].skipped.is_some())` e `:358` — `assert_eq!(report.sources[1].imported, 1)` | ✅ |
| 13. Import concorrente é recusado | erro dedicado | `history/import/mod.rs:373` — `assert!(matches!(nested, Err(ImportError::AlreadyRunning)))` | ✅ |
| 14. Relatório por fonte: lidas, importadas, descartadas | os três números | `history/import/mod.rs:313-315` — `assert_eq!(report.sources[0].read, 2)`, `imported, 1`, `discarded, 1` | ✅ |
| 15. Sem exit code grava NULL, nunca `0` | `known_exit_codes == 0` | `session/store.rs:2196` — `assert_eq!(found[0].known_exit_codes, 0)` | ✅ |
| 16. Teto de 100 000 | constante fixada | `session/store.rs:2280` — `assert_eq!(COMMAND_HISTORY_CAP, 100_000)` | ✅ |
| 17. Import só pela UI humana | nenhum canal de agente dispara | `hook_ipc/framing.rs:141` — `assert_eq!(answers, ["allow", "deny", "ack"])` | ⚠️ Tripwire, não prova: trava o conjunto de respostas do único canal do agente |
| 18. Texto de comando não vai para log | — | nenhuma chamada de log no módulo; `SourceOutcome` não tem campo de comando | ⚠️ Garantia estrutural, sem asserção |

### P1: Histórico importado ranqueia junto com o vivo

| Critério | Resultado esperado pela spec | `file:line` + asserção | Resultado |
| --- | --- | --- | --- |
| 1. Sem código conhecido, sem demérito | mesmo peso de um comando bem-sucedido | `history/mod.rs:311` — `assert_eq!(frecency(now, &unknown), frecency(now, &works))` | ✅ |
| 2. Com código conhecido, demérito como antes | metade do peso | `history/mod.rs:328` — `assert_eq!(frecency(now, &broken), frecency(now, &works) * 0.5)` | ✅ |
| 3. Busca considera todo comando que casa | o antigo aparece apesar da janela | `session/store.rs:2318` — `assert!(com_query.iter().any(\|c\| c.command == "deploy-legacy"))` | ✅ |
| 4. Limpar tudo apaga o importado também | contagem zera | `session/store.rs:2238` — `assert_eq!(history_count(&store), 0)` depois de importar e limpar | ✅ |
| 5. Limpar por repo mantém o importado sem `cwd` | sobrevive à limpeza do repo | `session/store.rs:2235` — `assert_eq!(remaining_commands(&store), vec!["importado"])` | ✅ |

**Status**: ✅ 23 de 23 critérios cobertos — 20 com asserção direta e 3 (P1-A 11, 17, 18) por garantia estrutural, nomeadas abaixo em vez de contadas como asserção.

---

## Discrimination Sensor

| # | Mutação | Arquivo | Morto? |
| --- | --- | --- | --- |
| 1 | Frecência volta a tratar exit code nulo como fracasso | `history/mod.rs` | ✅ Morto |
| 2 | Eviction volta a cortar por ordem de inserção | `session/store.rs` | ✅ Morto |
| 3 | Pré-filtro vira substring em vez de subsequência | `session/store.rs` | ✅ Morto |
| 4 | zsh deixa de desfazer a metafication | `parser/zsh.rs` | ✅ Morto |
| 5 | Import deixa de redigir o comando | `import/mod.rs` | ✅ Morto |
| 6 | Índice de import deixa de ser único | `session/store.rs` | ✅ Morto |
| 7 | bash trata qualquer comentário como data | `parser/bash.rs` | ❌ Sobreviveu → teste reforçado → ✅ Morto |
| 8 | Import deixa de respeitar `ignorespace` | `import/mod.rs` | ✅ Morto |

**Profundidade**: 8 mutações, acima do mínimo leve — a entrega mexe em ranking já publicado e em caminho de dado pessoal.
**Isolamento**: backup por arquivo e restauro com `trap`, nunca `git stash` (numa árvore limpa o stash não cria entrada e deixaria a falha para trás). `git status --porcelain` conferido antes e depois: idêntico.
**Resultado**: 8/8 mortos após a correção — ✅

**O que o sobrevivente ensinou**: a checagem de dígitos do `#<epoch>` do bash parecia redundante com o `parse::<i64>`, e não é: sem ela, `#-123` digitado no prompt viraria data em vez de comando. O teste passou a cobrir o negativo.

---

## Code Quality

| Princípio | Status |
| --- | --- |
| Código mínimo | ✅ |
| Mudança cirúrgica | ✅ |
| Sem escopo extra | ⚠️ Uma linha além do previsto no T3 (`HistoryHit.failed`), declarada no critério e no commit em vez de embutida |
| Segue os padrões do repo | ✅ migration idempotente como as existentes; teste inline `#[cfg(test)]`; erro como `String` no comando, como os vizinhos |
| Asserções batem com o resultado que a spec define | ✅ |
| Profundidade por camada | ✅ lógica pura com 1:1 por AC; store com caminhos de query e erro |
| Todo teste mapeia para um requisito | ✅ |
| Diretrizes seguidas | `CLAUDE.md` (parser com teste unit obrigatório; fixture sempre sintética), `.github/workflows/gates.yml` |

---

## Edge Cases

- [x] Arquivo vazio: zero entradas, sem erro — `parser/zsh.rs:216`, `bash.rs:181`, `fish.rs:216`
- [x] Entrada que não decodifica: descartada e contada — `parser/zsh.rs:207`, `bash.rs:170`, `fish.rs:205`
- [x] Mesmo comando em duas fontes: duas linhas — chaves diferem pelo prefixo da fonte (`import/mod.rs:57`)
- [x] `$HISTFILE` fora do home: lido do mesmo jeito — `source.rs:214`
- [ ] Arquivo acima de 50 MB: leitura em fluxo garantida por construção (`read_until` sobre `BufRead`, sem `read_to_string`), **sem teste de volume**
- [ ] Import interrompido por fechamento do app retoma do ponto: coberto por construção (lote é transação, chave impede duplicata), **sem teste de interrupção**

---

## Gate Check

- **Comando**: `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test` + `bun run typecheck` + `bun test`
- **Resultado**: fmt limpo, clippy limpo, 1224 testes Rust passaram, 547 testes de front passaram, 0 falhas
- **Testes antes da entrega**: 1170 Rust
- **Testes depois**: 1224 Rust (+54), 18 ignorados (pré-existentes, todos de plataforma)
- **Falhas**: nenhuma
- **Testes apagados ou enfraquecidos**: nenhum

---

## Requirement Traceability Update

| Requisito | Antes | Depois |
| --- | --- | --- |
| HIMP-01 a HIMP-09 | Implementing | ✅ Verified |
| HIMP-10 | Implementing | ⚠️ Verified por construção (fronteira de tipo + tripwire) |
| HIMP-11, HIMP-12 | Implementing | ✅ Verified |
| HIMP-13 | Implementing | ✅ Verified |
| HIMP-14, HIMP-15 | Pending | Fora da fatia (P2, P3) |

---

## Summary

**Geral**: ✅ Pronto

**Checagem contra a spec**: 23 de 23 cobertos (20 por asserção, 3 por construção)
**Sensor**: 8/8 mortos
**Gate**: 1224 Rust + 547 front, 0 falhas

**O que funciona**: importar zsh, bash e fish por Configurações, com redação na entrada, `ignorespace` respeitado, reimport sem duplicata, fonte quebrada isolada, relatório por fonte, e o histórico importado disputando o ranking em pé de igualdade com o vivo.

**Lacunas fechadas nesta rodada** (as duas que a primeira passagem apontou):

1. **Limpeza por repo mantém o importado** — a passagem inicial não tinha evidência nenhuma. Teste adicionado em `store.rs`.
2. **Arquivo que cresceu traz só o que é novo** — o teste antigo provava que o já importado não entra de novo, e não o caso real de quem reimporta semanas depois. Teste adicionado em `import/mod.rs`.

**Lacuna que fica, declarada**:

3. **Volume não exercido pelo caminho do produto** — nenhum teste importa 100 000 entradas; a medição de perf saiu de SQLite avulso. É ensaio de release, não gate de merge, e está anotado como tal.

**Três garantias sem asserção, por natureza** (P1-A 11, 17, 18): rodar fora da thread do PTY, o import não ser alcançável por agente, e não haver log de comando. Nenhuma delas é afirmável por teste — o que dá para travar é o conjunto de respostas do canal do agente, e isso está travado em `framing.rs:141`.

**Próximo passo**: abrir o PR.
