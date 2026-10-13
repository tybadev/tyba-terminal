use crate::session::{AwaitingReason, SessionStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSignal {
    Ready,
    Working,
    TurnEnded,
    /// O turno acabou por erro da API (`StopFailure`, que roda no lugar do
    /// `Stop`). O handler monta o status com o motivo.
    TurnFailed,
    AwaitingInput,
    Ended,
    SubagentStarted,
    SubagentEnded,
    /// `PreCompact`: o agente vai compactar o contexto. Não muda o status — só
    /// a nota da sessão ([`COMPACTING_NOTE`]).
    Compacting,
    /// `Interrupt` (Codex): o usuário interrompeu o turno. O Codex não manda
    /// `Stop` nesse caso, então sem isto a sessão ficaria `Running`.
    Interrupted,
}

/// A nota da sessão enquanto o agente compacta o contexto.
pub const COMPACTING_NOTE: &str = "compactando contexto";

/// O resumo do turno que o usuário interrompeu.
pub const INTERRUPTED_SUMMARY: &str = "Turno interrompido";

pub fn signal_for(hook_event_name: &str, notification_type: Option<&str>) -> Option<AgentSignal> {
    match hook_event_name {
        "SessionStart" => Some(AgentSignal::Ready),
        // `PostToolUse` e `UserPromptSubmit` são o que tira a sessão do prompt
        // nativo do Claude: a ferramenta rodou (aprovada) ou o usuário mandou a
        // próxima mensagem. Negar no prompt nativo não dispara hook nenhum —
        // nem `Stop` (medido no B0) —, então o `UserPromptSubmit` seguinte é a
        // saída desse caminho.
        //
        // `PermissionRequest` fica de fora de propósito: ele chega ~0,1 s depois
        // do `PreToolUse` da mesma chamada e é ele quem **põe** a sessão em
        // `NativePrompt` (no roteamento do handler). Se virasse `Working` aqui,
        // o sinal rodaria antes do roteamento e disputaria o status.
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "UserPromptSubmit" => {
            Some(AgentSignal::Working)
        }
        "Stop" => Some(AgentSignal::TurnEnded),
        "StopFailure" => Some(AgentSignal::TurnFailed),
        "SessionEnd" => Some(AgentSignal::Ended),
        "SubagentStart" => Some(AgentSignal::SubagentStarted),
        "SubagentStop" => Some(AgentSignal::SubagentEnded),
        "PreCompact" => Some(AgentSignal::Compacting),
        "Interrupt" => Some(AgentSignal::Interrupted),
        "Notification" => match notification_type {
            Some("idle_prompt") => Some(AgentSignal::AwaitingInput),
            _ => None,
        },
        _ => None,
    }
}

/// O status depois de um `StopFailure`: o processo segue vivo e o turno
/// acabou, então é `Idle` — não `Failed`, que para o resto do app é sessão
/// morta (convite de retomar, update que não ressuscita). A falha vai no
/// resumo, que é o que o painel e a lateral mostram.
pub fn status_on_stop_failure(reason: &str) -> SessionStatus {
    SessionStatus::Idle {
        summary: Some(format!("Falhou: {reason}")),
    }
}

/// O status de quem está esperando no prompt nativo do Claude.
pub fn native_prompt_status(hint: Option<String>) -> SessionStatus {
    SessionStatus::AwaitingInput {
        hint,
        reason: AwaitingReason::NativePrompt,
    }
}

/// O que o `SessionEnd` faz com o status atual.
///
/// Só mexe no prompt nativo: o agente que encerra a sessão (`/clear`, saída)
/// não está mais esperando aquela permissão, e sem isto a sessão ficaria
/// "precisa de você" por uma pergunta que não existe mais. Os outros estados
/// seguem como hoje — o `Exited` do processo chega por outro caminho.
pub fn status_on_session_end(current: &SessionStatus) -> Option<SessionStatus> {
    match current {
        SessionStatus::AwaitingInput {
            reason: AwaitingReason::NativePrompt,
            ..
        } => Some(SessionStatus::Idle { summary: None }),
        _ => None,
    }
}

/// O que o `Notification` `idle_prompt` faz com o status atual.
///
/// Normalmente vira `AwaitingInput{Reply}`. Mas o Claude também manda
/// `idle_prompt` com o prompt nativo de permissão aberto, e rebaixar a sessão
/// para "esperando resposta" esconderia que ela está bloqueada numa pergunta de
/// permissão — o motivo que a faixa "precisa de você" e o selo usam. O prompt
/// nativo só sai pelos eventos da tabela (ver `signal_for`).
pub fn status_on_idle_prompt(current: &SessionStatus) -> Option<SessionStatus> {
    match current {
        SessionStatus::AwaitingInput {
            reason: AwaitingReason::NativePrompt,
            ..
        } => None,
        _ => status_for(&AgentSignal::AwaitingInput),
    }
}

/// O que o `Interrupt` faz com o status atual: o turno que rodava acaba.
///
/// Só sai de `Running`. Esperando aprovação ou resposta, quem decide a saída é
/// a própria pergunta; e uma sessão já parada não volta a ser "interrompida".
pub fn status_on_interrupt(current: &SessionStatus) -> Option<SessionStatus> {
    match current {
        SessionStatus::Running => Some(SessionStatus::Idle {
            summary: Some(INTERRUPTED_SUMMARY.into()),
        }),
        _ => None,
    }
}

/// A nota que um evento deixa na sessão: `Some(Some)` só no `PreCompact`.
/// Qualquer outro evento do fio principal apaga — o `PostCompact`, ou o que
/// vier depois de uma compactação cujo fim não chegou. `None` = não mexe:
/// evento de subagente não fala pelo orquestrador.
pub fn note_for_event(
    signal: Option<&AgentSignal>,
    from_subagent: bool,
) -> Option<Option<&'static str>> {
    match signal {
        Some(AgentSignal::Compacting) => Some(Some(COMPACTING_NOTE)),
        _ if from_subagent => None,
        _ => Some(None),
    }
}

