# TYBA bash shell integration (OSC 133/633/7).
# Launched via: bash --rcfile <this> -i   (sem -l; login shell ignora --rcfile).
# Reproduz a cadeia de init do usuario e instala hooks. Falha do usuario nunca
# derruba a sessao; hooks terminam com `return 0` (nao morrem sob `set -e`).

# --- 1. Reproduz a cadeia de init do usuario (uma unica vez) ---
if [ -z "${TYBA_BASH_INTEGRATION:-}" ]; then
  export TYBA_BASH_INTEGRATION=1
  if [ -n "${TYBA_LOGIN_SHELL:-}" ]; then
    [ -r /etc/profile ] && . /etc/profile
    for __tyba_rc in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
      if [ -r "$__tyba_rc" ]; then . "$__tyba_rc"; break; fi
    done
    unset __tyba_rc
  else
    [ -r "$HOME/.bashrc" ] && . "$HOME/.bashrc"
  fi
fi

# --- 2. Hooks (instala uma unica vez, depois da config do usuario) ---
if [ -z "${TYBA_BASH_HOOKS:-}" ] && case "$-" in *i*) true ;; *) false ;; esac; then
  TYBA_BASH_HOOKS=1
  __tyba_osc133b=$'\033]133;B\007'
  __tyba_at_prompt=0

  __tyba_esc() { printf '\033]%s\007' "$1"; }

  __tyba_urlencode() {
    local s="$1" out="" i c LC_ALL=C
    for (( i=0; i<${#s}; i++ )); do
      c="${s:i:1}"
      case "$c" in
        [A-Za-z0-9/._~-]) out="$out$c" ;;
        *) printf -v c '%d' "'$c"; printf -v c '%%%02X' "$(( c & 0xFF ))"; out="$out$c" ;;
      esac
    done
    printf '%s' "$out"
  }

  __tyba_osc7() {
    __tyba_esc "7;file://${HOSTNAME}$(__tyba_urlencode "$PWD")"
    return 0
  }

  # DEBUG trap: dispara UMA vez por comando digitado.
  __tyba_preexec() {
    [ -n "${COMP_LINE:-}" ] && return 0
    [ "${BASH_SUBSHELL:-0}" = 0 ] || return 0
    [ "${__tyba_at_prompt:-0}" = 1 ] || return 0
    case "$BASH_COMMAND" in
      __tyba_precmd|__tyba_prompt_ready) return 0 ;;
    esac
    __tyba_at_prompt=0
    local __h
    __h="$(HISTTIMEFORMAT='' builtin history 1)"
    __h="${__h#"${__h%%[![:space:]]*}"}"
    __h="${__h#"${__h%%[!0-9]*}"}"
    __h="${__h#"${__h%%[![:space:]]*}"}"
    __tyba_esc "633;E;$(printf '%s' "$__h" | base64 | tr -d '\n')"
    __tyba_esc "133;C"
    return 0
  }

  # PRIMEIRO no PROMPT_COMMAND: captura $? antes de qualquer outro hook.
  __tyba_precmd() {
    local __c=$?
    __tyba_at_prompt=0
    __tyba_esc "133;D;$__c"
    __tyba_esc "133;A"
    case "$PS1" in
      *"$__tyba_osc133b"*) ;;
      *) PS1="$PS1\[$__tyba_osc133b\]" ;;
    esac
    __tyba_osc7
    return 0
  }

  # ULTIMO no PROMPT_COMMAND: arma o disparo do preexec.
  __tyba_prompt_ready() {
    __tyba_at_prompt=1
    return 0
  }

  # PROMPT_COMMAND: array em bash >= 5.1, string abaixo disso.
  if [ "${BASH_VERSINFO[0]}" -gt 5 ] || { [ "${BASH_VERSINFO[0]}" -eq 5 ] && [ "${BASH_VERSINFO[1]}" -ge 1 ]; }; then
    case "${PROMPT_COMMAND[*]:-}" in
      *__tyba_precmd*) ;;
      *) PROMPT_COMMAND=(__tyba_precmd "${PROMPT_COMMAND[@]}" __tyba_prompt_ready) ;;
    esac
  else
    case ";${PROMPT_COMMAND:-};" in
      *";__tyba_precmd;"*) ;;
      *) if [ -z "${PROMPT_COMMAND:-}" ]; then
           PROMPT_COMMAND="__tyba_precmd;__tyba_prompt_ready"
         else
           PROMPT_COMMAND="__tyba_precmd;${PROMPT_COMMAND};__tyba_prompt_ready"
         fi ;;
    esac
  fi

  trap '__tyba_preexec' DEBUG
fi
