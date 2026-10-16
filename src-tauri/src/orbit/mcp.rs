//! `tyba _orbit-mcp`: o servidor MCP (stdio) do Orquestrador.
//!
//! O CLI do agente de um nó com orquestrar ligado sobe este modo do próprio
//! binário (Claude: `--mcp-config=<arquivo>`; Codex: `--config
//! mcp_servers.orbit.*`). Ele não decide nada: traduz `tools/call` num
//! pedido `OrbitTool` pelo socket de hooks DA SESSÃO (`TYBA_HOOK_SOCKET`, o
//! mesmo canal dos hooks, na pasta 0700 dela) e devolve a resposta do core.
//! Quem valida, cria e para nó é o core (`orbit::orchestrator`).
//!
//! Transporte medido no Claude Code 2.1.301: JSON-RPC 2.0, uma mensagem por
//! linha (sem `Content-Length`), começando por `server/discover` (resposta
//! "método não existe" faz o cliente seguir) e `initialize` com
//! `protocolVersion: "2025-11-25"`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Argumento que liga este modo (`tyba _orbit-mcp`).
pub const MODE: &str = "_orbit-mcp";
/// Nome do servidor no CLI do agente (as ferramentas viram `mcp__orbit__*`
/// no Claude).
pub const SERVER_NAME: &str = "orbit";
/// `hook_event_name` do pedido de ferramenta no socket da sessão.
pub const TOOL_EVENT: &str = "OrbitTool";
/// Arquivo do `--mcp-config` do Claude, no runtime dir 0700 da sessão.
pub const CLAUDE_CONFIG_FILE: &str = "orbit-mcp.json";

/// Versões do protocolo que este servidor fala. Pedido com outra recebe a
/// mais nova (o cliente decide se segue).
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Nomes das ferramentas, na ordem do `tools/list`.
pub const TOOLS: [&str; 6] = [
    "orbit_plan",
    "orbit_create_process",
    "orbit_create_agent",
    "orbit_create_browser",
    "orbit_node_status",
    "orbit_stop_node",
];

pub fn maybe_run_orbit_mcp_mode() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(MODE) {
        return None;
    }
    let socket = std::env::var_os("TYBA_HOOK_SOCKET").map(PathBuf::from);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(stdin.lock(), stdout.lock(), |tool, args| {
        call_core(socket.as_deref(), tool, args)
    });
    Some(0)
}

/// Um pedido de ferramenta pelo socket da sessão. `result` traz o valor;
/// `deny` traz o motivo (validação recusada, sessão sem orquestrar).
fn call_core(socket: Option<&Path>, tool: &str, args: &Value) -> Result<Value, String> {
    let socket = socket.ok_or("sessão sem canal com o TYBA (TYBA_HOOK_SOCKET ausente)")?;
    let event = json!({
        "hook_event_name": TOOL_EVENT,
        "tool": tool,
        "arguments": args,
    });
    let response = crate::hook_ipc::request(socket, &event).ok_or("o TYBA não respondeu")?;
    match (response.action.as_str(), response.result) {
        ("result", Some(value)) => Ok(value),
        _ => Err(response
            .reason
            .unwrap_or_else(|| "recusado pelo TYBA".into())),
    }
}

/// Lê linhas até o EOF e responde cada pedido numa linha. Linha em branco é
/// ignorada; notificação não tem resposta.
pub fn serve<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    mut call: impl FnMut(&str, &Value) -> Result<Value, String>,
) {
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&line, &mut call) {
            let Ok(mut out) = serde_json::to_vec(&response) else {
                continue;
            };
            out.push(b'\n');
            if writer
                .write_all(&out)
                .and_then(|()| writer.flush())
                .is_err()
            {
                return;
            }
        }
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Uma mensagem JSON-RPC → a resposta, ou `None` (notificação, resposta do
/// cliente a algo nosso).
pub fn handle_line(
    line: &str,
    call: &mut impl FnMut(&str, &Value) -> Result<Value, String>,
) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Some(error(Value::Null, PARSE_ERROR, "Parse error")),
    };
    let Some(obj) = message.as_object() else {
        // Lote (array) saiu do MCP em 2025-06-18; escalar não é pedido.
        return Some(error(Value::Null, INVALID_REQUEST, "Invalid Request"));
    };
    let id = obj.get("id").cloned();
    let valid_id = matches!(id, None | Some(Value::String(_)) | Some(Value::Number(_)));
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // Resposta do cliente (tem `result`/`error`) não pede nada de volta.
        if obj.contains_key("result") || obj.contains_key("error") {
            return None;
        }
        return Some(error(
            id.filter(|_| valid_id).unwrap_or(Value::Null),
            INVALID_REQUEST,
            "Invalid Request",
        ));
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !valid_id {
        return Some(error(
            id.filter(|_| valid_id).unwrap_or(Value::Null),
            INVALID_REQUEST,
            "Invalid Request",
        ));
    }
    // Sem id é notificação (`notifications/initialized`, `…/cancelled`).
    let id = id?;
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => ok(id, initialize_result(&params)),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({"tools": tool_list()})),
        "tools/call" => tools_call(id, &params, call),
        _ => error(id, METHOD_NOT_FOUND, "Method not found"),
    })
}

