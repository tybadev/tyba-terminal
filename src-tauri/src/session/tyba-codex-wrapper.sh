#!/bin/sh
# `codex` do TYBA. Este diretorio entra na frente do PATH do pane, entao
# `command codex`, scripts e Makefiles chegam aqui em vez do binario de
# verdade. Tudo vai para `tyba _codex -- "$@"`, que sobe o codex de verdade
# com o hook pre-push do TYBA (push para main/master) e o guarda-corpo de
# texto, sem gate e sem jaula. Os args seguem como "$@".
#
# Nunca chama a si mesmo: sem o `tyba`, executa o codex de verdade pelo
# caminho que o core resolveu ao abrir o pane, ou pelo PATH pulando este
# diretorio.
if [ -n "${TYBA_BIN:-}" ] && [ -x "$TYBA_BIN" ]; then
  exec "$TYBA_BIN" _codex -- "$@"
fi

self_dir="${0%/*}"
real="${TYBA_REAL_CODEX:-}"
case "$real" in
  /*) ;;
  *) real="" ;;
esac
if [ -n "$real" ] && [ "${real%/*}" != "$self_dir" ] && [ -x "$real" ]; then
  exec "$real" "$@"
fi

old_ifs="$IFS"
IFS=:
for dir in $PATH; do
  case "$dir" in
    /*) ;;
    *) continue ;;
  esac
  [ "$dir" = "$self_dir" ] && continue
  if [ -x "$dir/codex" ]; then
    IFS="$old_ifs"
    exec "$dir/codex" "$@"
  fi
done
IFS="$old_ifs"

echo "tyba: codex nao encontrado no PATH" >&2
exit 127
