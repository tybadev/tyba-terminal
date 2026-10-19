use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const SESSION_FLAGS_KEY_SOURCE: &str = "/<session-flags>/config.toml";
const APPROVAL_STATUS_MESSAGE: &str = "Aguardando aprovação no TYBA…";
const APPROVAL_TIMEOUT_SECS: u64 = 86_400;
const SIGNAL_TIMEOUT_SECS: u64 = 60;
/// Teto dos hooks só de observação (status, subagentes, compactação). O
/// `tyba _hook` responde `ack` assim que o core recebe; o teto só importa com
/// o socket fora do ar, e aí o Codex segue sem o hook (fail-open).
const OBSERVE_TIMEOUT_SECS: u64 = 10;
/// `SessionEnd` e `Interrupt` só aceitam de 1 a 3 s (doc do Codex).
const SHORT_TIMEOUT_SECS: u64 = 3;

#[derive(Clone, Copy)]
struct HookSpec {
    event: &'static str,
    /// O nome do evento em snake_case: é o `event_name` da identidade que o
    /// Codex hasheia e o meio da chave em `hooks.state`.
    label: &'static str,
    matcher: Option<&'static str>,
    timeout: u64,
    status_message: Option<&'static str>,
}

const fn observe(event: &'static str, label: &'static str, timeout: u64) -> HookSpec {
    HookSpec {
        event,
        label,
        matcher: None,
        timeout,
        status_message: None,
    }
}

/// `PreToolUse` e `PermissionRequest` são o gate (bloqueantes, timeout de um
/// dia). Todo o resto só observa: sem `statusMessage` (o TUI não anuncia um
/// hook por ferramenta) e com timeout curto.
const SPECS: [HookSpec; 12] = [
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
    observe("SessionStart", "session_start", SIGNAL_TIMEOUT_SECS),
    observe("Stop", "stop", SIGNAL_TIMEOUT_SECS),
    HookSpec {
        event: "PostToolUse",
        label: "post_tool_use",
        matcher: Some("*"),
        timeout: OBSERVE_TIMEOUT_SECS,
        status_message: None,
    },
    observe(
        "UserPromptSubmit",
        "user_prompt_submit",
        OBSERVE_TIMEOUT_SECS,
    ),
    observe("SubagentStart", "subagent_start", OBSERVE_TIMEOUT_SECS),
    observe("SubagentStop", "subagent_stop", OBSERVE_TIMEOUT_SECS),
    observe("PreCompact", "pre_compact", OBSERVE_TIMEOUT_SECS),
    observe("PostCompact", "post_compact", OBSERVE_TIMEOUT_SECS),
    observe("SessionEnd", "session_end", SHORT_TIMEOUT_SECS),
    observe("Interrupt", "interrupt", SHORT_TIMEOUT_SECS),
];

/// Agente livre: com o hook do observador (`_hook --observe`) nada espera o
/// TYBA — todo evento tem teto curto e nenhum anuncia "Aguardando aprovação no
/// TYBA…". O `PreToolUse` só leva o guarda-corpo; a aprovação é do Codex.
fn specs_for(hook_cmd: &str) -> Vec<HookSpec> {
    let observing = hook_cmd.ends_with(crate::hook_ipc::OBSERVE_FLAG);
    SPECS
        .iter()
        .map(|spec| {
            if observing && spec.status_message.is_some() {
                HookSpec {
                    timeout: OBSERVE_TIMEOUT_SECS,
                    status_message: None,
                    ..*spec
                }
            } else {
                *spec
            }
        })
        .collect()
}

