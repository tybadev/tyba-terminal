#!/bin/sh
# `claude` do TYBA (M6-A do claude-monitor). Este diretorio entra na frente do
# PATH do pane, entao `command claude`, scripts e Makefiles chegam aqui em vez
# do binario de verdade. Tudo vai para `tyba _jail -- "$@"`, que decide como o
# claude sobe (hospedado com gate, cru para versao/ajuda, recusado se
# desligaria o gate). Os args seguem como "$@": nada vira string de shell.
#
# Nunca chama a si mesmo: sem o `tyba`, executa o claude de verdade pelo
# caminho que o core resolveu ao abrir o pane, ou pelo PATH pulando este
# diretorio.
if [ -n "${TYBA_BIN:-}" ] && [ -x "$TYBA_BIN" ]; then
  exec "$TYBA_BIN" _jail -- "$@"
fi

self_dir="${0%/*}"
real="${TYBA_REAL_CLAUDE:-}"
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
  if [ -x "$dir/claude" ]; then
    IFS="$old_ifs"
    exec "$dir/claude" "$@"
  fi
done
IFS="$old_ifs"

echo "tyba: claude nao encontrado no PATH" >&2
exit 127
