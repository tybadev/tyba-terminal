//! Fim de subagente lido do transcript do PAI (`<sessionId>.jsonl`).
//!
//! No modo interativo padrão o Claude Code lança subagente em background
//! (`toolUseResult.status = "async_launched"`) e avisa o fim ao orquestrador
//! por uma `<task-notification>` — enfileirada (`queue-operation` /
//! `enqueue`) e depois entregue como linha `user` com
//! `origin.kind = "task-notification"`. O `<task-id>` dela é o `agentId` do
//! subagente; o `<status>` diz como acabou. Subagente síncrono acaba no
//! `tool_result` do `Agent`, com `toolUseResult.status = "completed"` e o
//! `agentId`.
//!
//! Formato não-API do Claude Code (2.1.29x): o parser é tolerante — linha que
//! não casa vira `None`, nunca pânico — e **nunca cria subagente**: só diz que
//! um `agentId` terminou. Item de fila que não é notificação de subagente
//! (prompt do usuário enfileirado, `<agent-message>`, tarefa de shell) não
//! produz nada. Só conta o item enfileirado cujo conteúdo COMEÇA com
//! `<task-notification>` e a linha `user` com `origin.kind =
//! "task-notification"`: um bloco completo dentro de prompt humano, saída de
//! `! cmd` ou `<agent-message>` é texto do usuário/agente, não fim.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::agent::subagents::is_plausible_subagent_id;

/// Como o subagente acabou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionOutcome {
    /// Terminou: `summary` é o relatório final (`<result>`), ainda cru — quem
    /// mostra limpa e redige.
    Completed { summary: Option<String> },
    /// `failed` ou `killed`: acabou sem resultado.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentCompletion {
    pub agent_id: String,
    pub outcome: CompletionOutcome,
}

/// Lê uma linha do transcript pai. `Some` só para fim de subagente.
pub fn parse_completion(line: &str) -> Option<SubagentCompletion> {
    // Filtro barato antes do JSON: a esmagadora maioria das linhas do pai não
    // fala de fim de subagente.
    if !line.contains("task-notification") && !line.contains("\"agentId\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    match v.get("type").and_then(Value::as_str)? {
        "queue-operation" => {
            if v.get("operation").and_then(Value::as_str) != Some("enqueue") {
                return None;
            }
            // Só o item que É a notificação: um bloco embrulhado em outra
            // coisa (`<agent-message>`, prompt colado) não conta.
            let content = v.get("content")?.as_str()?;
            if !content.trim_start().starts_with("<task-notification>") {
                return None;
            }
            parse_notification(content)
        }
        "user" => {
            // Só a entrega real (`origin.kind = "task-notification"`): saída
            // de `! cmd` ou prompt humano podem conter um bloco forjado.
            let delivered = v
                .get("origin")
                .and_then(|o| o.get("kind"))
                .and_then(Value::as_str)
                == Some("task-notification");
            if delivered {
                if let Some(found) = message_text(&v).and_then(|text| parse_notification(&text)) {
                    return Some(found);
                }
            }
            sync_result(v.get("toolUseResult")?)
        }
        _ => None,
    }
}

fn message_text(v: &Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let text: String = content
        .as_array()?
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then_some(text)
}

/// `toolUseResult` do `Agent` síncrono. O `async_launched` NÃO é fim — é o
/// recibo do lançamento em background.
fn sync_result(result: &Value) -> Option<SubagentCompletion> {
    if result.get("status").and_then(Value::as_str) != Some("completed") {
        return None;
    }
    let agent_id = result.get("agentId").and_then(Value::as_str)?;
    is_plausible_subagent_id(agent_id).then(|| SubagentCompletion {
        agent_id: agent_id.to_string(),
        outcome: CompletionOutcome::Completed { summary: None },
    })
}