fn initialize_result(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "tyba-orbit", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Ferramentas do Orbit do TYBA. Para subir servidor, \
            processo longo ou outro agente, crie um nó com elas em vez de rodar \
            dentro da sua sessão: o dono vê e para cada nó no canvas. Página local \
            vai num nó de navegador (orbit_create_browser), nunca no navegador \
            do sistema.",
    })
}

fn tools_call(
    id: Value,
    params: &Value,
    call: &mut impl FnMut(&str, &Value) -> Result<Value, String>,
) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error(id, INVALID_PARAMS, "tools/call sem name");
    };
    if !TOOLS.contains(&name) {
        return error(id, INVALID_PARAMS, &format!("Unknown tool: {name}"));
    }
    let args = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return error(id, INVALID_PARAMS, "arguments tem que ser um objeto"),
    };
    // Erro da ferramenta volta como resultado com `isError`, para o modelo
    // ler o motivo e corrigir (é o que o MCP pede).
    let (text, is_error) = match call(name, &args) {
        Ok(value) => (
            serde_json::to_string_pretty(&value).unwrap_or_default(),
            false,
        ),
        Err(reason) => (reason, true),
    };
    ok(
        id,
        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
    )
}

fn string_prop(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

/// As ferramentas como o `tools/list` as anuncia.
pub fn tool_list() -> Vec<Value> {
    let dir = string_prop("Absolute path of an existing folder where it runs.");
    vec![
        json!({
            "name": "orbit_plan",
            "description": format!(
                "Write or update your plan, shown to the owner as the Plan note linked to \
                 you on the canvas. Call it before you start, with every step as todo; then \
                 call it again when a step starts (doing) and when it ends (done or failed), \
                 always with the whole list. In made, name the node you created in that \
                 step (e.g. \"caddy (:443)\"). Up to {} steps, {} characters per text, one \
                 line each.",
                super::orchestrator::MAX_PLAN_STEPS,
                super::orchestrator::MAX_PLAN_TEXT_CHARS
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "The whole plan, in order.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": string_prop("What the step does, one line."),
                                "state": {
                                    "type": "string",
                                    "enum": ["todo", "doing", "done", "failed"]
                                },
                                "made": string_prop(
                                    "Optional: the node you created in this step."
                                )
                            },
                            "required": ["text", "state"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["steps"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "orbit_create_process",
            "description": "Create a Process node on the TYBA Orbit canvas and start it now, in \
                its own terminal that the owner sees and can stop. Use it for servers, watchers \
                and any long command instead of running them in your own shell. Returns the \
                node_id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": string_prop("Short name shown on the node."),
                    "command": string_prop("Shell command (bash -lc, with the owner's login PATH)."),
                    "dir": dir,
                    "output": {
                        "type": "string",
                        "enum": ["log", "json", "diff"],
                        "description": "What the node produces when it ends (default log)."
                    }
                },
                "required": ["title", "command", "dir"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "orbit_create_agent",
            "description": "Create an Agent node (Claude or Codex) on the canvas and start it now \
                with the given task. Returns the node_id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": string_prop("Short name shown on the node."),
                    "runner": {"type": "string", "enum": ["claude", "codex"]},
                    "prompt": string_prop("The task for the agent."),
                    "dir": dir
                },
                "required": ["title", "runner", "prompt", "dir"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "orbit_create_browser",
            "description": format!(
                "Create a Browser node that shows a local page on the canvas; you get the \
                 control link to it. Link it to the node that serves the address with \
                 from_node: the node_id of the Process or Service node (e.g. the Caddy, or \
                 the front-end dev server); the browser waits for it to be ready. If the \
                 address is a virtual host (e.g. https://code.muriki.test served by the \
                 Caddy), also pass url: the browser opens it instead of the node's first \
                 port. Without from_node, give url (http/https on {} only). Returns the \
                 node_id.",
                super::service::LOCAL_HOSTS
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": string_prop("Short name shown on the node."),
                    "url": string_prop(
                        "Local address, e.g. http://localhost:5173 or https://app.test."
                    ),
                    "from_node": string_prop(
                        "node_id of the Process or Service node that serves the page."
                    )
                },
                "required": ["title"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "orbit_node_status",
            "description": "State of a node of this run: status, url/port, exit code, the \
                last lines of its log and, for an agent node, its final answer (text) and a \
                summary of what it changed (diff: files and +/- lines); secrets redacted. \
                Raw output files are not exposed.",
            "inputSchema": {
                "type": "object",
                "properties": {"node_id": string_prop("The node_id.")},
                "required": ["node_id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "orbit_stop_node",
            "description": "Stop a node that you created (kills its process group).",
            "inputSchema": {
                "type": "object",
                "properties": {"node_id": string_prop("The node_id returned by a create tool.")},
                "required": ["node_id"],
                "additionalProperties": false
            }
        }),
    ]
}

/// O `--mcp-config` do Claude: o servidor `orbit` é este binário no modo
/// [`MODE`], com o socket da sessão no env (o CLI não repassa o env inteiro
/// ao servidor stdio).
pub fn claude_config(exe: &Path, socket: &Path) -> Value {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "type": "stdio",
                "command": exe.to_string_lossy(),
                "args": [MODE],
                "env": {"TYBA_HOOK_SOCKET": socket.to_string_lossy()},
            }
        }
    })
}

/// Os argumentos que declaram o servidor no CLI do agente. Claude: o arquivo
/// do [`claude_config`] na forma `--mcp-config=<arquivo>` — a opção é variádica
/// e, separada, comeria o prompt posicional que vem depois. Codex: as
/// `--config mcp_servers.orbit.*` (medido no codex-cli 0.144.1 com `codex
/// mcp get`).
///
/// Os dois recebem a [`ORCHESTRATOR_PROMPT`]: o Claude por
/// `--append-system-prompt`, o Codex por `--config developer_instructions`
/// (medido no codex-cli 0.144.1: `codex debug -c developer_instructions=…
/// prompt-input` põe o texto na mensagem `developer`; chave desconhecida não
/// aparece). Sem ela o Codex subia os serviços no próprio shell.
pub fn agent_args(
    runner: &crate::session::AgentRunnerKind,
    exe: &Path,
    socket: &Path,
    claude_config_path: &Path,
) -> Vec<String> {
    use crate::agent::codex_hooks::toml_string;
    match runner {
        crate::session::AgentRunnerKind::Codex => {
            let key = format!("mcp_servers.{SERVER_NAME}");
            vec![
                "--config".into(),
                format!("{key}.command={}", toml_string(&exe.to_string_lossy())),
                "--config".into(),
                format!("{key}.args=[{}]", toml_string(MODE)),
                "--config".into(),
                format!(
                    "{key}.env={{TYBA_HOOK_SOCKET={}}}",
                    toml_string(&socket.to_string_lossy())
                ),
                "--config".into(),
                format!(
                    "developer_instructions={}",
                    toml_string(ORCHESTRATOR_PROMPT)
                ),
            ]
        }
        _ => vec![
            format!("--mcp-config={}", claude_config_path.to_string_lossy()),
            format!("--append-system-prompt={ORCHESTRATOR_PROMPT}"),
        ],
    }
}

/// A orientação do orquestrador, anexada ao system prompt do Claude e às
/// instruções de desenvolvedor do Codex.
///
/// Sem ela o agente fazia o que faria num terminal qualquer: subia os
/// serviços dentro da própria sessão e abria o Chrome pela extensão — nada
/// aparecia no canvas (teste do dono, 16/10). Com as ferramentas à mão e
/// esta regra, o caminho natural dele passa a ser o Orbit.
pub const ORCHESTRATOR_PROMPT: &str = "Você é o orquestrador de um grafo no Orbit do TYBA. \
O dono acompanha tudo pelo canvas, então trabalhe pelos nós. \
Antes de começar, chame orbit_plan com todos os passos (state todo); \
ao começar cada passo, chame de novo com ele em doing, e ao terminar, em done ou failed, \
sempre com a lista inteira, dizendo em made o nó que criou no passo. \
Para subir ou rodar um serviço, servidor de dev ou processo longo, use orbit_create_process \
(um nó por serviço, na pasta do projeto) em vez de rodar no seu próprio shell; \
para abrir ou mostrar uma página (inclusive domínios .test e .localhost), use orbit_create_browser \
em vez de um navegador externo; para navegador, ligue ao nó que serve o endereço com from_node; \
se o endereço é um host virtual (ex.: https://code.muriki.test servido pelo Caddy), passe também a url; \
use só url quando nenhum nó do grafo serve aquele endereço; \
para trabalho de agente em outro repositório, use orbit_create_agent. \
Use orbit_node_status para ler o estado e o log dos nós antes de concluir \
(de um agente criado, ele traz a resposta final e o que mudou). \
Comandos curtos de leitura (ls, cat, git status) podem rodar no seu shell.";

#[cfg(test)]
mod tests {
    use super::*;

    fn no_call(_: &str, _: &Value) -> Result<Value, String> {
        panic!("não devia chamar o core")
    }

    fn one(line: &str) -> Option<Value> {
        handle_line(line, &mut no_call)
    }

    #[test]
    fn initialize_answers_the_version_the_client_asked() {
        // Como o Claude Code 2.1.301 manda (medido).
        let r = one(
            r#"{"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{"roots":{"listChanged":true}},"clientInfo":{"name":"claude-code","version":"2.1.301"}},"jsonrpc":"2.0","id":0}"#,
        )
        .unwrap();
        assert_eq!(r["id"], 0);
        assert_eq!(r["jsonrpc"], "2.0");
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(r["result"]["serverInfo"]["name"], "tyba-orbit");
        // Versão desconhecida: a mais nova que este servidor fala.
        let r = one(r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#)
            .unwrap();
        assert_eq!(r["id"], "a");
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn discover_probe_gets_method_not_found_and_notifications_get_nothing() {
        let r = one(r#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{}}"#)
            .unwrap();
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(r["id"], "server-discover-probe-1");
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}"#),
            None
        );
        // Resposta do cliente a um pedido nosso: nada a responder.
        assert_eq!(one(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#), None);
        let r = one(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert_eq!(r["result"], json!({}));
    }

    #[test]
    fn tools_list_has_every_tool_with_schemas() {
        let r = one(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, TOOLS);
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{t}");
            assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()));
        }
        let by_name = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(
            by_name("orbit_create_process")["inputSchema"]["required"],
            json!(["title", "command", "dir"])
        );
        let plan = &by_name("orbit_plan")["inputSchema"]["properties"]["steps"]["items"];
        assert_eq!(
            plan["properties"]["state"]["enum"],
            json!(["todo", "doing", "done", "failed"])
        );
        assert_eq!(plan["required"], json!(["text", "state"]));
        // A regra de URL do navegador é a do core, domínios de dev inclusive.
        let browser = tools
            .iter()
            .find(|t| t["name"] == "orbit_create_browser")
            .unwrap();
        let description = browser["description"].as_str().unwrap();
        assert!(description.contains(crate::orbit::service::LOCAL_HOSTS));
        assert!(description.contains("*.test") && description.contains("*.localhost"));
        assert!(description.contains("virtual host") && description.contains("also pass url"));
    }

    #[test]
    fn tools_call_forwards_to_the_core_and_wraps_the_answer() {
        let mut seen = Vec::new();
        let mut call = |tool: &str, args: &Value| {
            seen.push((tool.to_string(), args.clone()));
            if tool == "orbit_stop_node" {
                Err("só nós que você criou".to_string())
            } else {
                Ok(json!({"nodeId": "orq-1"}))
            }
        };
        let r = handle_line(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"orbit_create_process","arguments":{"title":"api","command":"bun run dev","dir":"/tmp"}}}"#,
            &mut call,
        )
        .unwrap();
        assert_eq!(r["id"], 5);
        assert_eq!(r["result"]["isError"], false);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({"nodeId": "orq-1"})
        );

        // Erro da ferramenta é resultado com isError, não erro de protocolo.
        let r = handle_line(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"orbit_stop_node","arguments":{"node_id":"x"}}}"#,
            &mut call,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "só nós que você criou");

        // Sem arguments vira objeto vazio (o core recusa o que faltar).
        handle_line(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"orbit_node_status"}}"#,
            &mut call,
        );
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].0, "orbit_create_process");
        assert_eq!(seen[0].1["command"], "bun run dev");
        assert_eq!(seen[2].1, json!({}));
    }

    #[test]
    fn malformed_input_gets_the_json_rpc_errors() {
        let r = one("{nao é json").unwrap();
        assert_eq!(r["error"]["code"], PARSE_ERROR);
        assert_eq!(r["id"], Value::Null);
        // Lote e escalar.
        for line in [
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
            "42",
            r#""x""#,
        ] {
            assert_eq!(
                one(line).unwrap()["error"]["code"],
                INVALID_REQUEST,
                "{line}"
            );
        }
        // Sem jsonrpc 2.0, id de tipo inválido, sem method.
        for line in [
            r#"{"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
        ] {
            assert_eq!(
                one(line).unwrap()["error"]["code"],
                INVALID_REQUEST,
                "{line}"
            );
        }
        // Ferramenta desconhecida, sem name, arguments que não é objeto.
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rm_rf"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"orbit_stop_node","arguments":[1]}}"#,
        ] {
            assert_eq!(
                one(line).unwrap()["error"]["code"],
                INVALID_PARAMS,
                "{line}"
            );
        }
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#).unwrap()["error"]["code"],
            METHOD_NOT_FOUND
        );
    }

    #[test]
    fn serve_reads_lines_and_writes_one_line_per_answer() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            "\n\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            "\n",
        );
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, no_call);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["result"]["protocolVersion"], "2025-06-18");
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["id"], 1);
    }

    /// Ida e volta pelo socket de verdade: o pedido sai como `OrbitTool`,
    /// a resposta `result` vira valor e a recusa vira o motivo.
    #[cfg(unix)]
    #[test]
    fn call_core_goes_through_the_session_socket() {
        use crate::hook_ipc::{HookAction, HookServer};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hook.sock");
        let _server = HookServer::bind(
            &socket,
            Arc::new(|event| {
                assert_eq!(event.hook_event_name, TOOL_EVENT);
                match event.raw["tool"].as_str() {
                    Some("orbit_node_status") => HookAction::Result {
                        value: json!({"status": "running", "args": event.raw["arguments"]}),
                    },
                    _ => HookAction::Deny {
                        reason: "só nós que você criou".into(),
                    },
                }
            }),
        )
        .unwrap();
        let got = call_core(
            Some(&socket),
            "orbit_node_status",
            &json!({"node_id": "orq-1"}),
        )
        .unwrap();
        assert_eq!(got["status"], "running");
        assert_eq!(got["args"]["node_id"], "orq-1");
        assert_eq!(
            call_core(Some(&socket), "orbit_stop_node", &json!({})).unwrap_err(),
            "só nós que você criou"
        );
        assert!(call_core(None, "orbit_stop_node", &json!({})).is_err());
    }

    #[test]
    fn agent_args_declare_the_server_per_cli() {
        let exe = Path::new("/Applications/TYBA.app/Contents/MacOS/tyba");
        let socket = Path::new("/tmp/tyba-abc/hook.sock");
        let cfg = Path::new("/tmp/tyba-abc/orbit-mcp.json");
        let claude = agent_args(
            &crate::session::AgentRunnerKind::ClaudeCode,
            exe,
            socket,
            cfg,
        );
        assert_eq!(claude[0], "--mcp-config=/tmp/tyba-abc/orbit-mcp.json");
        // Numa peça só, como o --mcp-config: separada, a opção comeria o prompt.
        assert!(claude[1].starts_with("--append-system-prompt=Você é o orquestrador"));
        assert!(claude[1].contains("orbit_create_browser"));
        // Plano antes de começar e navegador pelo nó que serve o endereço.
        assert!(claude[1].contains("orbit_plan") && claude[1].contains("from_node"));
        // Host virtual do Caddy: from_node para esperar, url para abrir.
        assert!(claude[1].contains("host virtual") && claude[1].contains("passe também a url"));
        assert_eq!(claude.len(), 2);
        let config = claude_config(exe, socket);
        assert_eq!(config["mcpServers"]["orbit"]["args"], json!([MODE]));
        assert_eq!(
            config["mcpServers"]["orbit"]["env"]["TYBA_HOOK_SOCKET"],
            "/tmp/tyba-abc/hook.sock"
        );

        let codex = agent_args(&crate::session::AgentRunnerKind::Codex, exe, socket, cfg);
        assert_eq!(codex.len(), 8);
        assert!(
            codex.chunks(2).all(|pair| pair[0] == "--config"),
            "{codex:?}"
        );
        // A orientação, em TOML que o Codex lê de volta inteira.
        let instructions: toml::Value = toml::from_str(&codex[7]).unwrap();
        assert_eq!(
            instructions["developer_instructions"].as_str(),
            Some(ORCHESTRATOR_PROMPT)
        );
        let doc: String = codex[..6]
            .iter()
            .filter(|a| *a != "--config")
            .map(|a| a.replacen("mcp_servers.orbit.", "", 1))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed: toml::Value = toml::from_str(&doc).unwrap();
        assert_eq!(parsed["command"].as_str(), Some(exe.to_str().unwrap()));
        assert_eq!(parsed["args"][0].as_str(), Some(MODE));
        assert_eq!(
            parsed["env"]["TYBA_HOOK_SOCKET"].as_str(),
            Some("/tmp/tyba-abc/hook.sock")
        );
    }
}
