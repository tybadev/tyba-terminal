use std::path::Path;

use serde_json::{json, Value};

pub fn hook_command(exe: &Path) -> String {
    let raw = exe.to_string_lossy();
    let quoted = format!("'{}'", raw.replace('\'', "'\\''"));
    format!("{quoted} _hook")
}

/// Texto que o Claude mostra enquanto o hook do `PreToolUse` roda. Neutro: no
/// fluxo híbrido o amarelo do Claude não espera o TYBA (sai sem decisão, e quem
/// pergunta é o prompt nativo), então "aguardando aprovação" mentiria na
/// maioria das chamadas.
pub const PRE_TOOL_USE_STATUS_MESSAGE: &str = "Conferindo no TYBA…";

pub fn hooks_settings_json(hook_cmd: &str) -> Value {
    json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 86400,
                    "statusMessage": PRE_TOOL_USE_STATUS_MESSAGE
                }]
            }],
            // Só sinal: o prompt nativo abriu. O core responde `ack` (sem saída)
            // e nunca decide aqui — `allow` aprovaria em nome do usuário.
            "PermissionRequest": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "PostToolUse": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            // Ferramenta aprovada que FALHA (exit != 0, erro, interrupção)
            // dispara este, não o `PostToolUse`. Sem ele a linha deduzida
            // fecharia como negada um comando que o usuário aprovou.
            "PostToolUseFailure": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }],
            "UserPromptSubmit": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
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
            }],
            // Roda NO LUGAR do `Stop` quando o turno acaba por erro da API
            // (rate_limit, authentication_failed, billing_error, overloaded…).
            // Sem matcher de propósito: qualquer tipo encerra o turno, e sem
            // este evento a sessão ficaria "rodando" para sempre. O core
            // traduz o `error` (ver `agent::usage::stop_failure_reason`).
            "StopFailure": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 60
                }]
            }]
        }
    })
}

/// Primeira versão do Claude Code que entende `onFailure: "block"` num hook.
const ON_FAILURE_BLOCK_SINCE: (u64, u64, u64) = (2, 1, 295);

/// Se a saída de `claude --version` (`"2.1.296 (Claude Code)"`) é de uma
/// versão que entende `onFailure: "block"`. Ilegível é `false`: numa versão
/// que não conhece a chave, mandar o campo é aposta, e o gate de hoje
/// continua valendo sem ele.
pub fn supports_on_failure_block(version_output: &str) -> bool {
    let Some(token) = version_output.split_whitespace().next() else {
        return false;
    };
    let mut parts = token.split('.').map(|p| p.parse::<u64>().ok());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Some(a)), Some(Some(b)), Some(Some(c))) => (a, b, c) >= ON_FAILURE_BLOCK_SINCE,
        _ => false,
    }
}

/// M6-A: dentro do TYBA o gate falha FECHADO. Com `onFailure: "block"` no
/// `PreToolUse`, um hook que não sobe ou não responde (socket sumiu, `tyba`
/// fora do lugar) bloqueia a ferramenta em vez de liberá-la. Só no
/// `PreToolUse`, o evento que decide; os outros hooks são aviso, e travar um
/// `Stop` por falha não protege nada. Nunca vai para config global do Claude
/// — só para o `--settings` da sessão que o TYBA abriu.
pub fn block_pre_tool_use_on_failure(mut settings: Value) -> Value {
    if let Some(entries) = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut)
    {
        for entry in entries {
            if let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                for hook in hooks {
                    if let Some(hook) = hook.as_object_mut() {
                        hook.insert("onFailure".into(), json!("block"));
                    }
                }
            }
        }
    }
    settings
}

/// O `--settings` de uma sessão do Claude aberta pelo TYBA: os hooks, o
/// `statusLine` do TYBA (ver `agent::statusline`, que devolve a statusline do
/// próprio usuário), e o `onFailure: "block"` no `PreToolUse` quando o
/// `claude` instalado entende a chave. A versão é perguntada uma vez por
/// processo (`claude --version`).
pub fn claude_session_settings(hook_cmd: &str, status_line: Value) -> Value {
    static BLOCKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let blocks = *BLOCKS.get_or_init(|| {
        crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
            .and_then(|bin| {
                std::process::Command::new(bin)
                    .arg("--version")
                    .output()
                    .ok()
            })
            .filter(|out| out.status.success())
            .is_some_and(|out| supports_on_failure_block(&String::from_utf8_lossy(&out.stdout)))
    });
    compose_session_settings(hook_cmd, blocks, status_line)
}