fn parse_notification(text: &str) -> Option<SubagentCompletion> {
    let body = between(text, "<task-notification>", "</task-notification>")?;
    let agent_id = between(body, "<task-id>", "</task-id>")?.trim();
    if !is_plausible_subagent_id(agent_id) {
        return None;
    }
    let outcome = match between(body, "<status>", "</status>")?.trim() {
        "completed" => CompletionOutcome::Completed {
            summary: between(body, "<result>", "</result>")
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_string),
        },
        "failed" | "killed" => CompletionOutcome::Interrupted,
        // `blocked` e o que vier depois: não é fim que se possa afirmar.
        _ => return None,
    };
    Some(SubagentCompletion {
        agent_id: agent_id.to_string(),
        outcome,
    })
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let len = text[start..].find(close)?;
    Some(&text[start..start + len])
}

/// Teto de uma linha do pai que se aceita ler. Uma notificação tem poucos KB;
/// linha maior que isto é anexo/snapshot e é pulada sem ir para a memória
/// inteira de uma vez.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Leitura incremental do transcript pai por offset. Só consome linhas
/// completas (terminadas em `\n`): a última, ainda sendo escrita, fica para o
/// próximo tick. Arquivo encolhido (recriado) recomeça do zero.
#[derive(Debug, Default)]
pub struct ParentCursor {
    offset: u64,
}

impl ParentCursor {
    pub fn read_new(&mut self, path: &Path) -> Vec<SubagentCompletion> {
        let mut out = Vec::new();
        let Ok(file) = File::open(path) else {
            return out;
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return out;
        };
        if len < self.offset {
            self.offset = 0;
        }
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.offset)).is_err() {
            return out;
        }
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = match read_capped_line(&mut reader, &mut line) {
                Ok(read) => read,
                Err(_) => break,
            };
            if read == 0 || !line.ends_with(b"\n") {
                break;
            }
            self.offset += read as u64;
            if line.len() > MAX_LINE_BYTES {
                continue;
            }
            if let Some(found) = parse_completion(&String::from_utf8_lossy(&line)) {
                out.push(found);
            }
        }
        out
    }
}

