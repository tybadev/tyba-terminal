#!/bin/sh
# `tyba` do pane: a CLI fina sobre o TYBA aberto (`tyba ls`). Este diretorio
# entra na frente do PATH do pane. Tudo vai para `tyba _cli`, que nunca abre
# o app: subcomando desconhecido sai com 2.
if [ -n "${TYBA_BIN:-}" ] && [ -x "$TYBA_BIN" ]; then
  exec "$TYBA_BIN" _cli "$@"
fi
echo "tyba: fora de um pane do TYBA (sem TYBA_BIN)" >&2
exit 4
