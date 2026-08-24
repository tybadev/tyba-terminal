//! Identificador nativo da conversa de um agente.
//!
//! O TYBA não inventa esse id: cada CLI já grava o dela em disco e a repassa nos
//! hooks, e é ele que `claude --resume <id>` e `codex resume <id>` aceitam.
//! Inventar um id aqui daria um convite de retomar que sempre falha — pior que
//! não ter convite nenhum.
//!
//! Duas gravações diferentes, um leitor só:
//!
//! - **Claude Code** — `~/.claude/projects/<slug>/<id>.jsonl`, com `sessionId`
//!   repetido em toda entrada.
//! - **Codex** — `~/.codex/sessions/<a>/<m>/<d>/rollout-<ts>-<id>.jsonl`, com o
//!   id no `payload.id` da primeira linha (`type: "session_meta"`).
//!
//! O nome do arquivo só serve de rede de segurança, e só quando ele é o id
//! inteiro (Claude): o do Codex embute timestamp antes do UUID, então lê-lo como
//! id levaria a um `codex resume` de conversa inexistente.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde_json::Value;

/// Teto de leitura. O id vive no cabeçalho dos dois formatos; varrer o arquivo
/// inteiro seria pagar megabytes de transcript por um dado da primeira linha.
const SCAN_BYTES: u64 = 64 * 1024;
const SCAN_LINES: usize = 32;

/// Comprimento aceito. O piso corta lixo (`""`, `"1"`); o teto existe porque o
/// valor vira argumento de linha de comando, e id de conversa de CLI nenhuma
/// chega perto disso.
const MIN_LEN: usize = 8;
const MAX_LEN: usize = 64;

