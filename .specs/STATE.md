# STATE — TYBA

<!-- MACHINE-WRITTEN, human-audited. Decisões nunca são apagadas, só superadas
     (anotar "supersedes AD-NNN"). No resume, o Handoff é reconciliado contra o
     git — EVIDÊNCIA GANHA de snapshot velho. -->

## Decisions

### AD-000 — Onde vivem os artefatos deste pipeline
- Decision: `.specs/` é o diretório de trabalho do spec-driven; o roadmap do projeto continua em `docs/ROADMAP.md` e **não** é duplicado em `.specs/ROADMAP.md`, e o papel de AGENTS.md é do `CLAUDE.md` que já existe. A spec consolidada vai para o cofre (`swell-docs`) quando a feature fecha.
- Why: Duas fontes de roadmap divergem, e a que diverge é a que alguém lê por engano.
- Phase: shell-history-import / Specify
- Date: 2026-08-15

### AD-001 — Teto de `command_history` sobe de 20 000 para 100 000
- Decision: `COMMAND_HISTORY_CAP` passa a 100 000 linhas.
- Why: Com 20 000 e eviction FIFO por `id`, cada comando novo expulsaria uma entrada importada, e o import se desfaria sozinho em semanas.
- Phase: shell-history-import / Specify
- Date: 2026-08-15

### AD-002 — Exit code desconhecido não conta como fracasso
- Decision: o demérito de 0,5 em `history::frecency` só se aplica quando existe ao menos um exit code conhecido para aquele comando; NULL é desconhecido, não falha.
- Why: bash e fish não gravam exit code. Sem isso, todo o corpus importado nasce rebaixado como se só tivesse falhado.
- Phase: shell-history-import / Specify
- Date: 2026-08-15

### AD-003 — O import é superfície humana
- Decision: nenhum comando, hook ou tool acessível a sessão de agente dispara o import de histórico.
- Why: o import lê arquivo pessoal fora do worktree; é ação do dono da máquina, não de um agente enjaulado.
- Phase: shell-history-import / Specify
- Date: 2026-08-15

### AD-004 — Eviction de `command_history` é por idade, não por ordem de inserção
- Decision: o teto da tabela passa a cortar pela entrada mais antiga (`started_at_ms`), não pelo menor `id`.
- Why: com FIFO por `id`, importar 100 000 entradas com `id` novo expulsa justamente as linhas vivas, que têm `id` menor — o import apagaria o histórico real do usuário para caber o importado.
- Phase: shell-history-import / Design
- Date: 2026-08-15

## Handoff

- Current phase: shell-history-import — fatia 1 (fases 1 a 4) concluída e validada
- Status: done (P1); P2 (convite) e P3 (atuin) não iniciadas
- Branch: feat/shell-history-import, no worktree `.claude/worktrees/shell-history-import`
- Last commit: validação da feature, com `validation.md` em PASS
- Next step: abrir o PR da fatia 1. Depois dele, T13 (convite no primeiro uso) e T14 (fonte atuin)
- Open assumptions: as seis premissas da spec viraram código e teste. Nenhuma ficou por confirmar
- Dívida declarada: nenhum teste importa 100 000 entradas pelo caminho do produto — a medição de perf saiu de SQLite avulso. É ensaio de release, não gate de merge
