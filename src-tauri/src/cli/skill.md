---
name: tyba
description: Use o comando `tyba` quando estiver rodando dentro de um pane do TYBA (env `TYBA_PANE` definido) para rodar comandos numa aba visível e ler o resultado, ler o que apareceu em outro pane, acompanhar outros agentes ou mandar mensagem a um agente parado.
---
<!-- Gerado por `tyba skill`. Rode de novo depois de atualizar o TYBA. -->

# CLI do TYBA

Só funciona dentro do TYBA aberto (`TYBA_PANE` no env). Fora dele, a CLI sai
com o código 4. Todo comando aceita `--json`.

## Rodar um comando que a pessoa deve ver

```bash
tyba run --wait -- 'cargo test 2>&1 | tail -40'
```

Abre uma aba nova, roda o comando, imprime a saída (redigida) e sai com o
código do comando. Prefira isto a rodar escondido quando a pessoa quer ver o
progresso (testes longos, build, servidor). `--cwd <pasta>` escolhe a pasta.
Sem `--wait`, imprime o id do pane e volta na hora.

## Ver o que existe e o que apareceu

```bash
tyba ls --json                 # workspaces, panes, agente e estado de cada um
tyba capture <pane> --lines 80 # as últimas linhas do pane, já sem segredos
```

`<pane>`: id, rótulo, posição no `tyba ls` ou fim do id.

## Acompanhar mudanças

```bash
tyba events --json
```

Uma linha JSON por mudança (`pane_opened`, `pane_closed`, `agent`), até o app
fechar. Nunca termina sozinho: leia com um limite (`| head -n 1`) ou num
processo em segundo plano.

## Mandar mensagem a outro agente

```bash
tyba send <pane> 'rode os testes e me diga se passou'
```

Só entrega a um Claude Code ou Codex **parado esperando**. Sai com 1 se o agente
estiver trabalhando, num prompt de permissão, ou se o pane for um shell.

## Limites (não tente contornar)

- `run` e `send` recusam merge e aprovação de PR, `--no-verify`, mexer em
  `core.hooksPath`/`GIT_CONFIG_*`. O shell do `run` tem o hook que barra push
  para main. A recusa é definitiva: avise a pessoa em vez de procurar outro
  caminho.
- `send` não aceita texto que começa com `!` ou `/`, nem `exit`/`quit`, e
  nunca responde prompt de permissão.
- Uma linha só, sem TAB nem caractere de controle.

Códigos de saída: 0 ok, 1 falha ou recusa, 2 uso inválido, 3 pane não achado ou
ambíguo, 4 TYBA fechado, 130 interrompido.
