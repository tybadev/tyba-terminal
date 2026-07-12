use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const SESSION_FLAGS_KEY_SOURCE: &str = "/<session-flags>/config.toml";
const APPROVAL_STATUS_MESSAGE: &str = "Aguardando aprovação no TYBA…";
const APPROVAL_TIMEOUT_SECS: u64 = 86_400;
const SIGNAL_TIMEOUT_SECS: u64 = 60;

struct HookSpec {
    event: &'static str,
    label: &'static str,
    matcher: Option<&'static str>,
    timeout: u64,
    status_message: Option<&'static str>,
}

const SPECS: [HookSpec; 4] = [
    HookSpec {
        event: "PreToolUse",
        label: "pre_tool_use",
        matcher: Some("*"),
        timeout: APPROVAL_TIMEOUT_SECS,
        status_message: Some(APPROVAL_STATUS_MESSAGE),
    },
    HookSpec {
        event: "PermissionRequest",
        label: "permission_request",
        matcher: None,
        timeout: APPROVAL_TIMEOUT_SECS,
        status_message: Some(APPROVAL_STATUS_MESSAGE),
    },
    HookSpec {
        event: "SessionStart",
        label: "session_start",
        matcher: None,
        timeout: SIGNAL_TIMEOUT_SECS,
        status_message: None,
    },
    HookSpec {
        event: "Stop",
        label: "stop",
        matcher: None,
        timeout: SIGNAL_TIMEOUT_SECS,
        status_message: None,
    },
];

pub fn codex_config_overrides(hook_cmd: &str) -> Vec<String> {
    let mut overrides: Vec<String> = SPECS
        .iter()
        .map(|spec| format!("hooks.{}={}", spec.event, event_groups_toml(spec, hook_cmd)))
        .collect();
    overrides.push(format!("hooks.state={}", trust_state_toml(hook_cmd)));
    overrides
}

fn event_groups_toml(spec: &HookSpec, hook_cmd: &str) -> String {
    let mut group = String::from("{");
    if let Some(matcher) = spec.matcher {
        group.push_str(&format!("matcher={},", toml_string(matcher)));
    }
    group.push_str(&format!(
        "hooks=[{{type=\"command\",command={},timeout={}",
        toml_string(hook_cmd),
        spec.timeout
    ));
    if let Some(message) = spec.status_message {
        group.push_str(&format!(",statusMessage={}", toml_string(message)));
    }
    group.push_str("}]}");
    format!("[{group}]")
}

fn trust_state_toml(hook_cmd: &str) -> String {
    let entries: Vec<String> = SPECS
        .iter()
        .map(|spec| {
            format!(
                "{}={{trusted_hash={}}}",
                toml_string(&hook_state_key(spec.label)),
                toml_string(&trusted_hash(spec, hook_cmd))
            )
        })
        .collect();
    format!("{{{}}}", entries.join(","))
}

fn hook_state_key(label: &str) -> String {
    format!("{SESSION_FLAGS_KEY_SOURCE}:{label}:0:0")
}

fn identity_json(spec: &HookSpec, hook_cmd: &str) -> Vec<u8> {
    let mut handler = Map::new();
    handler.insert("async".into(), Value::Bool(false));
    handler.insert("command".into(), Value::String(hook_cmd.into()));
    if let Some(message) = spec.status_message {
        handler.insert("statusMessage".into(), Value::String(message.into()));
    }
    handler.insert("timeout".into(), Value::from(spec.timeout));
    handler.insert("type".into(), Value::String("command".into()));

    let mut identity = Map::new();
    identity.insert("event_name".into(), Value::String(spec.label.into()));
    identity.insert("hooks".into(), Value::Array(vec![Value::Object(handler)]));
    if let Some(matcher) = spec.matcher {
        identity.insert("matcher".into(), Value::String(matcher.into()));
    }
    serde_json::to_vec(&Value::Object(identity)).unwrap_or_default()
}