pub fn status_for(signal: &AgentSignal) -> Option<SessionStatus> {
    match signal {
        AgentSignal::Working => Some(SessionStatus::Running),
        AgentSignal::TurnEnded => Some(SessionStatus::Idle { summary: None }),
        AgentSignal::AwaitingInput => Some(SessionStatus::AwaitingInput {
            hint: None,
            reason: AwaitingReason::Reply,
        }),
        AgentSignal::Ready => None,
        AgentSignal::TurnFailed => None,
        AgentSignal::Ended => None,
        AgentSignal::SubagentStarted => None,
        AgentSignal::SubagentEnded => None,
        AgentSignal::Compacting => None,
        AgentSignal::Interrupted => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_session_start_is_ready() {
        assert_eq!(signal_for("SessionStart", None), Some(AgentSignal::Ready));
    }

    #[test]
    fn signal_pretooluse_is_working() {
        assert_eq!(signal_for("PreToolUse", None), Some(AgentSignal::Working));
    }

    #[test]
    fn post_tool_use_e_user_prompt_submit_voltam_a_trabalhar() {
        assert_eq!(signal_for("PostToolUse", None), Some(AgentSignal::Working));
        assert_eq!(
            signal_for("PostToolUseFailure", None),
            Some(AgentSignal::Working)
        );
        assert_eq!(
            signal_for("UserPromptSubmit", None),
            Some(AgentSignal::Working)
        );
    }

    /// O `PermissionRequest` não pode virar sinal: o sinal roda antes do
    /// roteamento, e um `Working` aqui brigaria com o `NativePrompt` que o
    /// próprio `PermissionRequest` põe.
    #[test]
    fn permission_request_nao_e_sinal_de_status() {
        assert_eq!(signal_for("PermissionRequest", None), None);
    }

    #[test]
    fn prompt_nativo_e_esperando_com_motivo_proprio() {
        assert_eq!(
            native_prompt_status(Some("cargo test".into())),
            SessionStatus::AwaitingInput {
                hint: Some("cargo test".into()),
                reason: AwaitingReason::NativePrompt,
            }
        );
    }

    #[test]
    fn session_end_tira_do_prompt_nativo() {
        assert_eq!(
            status_on_session_end(&native_prompt_status(None)),
            Some(SessionStatus::Idle { summary: None })
        );
    }

    #[test]
    fn session_end_nao_mexe_nos_outros_estados() {
        for current in [
            SessionStatus::Running,
            SessionStatus::Idle { summary: None },
            SessionStatus::AwaitingInput {
                hint: None,
                reason: AwaitingReason::Approval,
            },
            SessionStatus::AwaitingInput {
                hint: None,
                reason: AwaitingReason::Reply,
            },
            SessionStatus::Exited { code: 0 },
        ] {
            assert_eq!(status_on_session_end(&current), None, "{current:?}");
        }
    }

    /// Desvio pedido pelo orquestrador no B5: o `idle_prompt` não pode tirar a
    /// sessão do prompt nativo.
    #[test]
    fn idle_prompt_nao_rebaixa_o_prompt_nativo_para_reply() {
        assert_eq!(
            status_on_idle_prompt(&native_prompt_status(Some("cargo test".into()))),
            None
        );
    }

    #[test]
    fn idle_prompt_ainda_vira_reply_nos_outros_estados() {
        for current in [
            SessionStatus::Running,
            SessionStatus::Idle { summary: None },
        ] {
            assert_eq!(
                status_on_idle_prompt(&current),
                Some(SessionStatus::AwaitingInput {
                    hint: None,
                    reason: AwaitingReason::Reply,
                }),
                "{current:?}"
            );
        }
    }

    #[test]
    fn stop_failure_vira_turno_com_falha_e_idle_com_motivo() {
        assert_eq!(
            signal_for("StopFailure", None),
            Some(AgentSignal::TurnFailed)
        );
        assert_eq!(status_for(&AgentSignal::TurnFailed), None);
        assert_eq!(
            status_on_stop_failure("limite de uso atingido"),
            SessionStatus::Idle {
                summary: Some("Falhou: limite de uso atingido".into())
            }
        );
    }

    #[test]
    fn signal_stop_is_turn_ended() {
        assert_eq!(signal_for("Stop", None), Some(AgentSignal::TurnEnded));
    }

    #[test]
    fn signal_session_end_is_ended() {
        assert_eq!(signal_for("SessionEnd", None), Some(AgentSignal::Ended));
    }

    #[test]
    fn signal_subagent_start_is_subagent_started() {
        assert_eq!(
            signal_for("SubagentStart", None),
            Some(AgentSignal::SubagentStarted)
        );
    }

    #[test]
    fn signal_subagent_stop_is_subagent_ended() {
        assert_eq!(
            signal_for("SubagentStop", None),
            Some(AgentSignal::SubagentEnded)
        );
    }

    #[test]
    fn signal_notification_idle_prompt_is_awaiting_input() {
        assert_eq!(
            signal_for("Notification", Some("idle_prompt")),
            Some(AgentSignal::AwaitingInput)
        );
    }

    #[test]
    fn signal_notification_other_type_is_none() {
        assert_eq!(signal_for("Notification", Some("something_else")), None);
    }

    #[test]
    fn signal_notification_without_type_is_none() {
        assert_eq!(signal_for("Notification", None), None);
    }

    #[test]
    fn signal_unknown_event_is_none() {
        assert_eq!(signal_for("WhateverElse", None), None);
    }

    #[test]
    fn signal_is_case_sensitive() {
        assert_eq!(signal_for("sessionstart", None), None);
        assert_eq!(signal_for("pretooluse", None), None);
        assert_eq!(signal_for("STOP", None), None);
    }

    #[test]
    fn status_working_is_running() {
        assert!(matches!(
            status_for(&AgentSignal::Working),
            Some(SessionStatus::Running)
        ));
    }

    #[test]
    fn status_turn_ended_is_idle_without_summary() {
        assert!(matches!(
            status_for(&AgentSignal::TurnEnded),
            Some(SessionStatus::Idle { summary: None })
        ));
    }

    #[test]
    fn status_awaiting_input_is_reply_without_hint() {
        assert!(matches!(
            status_for(&AgentSignal::AwaitingInput),
            Some(SessionStatus::AwaitingInput {
                hint: None,
                reason: AwaitingReason::Reply
            })
        ));
    }

    #[test]
    fn status_ready_is_none() {
        assert!(status_for(&AgentSignal::Ready).is_none());
    }

    #[test]
    fn status_ended_is_none() {
        assert!(status_for(&AgentSignal::Ended).is_none());
    }

    #[test]
    fn pre_compact_e_interrupt_viram_sinal() {
        assert_eq!(
            signal_for("PreCompact", None),
            Some(AgentSignal::Compacting)
        );
        assert_eq!(
            signal_for("Interrupt", None),
            Some(AgentSignal::Interrupted)
        );
        assert_eq!(signal_for("PostCompact", None), None);
    }

    #[test]
    fn compactar_e_interromper_nao_passam_pelo_status_generico() {
        assert!(status_for(&AgentSignal::Compacting).is_none());
        assert!(status_for(&AgentSignal::Interrupted).is_none());
    }

    #[test]
    fn interrupt_fecha_o_turno_que_rodava() {
        assert_eq!(
            status_on_interrupt(&SessionStatus::Running),
            Some(SessionStatus::Idle {
                summary: Some("Turno interrompido".into())
            })
        );
    }

    #[test]
    fn interrupt_nao_mexe_em_quem_nao_rodava() {
        for current in [
            SessionStatus::Idle { summary: None },
            SessionStatus::AwaitingInput {
                hint: None,
                reason: AwaitingReason::Approval,
            },
            SessionStatus::Exited { code: 0 },
        ] {
            assert_eq!(status_on_interrupt(&current), None, "{current:?}");
        }
    }

    #[test]
    fn nota_de_compactacao_vive_do_pre_ao_proximo_evento_principal() {
        let pre = signal_for("PreCompact", None);
        assert_eq!(
            note_for_event(pre.as_ref(), false),
            Some(Some("compactando contexto"))
        );
        for name in ["PostCompact", "PostToolUse", "Stop", "SessionStart"] {
            let signal = signal_for(name, None);
            assert_eq!(note_for_event(signal.as_ref(), false), Some(None), "{name}");
        }
    }

    #[test]
    fn evento_de_subagente_nao_apaga_a_nota_do_orquestrador() {
        let post = signal_for("PostToolUse", None);
        assert_eq!(note_for_event(post.as_ref(), true), None);
        let stop = signal_for("SubagentStop", None);
        assert_eq!(note_for_event(stop.as_ref(), true), None);
    }

    #[test]
    fn status_subagent_signals_never_change_session_status() {
        assert!(status_for(&AgentSignal::SubagentStarted).is_none());
        assert!(status_for(&AgentSignal::SubagentEnded).is_none());
    }
}
