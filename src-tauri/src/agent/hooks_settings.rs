use std::path::Path;

use serde_json::{json, Value};

pub fn hook_command(exe: &Path) -> String {
    let raw = exe.to_string_lossy();
    let quoted = format!("'{}'", raw.replace('\'', "'\\''"));
    format!("{quoted} _hook")
}

pub fn hooks_settings_json(hook_cmd: &str) -> Value {
    json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 86400,
                    "statusMessage": "Aguardando aprovação no TYBA…"
                }]
            }],
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "Stop": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "SessionEnd": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "Notification": [{
                "matcher": "idle_prompt",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "SubagentStart": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "SubagentStop": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_command_quotes_plain_path() {
        assert_eq!(
            hook_command(Path::new("/usr/bin/tyba")),
            "'/usr/bin/tyba' _hook"
        );
    }

    #[test]
    fn hook_command_quotes_path_with_space() {
        assert_eq!(
            hook_command(Path::new("/Apps/My App/tyba")),
            "'/Apps/My App/tyba' _hook"
        );
    }

    #[test]
    fn hook_command_escapes_embedded_single_quote() {
        assert_eq!(
            hook_command(Path::new("/o'brien/tyba")),
            "'/o'\\''brien/tyba' _hook"
        );
    }

    #[test]
    fn root_has_only_hooks_key() {
        let v = hooks_settings_json("x _hook");
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("hooks"));
    }

    #[test]
    fn all_seven_events_present() {
        let v = hooks_settings_json("x _hook");
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), 7);
        for name in [
            "PreToolUse",
            "SessionStart",
            "Stop",
            "SessionEnd",
            "Notification",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert!(hooks.contains_key(name), "missing {name}");
        }
    }

    #[test]
    fn pretooluse_shape() {
        let cmd = "x _hook";
        let v = hooks_settings_json(cmd);
        let entry = &v["hooks"]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "*");
        let hook = &entry["hooks"][0];
        assert_eq!(hook["type"], "command");
        assert_eq!(hook["command"], cmd);
        assert_eq!(hook["timeout"], 86400);
        assert_eq!(hook["statusMessage"], "Aguardando aprovação no TYBA…");
    }

    #[test]
    fn timeout_86400_only_on_pretooluse() {
        let v = hooks_settings_json("x _hook");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 86400);
        for name in [
            "SessionStart",
            "Stop",
            "SessionEnd",
            "Notification",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert_eq!(
                v["hooks"][name][0]["hooks"][0]["timeout"], 60,
                "wrong timeout on {name}"
            );
        }
    }

    #[test]
    fn matchers_are_correct() {
        let v = hooks_settings_json("x _hook");
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "*");
        assert_eq!(v["hooks"]["Notification"][0]["matcher"], "idle_prompt");
        for name in [
            "SessionStart",
            "Stop",
            "SessionEnd",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert!(
                v["hooks"][name][0].get("matcher").is_none(),
                "{name} should have no matcher"
            );
        }
    }

    #[test]
    fn matcherless_events_shape() {
        let cmd = "x _hook";
        let v = hooks_settings_json(cmd);
        for name in [
            "SessionStart",
            "Stop",
            "SessionEnd",
            "SubagentStart",
            "SubagentStop",
        ] {
            let hook = &v["hooks"][name][0]["hooks"][0];
            assert_eq!(hook["type"], "command", "{name}");
            assert_eq!(hook["command"], cmd, "{name}");
            assert_eq!(hook["timeout"], 60, "{name}");
        }
    }

    #[test]
    fn notification_shape() {
        let cmd = "x _hook";
        let v = hooks_settings_json(cmd);
        let entry = &v["hooks"]["Notification"][0];
        assert_eq!(entry["matcher"], "idle_prompt");
        let hook = &entry["hooks"][0];
        assert_eq!(hook["type"], "command");
        assert_eq!(hook["command"], cmd);
        assert_eq!(hook["timeout"], 60);
    }

    #[test]
    fn status_message_only_on_pretooluse() {
        let v = hooks_settings_json("x _hook");
        for name in [
            "SessionStart",
            "Stop",
            "SessionEnd",
            "Notification",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert!(
                v["hooks"][name][0]["hooks"][0]
                    .get("statusMessage")
                    .is_none(),
                "{name} should not carry statusMessage"
            );
        }
    }
}
