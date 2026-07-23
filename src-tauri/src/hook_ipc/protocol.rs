use serde::{Deserialize, Serialize};

use super::HookEvent;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub v: u32,
    pub event: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub v: u32,
    pub action: String,
    pub reason: Option<String>,
}

pub fn hook_event_from_value(event: serde_json::Value) -> HookEvent {
    let str_field = |key: &str| {
        event
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    HookEvent {
        hook_event_name: str_field("hook_event_name").unwrap_or_default(),
        tool_name: str_field("tool_name"),
        tool_input: event.get("tool_input").cloned(),
        notification_type: str_field("notification_type"),
        cwd: str_field("cwd"),
        agent_id: str_field("agent_id"),
        agent_type: str_field("agent_type"),
        raw: event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_agent_id_and_agent_type_from_subagent_start() {
        let event = hook_event_from_value(json!({
            "hook_event_name": "SubagentStart",
            "agent_id": "a0123456789abcdef",
            "agent_type": "code-reviewer",
            "transcript_path": "/p/session.jsonl"
        }));
        assert_eq!(event.hook_event_name, "SubagentStart");
        assert_eq!(event.agent_id.as_deref(), Some("a0123456789abcdef"));
        assert_eq!(event.agent_type.as_deref(), Some("code-reviewer"));
        assert_eq!(event.raw["transcript_path"], "/p/session.jsonl");
    }

    #[test]
    fn agent_fields_absent_when_missing() {
        let event = hook_event_from_value(json!({ "hook_event_name": "PreToolUse" }));
        assert!(event.agent_id.is_none());
        assert!(event.agent_type.is_none());
    }

    #[test]
    fn non_string_agent_fields_are_ignored() {
        let event = hook_event_from_value(json!({
            "hook_event_name": "SubagentStop",
            "agent_id": 42,
            "agent_type": ["x"]
        }));
        assert!(event.agent_id.is_none());
        assert!(event.agent_type.is_none());
    }
}
