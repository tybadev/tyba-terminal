# TYBA bash shell integration (OSC 133/633/7).
# Launched via: bash --rcfile <this> -i   (sem -l; login shell ignora --rcfile).
# Reproduz a cadeia de init do usuario e instala hooks. Falha do usuario nunca
# derruba a sessao; hooks terminam com `return 0` (nao morrem sob `set -e`).

# --- 1. Reproduz a cadeia de init do usuario (uma unica vez) ---
# Roda a cadeia de login (paridade com o `-l` antigo) e depois ~/.bashrc, que e
# onde a maioria poe alias/prompt. Se o profile ja sourceia o bashrc (padrao das
# distros), nao sourceia de novo: PATH duplicado e nvm/rbenv reinicializados.
if [ -z "${TYBA_BASH_INTEGRATION:-}" ]; then
  export TYBA_BASH_INTEGRATION=1

  __tyba_sources_bashrc() {
    local f="$1" line trimmed
    [ -n "$f" ] && [ -r "$f" ] || return 1
    while IFS= read -r line || [ -n "$line" ]; do
      trimmed="${line#"${line%%[![:space:]]*}"}"
      case "$trimmed" in \#*|'') continue ;; esac
      if [[ "$trimmed" =~ (^|[[:space:]])(\.|source)[[:space:]]+[^[:space:]]*bashrc ]]; then
        return 0
      fi
    done < "$f"
    return 1
  }

  __tyba_profile=""
  if [ -n "${TYBA_LOGIN_SHELL:-}" ]; then
    [ -r /etc/profile ] && . /etc/profile
    for __tyba_rc in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
      if [ -r "$__tyba_rc" ]; then __tyba_profile="$__tyba_rc"; . "$__tyba_rc"; break; fi
    done
    unset __tyba_rc
  fi

  if [ -r "$HOME/.bashrc" ] && ! __tyba_sources_bashrc "$__tyba_profile"; then
    . "$HOME/.bashrc"
  fi
  unset __tyba_profile
  unset -f __tyba_sources_bashrc
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
    local __raw __n __h
    __raw="$(HISTTIMEFORMAT='' builtin history 1)"
    __raw="${__raw#"${__raw%%[![:space:]]*}"}"
    # O NÚMERO do histórico, guardado para comparar com o do comando anterior.
    __n="${__raw%%[![:digit:]]*}"
    __h="${__raw#"$__n"}"
    __h="${__h#"${__h%%[![:space:]]*}"}"
    # Histórico que não avançou = o bash NÃO guardou este comando.
    #
    # É o que `HISTCONTROL=ignorespace` faz com linha iniciada por espaço — e
    # aí `history 1` devolve o comando ANTERIOR. Sem esta guarda o hook reporta
    # o comando errado, registrando de novo o que já tinha rodado.
    #
    # O zsh não precisa disso: lá o `preexec` recebe a linha CRUA em `$1`, com
    # o espaço inicial. No bash não há equivalente — o `DEBUG` trap só tem
    # `BASH_COMMAND`, já expandido e disparado uma vez por elo do pipeline.
    #
    # Devolver o comando com um espaço à frente reusa a regra que o core já
    # aplica (`history::should_record`): não entra no histórico, mas ainda vira
    # bloco. `ignoredups` cai aqui também, e o efeito é o mesmo — o comando
    # repetido não é regravado, o que é justamente o que se pediu ao repetir.
    if [ -n "${__tyba_hist_n:-}" ] && [ "$__n" = "$__tyba_hist_n" ]; then
      __h=" $BASH_COMMAND"
    fi
    __tyba_hist_n="$__n"
    __tyba_esc "633;E;$(printf '%s' "$__h" | base64 | tr -d '\n')"
    __tyba_esc "133;C"
    return 0
  }

  # --- Modo prompt do TYBA ---------------------------------------------------
  # A linha de comando passa a ser um editor do app e o PS1 sai da tela. O
  # original e guardado na PRIMEIRA aplicacao — depois disso o que esta em PS1
  # ja e o nosso.
  __tyba_prompt_mode="${TYBA_PROMPT_MODE:-0}"
  __tyba_prompt_saved=0

  __tyba_prompt_apply() {
    if [ "$__tyba_prompt_saved" = 0 ]; then
      __tyba_saved_ps1="$PS1"
      __tyba_prompt_saved=1
    fi
    PS1="\[$__tyba_osc133b\]"
    return 0
  }

  __tyba_prompt_restore() {
    [ "$__tyba_prompt_saved" = 1 ] || return 0
    PS1="$__tyba_saved_ps1"
    return 0
  }

  # Valvula de escape: toda heuristica falha, e o caminho de volta nao pode ser
  # fechar o app.
  __tyba_prompt_toggle() {
    if [ "$__tyba_prompt_mode" = 1 ]; then
      __tyba_prompt_mode=0
      __tyba_prompt_restore
    else
      __tyba_prompt_mode=1
      __tyba_prompt_apply
    fi
    __tyba_esc "633;P;tyba-prompt=$__tyba_prompt_mode"
    return 0
  }

  # PRIMEIRO no PROMPT_COMMAND: captura $? antes de qualquer outro hook.
  __tyba_precmd() {
    local __c=$?
    __tyba_at_prompt=0
    __tyba_esc "133;D;$__c"
    __tyba_esc "133;A"
    if [ "$__tyba_prompt_mode" = 1 ]; then
      __tyba_prompt_apply
    else
      case "$PS1" in
        *"$__tyba_osc133b"*) ;;
        *) PS1="$PS1\[$__tyba_osc133b\]" ;;
      esac
    fi
    __tyba_osc7
    __tyba_esc "633;P;tyba-prompt=$__tyba_prompt_mode"
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

  # Alt+~ alterna o modo; Alt+= limpa a linha antes de o app enviar texto.
  # O Warp usa ^P para o kill-buffer e com isso ROUBA o history-up do usuario —
  # nao copiar essa escolha. Ponto unico a mudar se conflitar com algum plugin.
  bind -x '"\e~": __tyba_prompt_toggle' 2>/dev/null
  bind '"\e=": kill-whole-line' 2>/dev/null

  trap '__tyba_preexec' DEBUG
