use crate::session::{AwaitingReason, SessionStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSignal {
    Ready,
    Working,
    TurnEnded,
    AwaitingInput,
    Ended,
    SubagentStarted,
    SubagentEnded,
}

pub fn signal_for(hook_event_name: &str, notification_type: Option<&str>) -> Option<AgentSignal> {
    match hook_event_name {
        "SessionStart" => Some(AgentSignal::Ready),
        "PreToolUse" => Some(AgentSignal::Working),
        "Stop" => Some(AgentSignal::TurnEnded),
        "SessionEnd" => Some(AgentSignal::Ended),
        "SubagentStart" => Some(AgentSignal::SubagentStarted),
        "SubagentStop" => Some(AgentSignal::SubagentEnded),
        "Notification" => match notification_type {
            Some("idle_prompt") => Some(AgentSignal::AwaitingInput),
            _ => None,
        },
        _ => None,
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
        AgentSignal::Ended => None,
        AgentSignal::SubagentStarted => None,
        AgentSignal::SubagentEnded => None,
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
    fn status_subagent_signals_never_change_session_status() {
        assert!(status_for(&AgentSignal::SubagentStarted).is_none());
        assert!(status_for(&AgentSignal::SubagentEnded).is_none());
    }
}
