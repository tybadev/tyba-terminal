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
        raw: event,
    }
}