/// A montagem, sem a pergunta de versão — testável.
pub fn compose_session_settings(hook_cmd: &str, blocks: bool, status_line: Value) -> Value {
    let mut settings = hooks_settings_json(hook_cmd);
    if blocks {
        settings = block_pre_tool_use_on_failure(settings);
    }
    settings["statusLine"] = status_line;
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_failure_block_only_from_2_1_295() {
        assert!(supports_on_failure_block("2.1.296 (Claude Code)"));
        assert!(supports_on_failure_block("2.1.295 (Claude Code)"));
        assert!(supports_on_failure_block("2.2.0"));
        assert!(supports_on_failure_block("3.0.0 (Claude Code)"));
        assert!(!supports_on_failure_block("2.1.294 (Claude Code)"));
        assert!(!supports_on_failure_block("1.9.999"));
        assert!(!supports_on_failure_block(""));
        assert!(!supports_on_failure_block("claude version unknown"));
        assert!(!supports_on_failure_block("2.1"));
    }

    #[test]
    fn block_on_failure_touches_only_pre_tool_use() {
        let v = block_pre_tool_use_on_failure(hooks_settings_json("x _hook"));
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["onFailure"],
            "block"
        );
        for event in ["SessionStart", "Stop", "SessionEnd", "Notification"] {
            assert!(
                v["hooks"][event][0]["hooks"][0].get("onFailure").is_none(),
                "{event} não pode travar por falha"
            );
        }
    }

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
    fn all_twelve_events_present() {
        let v = hooks_settings_json("x _hook");
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), 12);
        for name in [
            "PreToolUse",
            "PermissionRequest",
            "PostToolUse",
            "PostToolUseFailure",
            "UserPromptSubmit",
            "SessionStart",
            "Stop",
            "SessionEnd",
            "Notification",
            "SubagentStart",
            "SubagentStop",
            "StopFailure",
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
        assert_eq!(hook["statusMessage"], PRE_TOOL_USE_STATUS_MESSAGE);
    }

    /// O amarelo do Claude não espera o TYBA: a mensagem não pode prometer
    /// uma aprovação que não vai acontecer aqui.
    #[test]
    fn status_message_do_pretooluse_e_neutro() {
        let v = hooks_settings_json("x _hook");
        let msg = v["hooks"]["PreToolUse"][0]["hooks"][0]["statusMessage"]
            .as_str()
            .unwrap()
            .to_lowercase();
        assert!(!msg.contains("aguardando"), "{msg}");
        assert!(!msg.contains("aprova"), "{msg}");
    }

    #[test]
    fn eventos_do_prompt_nativo_tem_a_forma_certa() {
        let cmd = "x _hook";
        let v = hooks_settings_json(cmd);
        for (name, matcher) in [
            ("PermissionRequest", Some("*")),
            ("PostToolUse", Some("*")),
            ("PostToolUseFailure", Some("*")),
            ("UserPromptSubmit", None),
        ] {
            let entry = &v["hooks"][name][0];
            match matcher {
                Some(m) => assert_eq!(entry["matcher"], m, "{name}"),
                None => assert!(entry.get("matcher").is_none(), "{name}"),
            }
            let hooks = entry["hooks"].as_array().unwrap();
            assert_eq!(hooks.len(), 1, "{name}");
            let hook = &hooks[0];
            assert_eq!(hook["type"], "command", "{name}");
            assert_eq!(hook["command"], cmd, "{name}");
            assert_eq!(hook["timeout"], 60, "{name}");
            assert!(hook.get("statusMessage").is_none(), "{name}");
        }
    }

    #[test]
    fn timeout_86400_only_on_pretooluse() {
        let v = hooks_settings_json("x _hook");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 86400);
        for name in [
            "PermissionRequest",
            "PostToolUse",
            "PostToolUseFailure",
            "UserPromptSubmit",
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
        assert_eq!(v["hooks"]["PermissionRequest"][0]["matcher"], "*");
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert_eq!(v["hooks"]["PostToolUseFailure"][0]["matcher"], "*");
        assert_eq!(v["hooks"]["Notification"][0]["matcher"], "idle_prompt");
        for name in [
            "UserPromptSubmit",
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
            "PermissionRequest",
            "PostToolUse",
            "PostToolUseFailure",
            "UserPromptSubmit",
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

    #[test]
    fn stop_failure_sem_matcher_e_sem_bloqueio() {
        let v = block_pre_tool_use_on_failure(hooks_settings_json("x _hook"));
        let entry = &v["hooks"]["StopFailure"][0];
        assert!(entry.get("matcher").is_none());
        let hook = &entry["hooks"][0];
        assert_eq!(hook["type"], "command");
        assert_eq!(hook["command"], "x _hook");
        assert_eq!(hook["timeout"], 60);
        assert!(hook.get("onFailure").is_none());
    }

    #[test]
    fn settings_da_sessao_tem_hooks_e_statusline() {
        let sl = json!({ "type": "command", "command": "'/t' _statusline" });
        let v = compose_session_settings("x _hook", true, sl.clone());
        assert_eq!(v["statusLine"], sl);
        assert!(v["hooks"]["StopFailure"].is_array());
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["onFailure"],
            "block"
        );
        let v = compose_session_settings("x _hook", false, sl);
        assert!(v["hooks"]["PreToolUse"][0]["hooks"][0]
            .get("onFailure")
            .is_none());
    }

    /// `statusLine` e `StopFailure` são do Claude: a config de hooks do Codex
    /// não ganha nenhum dos dois.
    #[test]
    fn codex_nao_recebe_statusline_nem_stop_failure() {
        let joined = crate::agent::codex_hooks::codex_config_overrides("x _hook").join("\n");
        assert!(!joined.contains("StopFailure"), "{joined}");
        assert!(!joined.contains("statusLine"), "{joined}");
        assert!(!joined.contains("_statusline"), "{joined}");
    }
}