fi

# --- Shim de agente v2 (passo 2: canal shim<->core) -------------------------
#
# POR QUE FUNCAO DE SHELL, e nao OSC. O gate do TYBA e montado no spawn do
# agente: jaula, env por allowlist e a config de hook que faz ele pedir licenca
# antes de agir. Nada disso da para pendurar num processo ja nascido, entao
# `claude` digitado no shell fica para sempre sem gate — e a unica oferta
# possivel depois e matar e subir de novo, que o dono nao aceita.
#
# Interceptar exige um ponto de BLOQUEIO antes do exec. OSC 133/633 dispara no
# preexec, sim, mas OSC e saida, mao unica: quando o core le o byte o exec ja
# aconteceu. Funcao de shell bloqueia, e e o idioma que direnv, nvm e pyenv
# usam ha anos — inspecionavel com `type claude`, com escotilha conhecida.
#
# SO `claude` PURO, SEM ARGUMENTO NENHUM, GATEIA (Q1, ADR 2026-09-02):
# `claude <args>` cai cru pra `command claude "$@"`. Encaminhar "$@" pro canal
# reabriria a superfície que o teste anti-bypass fecha — o pedido ao core e
# sempre "hospede claude", nunca "execute isto com estes argumentos".
#
# SEM exec: `"$TYBA_BIN" _jail` roda como FILHO desta funcao, no MESMO pane. O
# `exec` substituiria o PROPRIO shell — sair do agente mataria a aba. A cadeia
# tyba -> _seccomp-exec -> bwrap -> claude e mutacao-exec de UM UNICO filho da
# shell; a shell em si nunca e tocada.
#
# FALHA ABERTA e a regra permanente: sem TYBA_BIN, o shim inteiro nem se
# instala (bloco abaixo). Com TYBA_BIN setado mas inalcancavel/quebrado, ou
# sem o core respondendo, ou sem jaula — o `||` final cai pro binario de
# verdade e sai do caminho. Um terminal que se recusa a rodar o que foi
# digitado esta quebrado, e ligado por padrao o raio disso e todo mundo.
if [ -n "${TYBA_BIN:-}" ] && [ "${TYBA_AGENT_SHIM:-1}" = "1" ]; then
  __tyba_shim_record() {
    # Rastro so quando alguem pede (o teste pede). Em uso normal a variavel nao
    # existe e isto e um `[ -n "" ]` — nenhum processo, nenhum arquivo.
    [ -n "${TYBA_SHIM_LOG:-}" ] && printf "%s\n" "$1" >> "$TYBA_SHIM_LOG"
    return 0
  }

  claude() {
    __tyba_shim_record "claude"
    if [ "$#" -eq 0 ]; then
      "$TYBA_BIN" _jail || command claude
    else
      command claude "$@"
    fi
  }
fi