fn trusted_hash(spec: &HookSpec, hook_cmd: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(identity_json(spec, hook_cmd));
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "'/usr/bin/tyba' _hook";

    fn override_value(key: &str) -> String {
        codex_config_overrides(CMD)
            .into_iter()
            .find_map(|o| o.strip_prefix(&format!("{key}=")).map(str::to_string))
            .unwrap_or_else(|| panic!("override {key} ausente"))
    }

    #[test]
    fn emits_four_events_and_trust_state() {
        let overrides = codex_config_overrides(CMD);
        assert_eq!(overrides.len(), 5);
        for key in [
            "hooks.PreToolUse",
            "hooks.PermissionRequest",
            "hooks.SessionStart",
            "hooks.Stop",
            "hooks.state",
        ] {
            assert!(
                overrides.iter().any(|o| o.starts_with(&format!("{key}="))),
                "faltou {key}"
            );
        }
    }

    #[test]
    fn every_override_value_parses_as_toml() {
        for over in codex_config_overrides(CMD) {
            let (key, value) = over.split_once('=').unwrap();
            let doc = format!("x={value}");
            toml::from_str::<toml::Value>(&doc)
                .unwrap_or_else(|e| panic!("valor de {key} não é TOML: {e}"));
        }
    }

    #[test]
    fn pre_tool_use_group_has_matcher_timeout_and_status() {
        let value = override_value("hooks.PreToolUse");
        let doc: toml::Value = toml::from_str(&format!("x={value}")).unwrap();
        let group = &doc["x"][0];
        assert_eq!(group["matcher"].as_str(), Some("*"));
        let hook = &group["hooks"][0];
        assert_eq!(hook["type"].as_str(), Some("command"));
        assert_eq!(hook["command"].as_str(), Some(CMD));
        assert_eq!(hook["timeout"].as_integer(), Some(86_400));
        assert_eq!(
            hook["statusMessage"].as_str(),
            Some("Aguardando aprovação no TYBA…")
        );
    }

    #[test]
    fn signal_hooks_have_short_timeout_and_no_matcher() {
        for key in ["hooks.SessionStart", "hooks.Stop"] {
            let value = override_value(key);
            let doc: toml::Value = toml::from_str(&format!("x={value}")).unwrap();
            let group = &doc["x"][0];
            assert!(
                group.get("matcher").is_none(),
                "{key} não devia ter matcher"
            );
            let hook = &group["hooks"][0];
            assert_eq!(hook["timeout"].as_integer(), Some(60), "{key}");
            assert!(hook.get("statusMessage").is_none(), "{key}");
        }
    }

    #[test]
    fn trust_state_has_session_flags_keys_for_all_events() {
        let value = override_value("hooks.state");
        let doc: toml::Value = toml::from_str(&format!("x={value}")).unwrap();
        let state = doc["x"].as_table().unwrap();
        assert_eq!(state.len(), 4);
        for label in [
            "pre_tool_use",
            "permission_request",
            "session_start",
            "stop",
        ] {
            let key = format!("/<session-flags>/config.toml:{label}:0:0");
            let entry = state
                .get(&key)
                .unwrap_or_else(|| panic!("faltou state de {label}"));
            let hash = entry["trusted_hash"].as_str().unwrap();
            assert!(hash.starts_with("sha256:"), "{label}: {hash}");
            assert_eq!(hash.len(), "sha256:".len() + 64, "{label}");
        }
    }

    #[test]
    fn identity_json_is_canonical_sorted_compact() {
        let spec = &SPECS[2];
        let json = String::from_utf8(identity_json(spec, "x _hook")).unwrap();
        assert_eq!(
            json,
            r#"{"event_name":"session_start","hooks":[{"async":false,"command":"x _hook","timeout":60,"type":"command"}]}"#
        );
        let spec = &SPECS[0];
        let json = String::from_utf8(identity_json(spec, "x _hook")).unwrap();
        assert_eq!(
            json,
            r#"{"event_name":"pre_tool_use","hooks":[{"async":false,"command":"x _hook","statusMessage":"Aguardando aprovação no TYBA…","timeout":86400,"type":"command"}],"matcher":"*"}"#
        );
    }

    #[test]
    fn trusted_hash_changes_with_command() {
        let a = trusted_hash(&SPECS[0], "a _hook");
        let b = trusted_hash(&SPECS[0], "b _hook");
        assert_ne!(a, b);
    }

    #[test]
    fn toml_string_escapes_specials() {
        assert_eq!(toml_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(toml_string("linha\nnova"), r#""linha\nnova""#);
        assert_eq!(
            toml_string("caminho com 'aspas'"),
            r#""caminho com 'aspas'""#
        );
    }
}