pub fn codex_config_overrides(hook_cmd: &str) -> Vec<String> {
    let specs = specs_for(hook_cmd);
    let mut overrides: Vec<String> = specs
        .iter()
        .map(|spec| format!("hooks.{}={}", spec.event, event_groups_toml(spec, hook_cmd)))
        .collect();
    overrides.push(format!(
        "hooks.state={}",
        trust_state_toml(&specs, hook_cmd)
    ));
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

fn trust_state_toml(specs: &[HookSpec], hook_cmd: &str) -> String {
    let entries: Vec<String> = specs
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

pub(crate) fn toml_string(text: &str) -> String {
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

    /// Codex livre: nenhum hook espera o TYBA nem diz que espera.
    #[test]
    fn observer_hooks_never_wait_for_tyba() {
        let overrides = codex_config_overrides("'/usr/bin/tyba' _hook --observe").join("\n");
        assert!(!overrides.contains(APPROVAL_STATUS_MESSAGE));
        assert!(!overrides.contains(&format!("timeout={APPROVAL_TIMEOUT_SECS}")));
        let gated = codex_config_overrides("'/usr/bin/tyba' _hook").join("\n");
        assert!(gated.contains(APPROVAL_STATUS_MESSAGE));
    }

    const CMD: &str = "'/usr/bin/tyba' _hook";

    fn override_value(key: &str) -> String {
        codex_config_overrides(CMD)
            .into_iter()
            .find_map(|o| o.strip_prefix(&format!("{key}=")).map(str::to_string))
            .unwrap_or_else(|| panic!("override {key} ausente"))
    }

    const EVENTS: [(&str, &str); 12] = [
        ("PreToolUse", "pre_tool_use"),
        ("PermissionRequest", "permission_request"),
        ("SessionStart", "session_start"),
        ("Stop", "stop"),
        ("PostToolUse", "post_tool_use"),
        ("UserPromptSubmit", "user_prompt_submit"),
        ("SubagentStart", "subagent_start"),
        ("SubagentStop", "subagent_stop"),
        ("PreCompact", "pre_compact"),
        ("PostCompact", "post_compact"),
        ("SessionEnd", "session_end"),
        ("Interrupt", "interrupt"),
    ];

    fn hook_of(key: &str) -> (toml::Value, toml::Value) {
        let value = override_value(key);
        let doc: toml::Value = toml::from_str(&format!("x={value}")).unwrap();
        let group = doc["x"][0].clone();
        let hook = group["hooks"][0].clone();
        (group, hook)
    }

    #[test]
    fn emits_every_event_and_trust_state() {
        let overrides = codex_config_overrides(CMD);
        assert_eq!(overrides.len(), EVENTS.len() + 1);
        for key in EVENTS
            .iter()
            .map(|(event, _)| format!("hooks.{event}"))
            .chain(["hooks.state".to_string()])
        {
            assert!(
                overrides.iter().any(|o| o.starts_with(&format!("{key}="))),
                "faltou {key}"
            );
        }
    }

    #[test]
    fn labels_are_the_snake_case_of_the_event() {
        for (spec, (event, label)) in SPECS.iter().zip(EVENTS) {
            assert_eq!(spec.event, event);
            assert_eq!(spec.label, label);
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
            let (group, hook) = hook_of(key);
            assert!(
                group.get("matcher").is_none(),
                "{key} não devia ter matcher"
            );
            assert_eq!(hook["timeout"].as_integer(), Some(60), "{key}");
            assert!(hook.get("statusMessage").is_none(), "{key}");
        }
    }

    /// Observação nunca segura o Codex: timeout curto, sem `statusMessage` e
    /// síncrono (assíncrono entregaria `PostToolUse` depois do `Stop` e
    /// deixaria a sessão presa em `Running`).
    #[test]
    fn observation_hooks_are_short_silent_and_synchronous() {
        for key in [
            "hooks.PostToolUse",
            "hooks.UserPromptSubmit",
            "hooks.SubagentStart",
            "hooks.SubagentStop",
            "hooks.PreCompact",
            "hooks.PostCompact",
        ] {
            let (_, hook) = hook_of(key);
            assert_eq!(hook["type"].as_str(), Some("command"), "{key}");
            assert_eq!(hook["command"].as_str(), Some(CMD), "{key}");
            assert_eq!(hook["timeout"].as_integer(), Some(10), "{key}");
            assert!(hook.get("statusMessage").is_none(), "{key}");
            assert!(hook.get("async").is_none(), "{key}");
        }
    }

    /// O Codex limita `SessionEnd` e `Interrupt` a 1–3 s.
    #[test]
    fn session_end_and_interrupt_stay_within_three_seconds() {
        for key in ["hooks.SessionEnd", "hooks.Interrupt"] {
            let (group, hook) = hook_of(key);
            assert!(group.get("matcher").is_none(), "{key}");
            assert_eq!(hook["timeout"].as_integer(), Some(3), "{key}");
        }
    }

    #[test]
    fn matchers_only_where_the_tool_name_filters() {
        let (group, _) = hook_of("hooks.PostToolUse");
        assert_eq!(group["matcher"].as_str(), Some("*"));
        for key in [
            "hooks.PermissionRequest",
            "hooks.UserPromptSubmit",
            "hooks.SubagentStart",
            "hooks.SubagentStop",
            "hooks.PreCompact",
            "hooks.PostCompact",
        ] {
            let (group, _) = hook_of(key);
            assert!(group.get("matcher").is_none(), "{key}");
        }
    }

    #[test]
    fn trust_state_has_session_flags_keys_for_all_events() {
        let value = override_value("hooks.state");
        let doc: toml::Value = toml::from_str(&format!("x={value}")).unwrap();
        let state = doc["x"].as_table().unwrap();
        assert_eq!(state.len(), EVENTS.len());
        for (_, label) in EVENTS {
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
    fn identity_json_of_observation_hooks() {
        let post = SPECS.iter().find(|s| s.label == "post_tool_use").unwrap();
        assert_eq!(
            String::from_utf8(identity_json(post, "x _hook")).unwrap(),
            r#"{"event_name":"post_tool_use","hooks":[{"async":false,"command":"x _hook","timeout":10,"type":"command"}],"matcher":"*"}"#
        );
        let end = SPECS.iter().find(|s| s.label == "session_end").unwrap();
        assert_eq!(
            String::from_utf8(identity_json(end, "x _hook")).unwrap(),
            r#"{"event_name":"session_end","hooks":[{"async":false,"command":"x _hook","timeout":3,"type":"command"}]}"#
        );
    }

    #[test]
    fn trusted_hash_differs_per_event() {
        let mut hashes: Vec<String> = SPECS.iter().map(|s| trusted_hash(s, CMD)).collect();
        hashes.sort();
        hashes.dedup();
        assert_eq!(hashes.len(), SPECS.len());
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