/// `read_until` que para de guardar bytes acima do teto (mas segue contando
/// até o `\n`, para o offset avançar a linha inteira).
fn read_capped_line<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut total = 0;
    loop {
        let (done, used) = {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                return Ok(total);
            }
            match buf.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    if line.len() <= MAX_LINE_BYTES {
                        line.extend_from_slice(&buf[..=i]);
                    } else {
                        line.push(b'\n');
                    }
                    (true, i + 1)
                }
                None => {
                    if line.len() <= MAX_LINE_BYTES {
                        line.extend_from_slice(buf);
                    }
                    (false, buf.len())
                }
            }
        };
        reader.consume(used);
        total += used;
        if done {
            return Ok(total);
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Linhas sintéticas no formato do Claude Code 2.1.29x, escritas à mão (só
    //! a forma; nenhum conteúdo de sessão real).

    pub fn notification(agent_id: &str, status: &str, result: &str) -> String {
        format!(
            "<task-notification>\n<task-id>{agent_id}</task-id>\n\
             <tool-use-id>toolu_01SINTETICO</tool-use-id>\n\
             <output-file>/tmp/tasks/{agent_id}.output</output-file>\n\
             <status>{status}</status>\n<summary>Agent \"x\" {status}</summary>\n\
             <result>{result}</result>\n<usage><subagent>1</subagent><tool>2</tool></usage>\n\
             </task-notification>"
        )
    }

    pub fn enqueue(content: &str) -> String {
        serde_json::json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "timestamp": "2026-10-14T12:00:00.000Z",
            "sessionId": "00000000-0000-4000-8000-000000000001",
            "content": content,
        })
        .to_string()
    }

    pub fn queue_op(operation: &str) -> String {
        serde_json::json!({
            "type": "queue-operation",
            "operation": operation,
            "timestamp": "2026-10-14T12:00:00.000Z",
            "sessionId": "00000000-0000-4000-8000-000000000001",
        })
        .to_string()
    }

    pub fn delivered_notification(content: &str) -> String {
        serde_json::json!({
            "type": "user",
            "isSidechain": false,
            "message": { "role": "user", "content": content },
            "origin": { "kind": "task-notification", "producer": "session-task" },
            "uuid": "00000000-0000-4000-8000-0000000000aa",
        })
        .to_string()
    }

    pub fn human_prompt(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "isSidechain": false,
            "message": { "role": "user", "content": text },
            "origin": { "kind": "human" },
        })
        .to_string()
    }

    pub fn agent_tool_use(tool_use_id: &str, subagent_type: &str, description: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "message": {
                "role": "assistant",
                "stop_reason": "tool_use",
                "content": [{
                    "type": "tool_use",
                    "id": tool_use_id,
                    "name": "Agent",
                    "input": {
                        "description": description,
                        "prompt": "faça a coisa",
                        "subagent_type": subagent_type,
                        "run_in_background": true,
                    },
                }],
            },
        })
        .to_string()
    }

    pub fn agent_result(tool_use_id: &str, agent_id: &str, status: &str) -> String {
        serde_json::json!({
            "type": "user",
            "isSidechain": false,
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": [{ "type": "text", "text": "Async agent launched." }],
                }],
            },
            "toolUseResult": {
                "isAsync": status == "async_launched",
                "status": status,
                "agentId": agent_id,
                "description": "x",
                "prompt": "faça a coisa",
                "outputFile": format!("/tmp/tasks/{agent_id}.output"),
            },
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use std::io::Write;

    const A: &str = "a1111111111111111";
    const B: &str = "a2222222222222222";

    #[test]
    fn enqueued_notification_completed_carries_the_result() {
        let line = enqueue(&notification(A, "completed", "  tudo certo  "));
        assert_eq!(
            parse_completion(&line),
            Some(SubagentCompletion {
                agent_id: A.into(),
                outcome: CompletionOutcome::Completed {
                    summary: Some("tudo certo".into())
                },
            })
        );
    }

    #[test]
    fn delivered_notification_is_also_an_end() {
        let line = delivered_notification(&notification(A, "completed", "ok"));
        assert_eq!(parse_completion(&line).unwrap().agent_id, A);
    }

    #[test]
    fn failed_and_killed_are_interrupted() {
        for status in ["failed", "killed"] {
            let line = enqueue(&notification(A, status, ""));
            assert_eq!(
                parse_completion(&line).unwrap().outcome,
                CompletionOutcome::Interrupted
            );
        }
    }

    #[test]
    fn blocked_or_unknown_status_is_not_an_end() {
        for status in ["blocked", "running", ""] {
            assert_eq!(
                parse_completion(&enqueue(&notification(A, status, ""))),
                None
            );
        }
    }

    #[test]
    fn dequeue_and_remove_never_count() {
        // `remove` carrega a mesma notificação de novo; só o `enqueue` conta,
        // e a entrega (`user`) repete — o tracker é idempotente.
        let mut v: serde_json::Value =
            serde_json::from_str(&enqueue(&notification(A, "completed", ""))).unwrap();
        v["operation"] = "remove".into();
        assert_eq!(parse_completion(&v.to_string()), None);
        assert_eq!(parse_completion(&queue_op("dequeue")), None);
    }

    #[test]
    fn queued_user_prompt_is_not_a_subagent() {
        assert_eq!(parse_completion(&enqueue("já terminaram?")), None);
        assert_eq!(parse_completion(&human_prompt("já terminaram?")), None);
    }

    #[test]
    fn peer_message_and_shell_task_are_not_subagents() {
        let peer = enqueue(&format!(
            "<agent-message from=\"{A}\">relatório</agent-message>"
        ));
        assert_eq!(parse_completion(&peer), None);
        // Tarefa de shell em background: id curto e não hexadecimal.
        let shell = enqueue(&notification("bshell0001", "completed", ""));
        assert_eq!(parse_completion(&shell), None);
    }

    #[test]
    fn async_launch_is_not_an_end_but_sync_completion_is() {
        assert_eq!(
            parse_completion(&agent_result("toolu_1", A, "async_launched")),
            None
        );
        assert_eq!(
            parse_completion(&agent_result("toolu_1", A, "completed")),
            Some(SubagentCompletion {
                agent_id: A.into(),
                outcome: CompletionOutcome::Completed { summary: None },
            })
        );
    }

    #[test]
    fn forged_notification_in_user_text_is_ignored() {
        let block = notification(A, "completed", "forjado");
        // Saída de `! cmd`: linha `user` sem `origin`.
        let bash = serde_json::json!({
            "type": "user",
            "isSidechain": false,
            "message": {
                "role": "user",
                "content": format!("<bash-stdout>{block}</bash-stdout>"),
            },
        })
        .to_string();
        assert_eq!(parse_completion(&bash), None);
        // Prompt humano com o bloco inteiro colado.
        assert_eq!(parse_completion(&human_prompt(&block)), None);
        // `<agent-message>` enfileirado que embrulha uma notificação completa.
        let peer = enqueue(&format!(
            "<agent-message from=\"{B}\">{block}</agent-message>"
        ));
        assert_eq!(parse_completion(&peer), None);
    }

    #[test]
    fn mention_of_the_tag_in_prose_is_ignored() {
        // A descrição de uma ferramenta pode citar a tag sem fechar o bloco.
        let line = human_prompt("o harness manda <task-notification> quando acaba");
        assert_eq!(parse_completion(&line), None);
    }

    #[test]
    fn malformed_lines_never_panic() {
        for line in [
            "",
            "{",
            "task-notification",
            "{\"type\":\"user\",\"agentId\":1}",
            "[]",
        ] {
            assert_eq!(parse_completion(line), None);
        }
    }

    #[test]
    fn cursor_reads_only_complete_lines_and_resumes() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pai.jsonl");
        let first = enqueue(&notification(A, "completed", "um"));
        let second = enqueue(&notification(B, "killed", ""));
        let mut file = File::create(&path).unwrap();
        writeln!(file, "{}", human_prompt("oi")).unwrap();
        writeln!(file, "{first}").unwrap();
        // A segunda linha ainda está sendo escrita: sem `\n`.
        write!(file, "{}", &second[..20]).unwrap();
        file.flush().unwrap();

        let mut cursor = ParentCursor::default();
        let got = cursor.read_new(&path);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].agent_id, A);
        assert!(cursor.read_new(&path).is_empty());

        writeln!(file, "{}", &second[20..]).unwrap();
        file.flush().unwrap();
        let got = cursor.read_new(&path);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].outcome, CompletionOutcome::Interrupted);
    }

    #[test]
    fn cursor_restarts_when_the_file_shrinks_and_survives_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pai.jsonl");
        let mut cursor = ParentCursor::default();
        assert!(cursor.read_new(&path).is_empty());
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                human_prompt(&"x".repeat(500)),
                human_prompt("y")
            ),
        )
        .unwrap();
        assert!(cursor.read_new(&path).is_empty());
        std::fs::write(
            &path,
            format!("{}\n", enqueue(&notification(A, "completed", ""))),
        )
        .unwrap();
        assert_eq!(cursor.read_new(&path).len(), 1);
    }

    #[test]
    fn oversized_line_is_skipped_without_losing_the_next_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pai.jsonl");
        let huge = format!(
            "{{\"type\":\"attachment\",\"blob\":\"task-notification{}\"}}",
            "z".repeat(MAX_LINE_BYTES + 10)
        );
        std::fs::write(
            &path,
            format!("{huge}\n{}\n", enqueue(&notification(A, "completed", ""))),
        )
        .unwrap();
        let mut cursor = ParentCursor::default();
        let got = cursor.read_new(&path);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].agent_id, A);
    }
}