/// O id serve de argumento para `claude --resume` / `codex resume`. Aceitar só
/// esta forma é o que garante que nada com espaço, aspas ou `-` inicial —
/// sequência que a CLI leria como opção — chegue ao argv.
pub fn is_plausible(id: &str) -> bool {
    let len = id.len();
    if !(MIN_LEN..=MAX_LEN).contains(&len) {
        return false;
    }
    if !id.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Id da conversa a partir do payload de um hook.
///
/// Prefere o transcript porque ele é o que o próprio agente escreveu; o
/// `session_id` do payload entra como segunda fonte para o runner que mandar o
/// id sem mandar o caminho.
pub fn from_hook_payload(raw: &Value) -> Option<String> {
    let from_file = raw
        .get("transcript_path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .and_then(|p| from_transcript(Path::new(p)));
    from_file.or_else(|| {
        raw.get("session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| is_plausible(id))
            .map(str::to_string)
    })
}

/// Id da conversa lido do transcript do Claude Code ou do rollout do Codex.
///
/// Tolerante de propósito: arquivo ausente, linha corrompida e formato
/// desconhecido devolvem `None`, nunca erro — sem id o convite simplesmente não
/// aparece.
pub fn from_transcript(path: &Path) -> Option<String> {
    let found = std::fs::File::open(path)
        .ok()
        .and_then(|file| scan_head(BufReader::new(file).take(SCAN_BYTES)));
    found.or_else(|| stem_id(path))
}

fn scan_head(mut reader: impl BufRead) -> Option<String> {
    let mut line = String::new();
    for _ in 0..SCAN_LINES {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(id) = from_entry(&entry) {
            return Some(id);
        }
    }
    None
}

fn from_entry(entry: &Value) -> Option<String> {
    let candidate = if entry.get("type").and_then(Value::as_str) == Some("session_meta") {
        let payload = entry.get("payload")?;
        payload
            .get("id")
            .or_else(|| payload.get("session_id"))
            .and_then(Value::as_str)
    } else {
        entry.get("sessionId").and_then(Value::as_str)
    }?;
    let candidate = candidate.trim();
    is_plausible(candidate).then(|| candidate.to_string())
}

/// O nome do arquivo, quando ele é um UUID inteiro — o caso do Claude Code.
///
/// A forma é conferida em vez de aceita: o rollout do Codex se chama
/// `rollout-<timestamp>-<uuid>.jsonl`, e passar esse nome adiante como id daria
/// um `codex resume` que não encontra conversa nenhuma.
fn stem_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    is_uuid(stem).then(|| stem.to_string())
}

fn is_uuid(raw: &str) -> bool {
    let groups: Vec<&str> = raw.split('-').collect();
    if groups.len() != 5 {
        return false;
    }
    let sizes = [8usize, 4, 4, 4, 12];
    groups
        .iter()
        .zip(sizes)
        .all(|(g, size)| g.len() == size && g.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Fixtures escritas à mão, como manda o CLAUDE.md: nada aqui sai de
    /// `~/.claude/projects` nem de `~/.codex/sessions`.
    fn write(name: &str, lines: &[&str]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tyba-conv-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    const CLAUDE_ID: &str = "5f2a1c40-0000-4000-8000-00000000abcd";
    const CODEX_ID: &str = "019fd9eb-1b97-7811-9002-13dca6ae6ba7";

    #[test]
    fn reads_claude_session_id_from_the_transcript_body() {
        let path = write(
            "qualquer-nome.jsonl",
            &[
                &format!(r#"{{"type":"last-prompt","sessionId":"{CLAUDE_ID}"}}"#),
                r#"{"type":"assistant","message":{"content":"oi"}}"#,
            ],
        );
        assert_eq!(from_transcript(&path).as_deref(), Some(CLAUDE_ID));
    }

    #[test]
    fn reads_codex_id_from_the_session_meta_header() {
        let path = write(
            "rollout-2026-08-06T22-51-31-019fd9eb-1b97-7811-9002-13dca6ae6ba7.jsonl",
            &[
                &format!(
                    r#"{{"timestamp":"2026-08-06T22:51:31.000Z","type":"session_meta","payload":{{"id":"{CODEX_ID}","cwd":"/wt"}}}}"#
                ),
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"oi"}}"#,
            ],
        );
        assert_eq!(from_transcript(&path).as_deref(), Some(CODEX_ID));
    }

    #[test]
    fn accepts_session_meta_that_only_carries_session_id() {
        let path = write(
            "rollout-2026-08-06T22-51-31-019fd9eb-1b97-7811-9002-13dca6ae6ba7.jsonl",
            &[&format!(
                r#"{{"type":"session_meta","payload":{{"session_id":"{CODEX_ID}","cwd":"/wt"}}}}"#
            )],
        );
        assert_eq!(from_transcript(&path).as_deref(), Some(CODEX_ID));
    }

    #[test]
    fn skips_malformed_lines_before_the_header() {
        let path = write(
            "x.jsonl",
            &[
                "",
                "{corrompido",
                r#"{"type":"summary"}"#,
                &format!(r#"{{"type":"user","sessionId":"{CLAUDE_ID}"}}"#),
            ],
        );
        assert_eq!(from_transcript(&path).as_deref(), Some(CLAUDE_ID));
    }

    #[test]
    fn falls_back_to_the_file_name_when_it_is_a_whole_uuid() {
        let path = write(
            &format!("{CLAUDE_ID}.jsonl"),
            &[r#"{"type":"assistant","message":{"content":"sem sessionId"}}"#],
        );
        assert_eq!(from_transcript(&path).as_deref(), Some(CLAUDE_ID));
    }

    /// O nome do rollout do Codex embute timestamp antes do UUID. Aceitá-lo como
    /// id daria um convite que abre `codex resume rollout-2026-…` — conversa que
    /// não existe. Sem id é o resultado certo.
    #[test]
    fn never_takes_the_codex_rollout_file_name_as_the_id() {
        let path = write(
            "rollout-2026-08-06T22-51-31-019fd9eb-1b97-7811-9002-13dca6ae6ba7.jsonl",
            &[r#"{"type":"event_msg","payload":{"type":"agent_message","message":"oi"}}"#],
        );
        assert_eq!(from_transcript(&path), None);
    }

    #[test]
    fn missing_file_has_no_id() {
        assert_eq!(from_transcript(Path::new("/nope/x.jsonl")), None);
    }

    #[test]
    fn ignores_implausible_ids_in_the_body() {
        let path = write(
            "y.jsonl",
            &[
                r#"{"type":"user","sessionId":"x"}"#,
                r#"{"type":"user","sessionId":"--resume /etc/passwd"}"#,
            ],
        );
        assert_eq!(from_transcript(&path), None);
    }

    #[test]
    fn reads_only_the_head_of_a_long_transcript() {
        let filler = r#"{"type":"assistant","message":{"content":"linha sem id"}}"#;
        let mut lines: Vec<String> = (0..SCAN_LINES + 40).map(|_| filler.to_string()).collect();
        lines.push(format!(r#"{{"type":"user","sessionId":"{CLAUDE_ID}"}}"#));
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let path = write("z.jsonl", &refs);
        assert_eq!(
            from_transcript(&path),
            None,
            "o id mora no cabeçalho; varrer o transcript inteiro seria I/O por nada"
        );
    }

    #[test]
    fn hook_payload_prefers_the_transcript_over_the_inline_field() {
        let path = write(
            "w.jsonl",
            &[&format!(r#"{{"type":"user","sessionId":"{CLAUDE_ID}"}}"#)],
        );
        let raw = json!({
            "hook_event_name": "Stop",
            "transcript_path": path.to_string_lossy(),
            "session_id": CODEX_ID,
        });
        assert_eq!(from_hook_payload(&raw).as_deref(), Some(CLAUDE_ID));
    }

    #[test]
    fn hook_payload_falls_back_to_the_inline_session_id() {
        let raw = json!({
            "hook_event_name": "SessionStart",
            "transcript_path": "/nao/existe.jsonl",
            "session_id": CODEX_ID,
        });
        assert_eq!(from_hook_payload(&raw).as_deref(), Some(CODEX_ID));
    }

    #[test]
    fn hook_payload_without_any_source_has_no_id() {
        assert_eq!(from_hook_payload(&json!({"hook_event_name": "Stop"})), None);
        assert_eq!(
            from_hook_payload(&json!({"session_id": 42, "transcript_path": ""})),
            None
        );
    }

    #[test]
    fn plausible_ids_are_argv_safe() {
        assert!(is_plausible(CLAUDE_ID));
        assert!(is_plausible("019fd9eb1b9778119002"));
        assert!(!is_plausible(""));
        assert!(!is_plausible("curta"));
        assert!(!is_plausible("-rf /tmp/algo-importante"));
        assert!(!is_plausible("id com espaço aqui"));
        assert!(!is_plausible("id;rm -rf /"));
        assert!(!is_plausible(&"a".repeat(MAX_LEN + 1)));
    }
}
