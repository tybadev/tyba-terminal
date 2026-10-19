//! Marcadores de sessão de agente que o TYBA não repassa aos panes.
//!
//! Aberto de dentro de um Claude Code (ex.: `open -a "TYBA Dev"` no Bash
//! dele), o TYBA herda o env daquela sessão: `CLAUDE_CODE_CHILD_SESSION`,
//! `CLAUDE_CODE_SESSION_ID`, o token do canal de mensagens… Com o agente
//! livre (fatia 3) o pane passa o env inteiro para o `claude` digitado, que
//! se acha subagente da sessão de fora e desliga a gravação do transcript —
//! a conversa some. Um terminal nunca repassa a identidade de outra sessão,
//! então o core tira esses marcadores do próprio env no arranque, antes de
//! qualquer thread, e todo filho nasce limpo.
//!
//! Lista positiva de propósito: variável de configuração que o usuário
//! exporta (`CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_MAX_OUTPUT_TOKENS`…) passa.

const INHERITED_SESSION_MARKERS: &[&str] = &[
    // Claude Code: identidade e canais da sessão que abriu o TYBA.
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_PID",
    "CLAUDE_PLUGIN_DATA",
    "CLAUDE_PLUGIN_ROOT",
    // Codex: sandbox da sessão de fora.
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    // O próprio TYBA, aberto de um pane de outro TYBA: com `TYBA_HOSTED=1`
    // herdado, todo `claude` digitado cairia no ramo do aninhado.
    "TYBA_HOSTED",
    "TYBA_HOOK_SOCKET",
    "TYBA_JAILED",
    "TYBA_SHIM",
];

pub(crate) fn is_inherited_session_marker(key: &str) -> bool {
    INHERITED_SESSION_MARKERS.contains(&key)
}

/// Tira do env do processo os marcadores de sessão herdados. Chamar no
/// `main`, antes de qualquer thread (`remove_var` não é seguro com outras
/// threads lendo o env).
pub fn scrub_inherited_session_markers() {
    let present: Vec<String> = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| is_inherited_session_marker(k))
        .collect();
    for key in present {
        std::env::remove_var(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_markers_are_scrubbed_and_user_config_passes() {
        for key in [
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "CLAUDECODE",
            "TYBA_HOSTED",
        ] {
            assert!(is_inherited_session_marker(key), "{key}");
        }
        for key in [
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            "ANTHROPIC_API_KEY",
            "TYBA_PANE_TOKEN",
            "PATH",
        ] {
            assert!(!is_inherited_session_marker(key), "{key}");
        }
    }
}
