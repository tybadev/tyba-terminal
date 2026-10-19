#!/bin/sh
# Hook do plugin TYBA para o Claude Code. Roda em TODA sessao do Claude da
# maquina, entao fora do TYBA tem que custar nada: so testes de variavel, sem
# jq e sem chamar binario nenhum, e sai com 0.
#
# Fora de um pane do TYBA: nao ha TYBA_PANE_TOKEN.
[ -n "${TYBA_PANE_TOKEN:-}" ] || exit 0
# Claude ja com gate (sessao gerenciada ou hospedada): os hooks do
# `--settings` da sessao ja falam com o TYBA. Nada de evento em dobro.
[ -n "${TYBA_HOOK_SOCKET:-}" ] && exit 0
[ "${TYBA_HOSTED:-}" = "1" ] && exit 0
# Claude cru dentro de um pane: o TYBA observa e aplica o guarda-corpo de
# texto (merge e aprovacao de PR). Sem o binario (app movido ou
# desinstalado), sai 0 — nunca trava o Claude.
[ -n "${TYBA_BIN:-}" ] && [ -x "$TYBA_BIN" ] || exit 0
exec "$TYBA_BIN" _plugin-hook
