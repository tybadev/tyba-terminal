# TYBA shell integration (OSC 133/633/7) + modo prompt do TYBA.
# Sourceado no fim do .zshrc gerado em ZDOTDIR, DEPOIS da config do usuario.

if [[ -o interactive ]] && autoload -Uz add-zsh-hook 2>/dev/null; then
  __tyba_esc() { printf '\033]%s\007' "$1"; }

  __tyba_urlencode() {
    emulate -L zsh
    local s=$1 out= i c
    for (( i=1; i<=${#s}; i++ )); do
      c=$s[i]
      if [[ $c == [A-Za-z0-9/._~-] ]]; then
        out+=$c
      else
        out+=$(printf '%%%02X' ${(s: :)$(printf '%s' $c | od -An -tu1)})
      fi
    done
    printf '%s' $out
  }

  __tyba_osc7() { __tyba_esc "7;file://${HOST}$(__tyba_urlencode "$PWD")"; }

  __tyba_ps1b() { [[ "$PS1" == *$'\033]133;B'* ]] || PS1="$PS1%{$(__tyba_esc '133;B')%}"; }

  # --- Modo prompt do TYBA ---------------------------------------------------
  # A linha de comando passa a ser um editor do app; o PS1 sai da tela. Sem
  # isso nao ha onde desenhar sugestao: o zle e dono da linha, do cursor e do
  # redraw.
  #
  # p10k/starship reconstroem o PS1 a cada precmd, entao o prompt e reaplicado
  # ali, nao uma vez so. O original e guardado na PRIMEIRA vez, que e o unico
  # momento em que ele ainda e o do usuario.
  __tyba_prompt_mode=${TYBA_PROMPT_MODE:-0}
  __tyba_prompt_saved=0

  # Framework que repinta ASSINCRONO desfaz o PS1 vazio meio segundo depois: ele
  # recalcula em background e chama `zle reset-prompt` com o prompt dele de
  # volta. Verificado em pty real com Spaceship (2026-07-31).
  #
  # No spawn isso ja vem por env, mas o toggle em sessao viva (Alt+~) nao tem
  # esse caminho — dai a mesma troca aqui, e reversivel.
  __tyba_prompt_sync_on() {
    if [[ -z ${__tyba_saved_async+x} ]]; then
      __tyba_saved_async=${SPACESHIP_PROMPT_ASYNC-__tyba_none}
    fi
    SPACESHIP_PROMPT_ASYNC=false
  }

  __tyba_prompt_sync_off() {
    [[ -n ${__tyba_saved_async+x} ]] || return 0
    if [[ $__tyba_saved_async == __tyba_none ]]; then
      unset SPACESHIP_PROMPT_ASYNC
    else
      SPACESHIP_PROMPT_ASYNC=$__tyba_saved_async
    fi
    unset __tyba_saved_async
  }

  __tyba_prompt_apply() {
    if [[ $__tyba_prompt_saved == 0 ]]; then
      __tyba_saved_ps1=$PS1
      __tyba_saved_rprompt=$RPROMPT
      __tyba_prompt_saved=1
    fi
    __tyba_prompt_sync_on
    PS1="%{$(__tyba_esc '133;B')%}"
    RPROMPT=""
  }

  __tyba_prompt_restore() {
    __tyba_prompt_sync_off
    [[ $__tyba_prompt_saved == 1 ]] || return 0
    PS1=$__tyba_saved_ps1
    RPROMPT=$__tyba_saved_rprompt
  }

  # Valvula de escape: toda heuristica falha, e o caminho de volta nao pode ser
  # fechar o app. O core dispara esta sequencia; o usuario tambem alcanca pelo
  # teclado.
  __tyba_prompt_toggle() {
    if [[ $__tyba_prompt_mode == 1 ]]; then
      __tyba_prompt_mode=0
      __tyba_prompt_restore
    else
      __tyba_prompt_mode=1
      __tyba_prompt_apply
    fi
    __tyba_esc "633;P;tyba-prompt=$__tyba_prompt_mode"
    zle && zle reset-prompt
  }
  zle -N __tyba_prompt_toggle

  __tyba_preexec() {
    __tyba_esc "633;E;$(print -rn -- "$1" | base64 | tr -d '\n')"
    __tyba_esc "133;C"
  }

  __tyba_precmd() {
    local __c=$?
    __tyba_esc "133;D;$__c"
    __tyba_esc "133;A"
    if [[ $__tyba_prompt_mode == 1 ]]; then
      __tyba_prompt_apply
    else
      __tyba_ps1b
    fi
    __tyba_osc7
    __tyba_esc "633;P;tyba-prompt=$__tyba_prompt_mode"
  }

  add-zsh-hook preexec __tyba_preexec
  add-zsh-hook precmd __tyba_precmd
  add-zsh-hook chpwd __tyba_osc7

  # Alt+~ alterna o modo; Alt+= limpa a linha antes de o app enviar texto.
  # O Warp usa ^P para o kill-buffer e com isso ROUBA o history-up do usuario —
  # nao copiar essa escolha. Se estas sequencias conflitarem com algum plugin,
  # sao o ponto unico a mudar.
  bindkey '\e~' __tyba_prompt_toggle
  bindkey '\e=' kill-buffer

  __tyba_osc7
fi
