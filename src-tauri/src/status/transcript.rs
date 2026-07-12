use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TAIL_BYTES: u64 = 256 * 1024;

pub fn last_assistant_text(path: &Path, max_chars: usize) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;
    let tail = if start > 0 {
        buf.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        buf.as_str()
    };
    tail.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|entry| assistant_text(&entry).or_else(|| codex_agent_text(&entry)))
        .map(|text| truncate_chars(&collapse_whitespace(&text), max_chars))
        .filter(|text| !text.is_empty())
}

pub fn clean_summary(text: &str, max_chars: usize) -> Option<String> {
    let collapsed = collapse_whitespace(text);
    (!collapsed.is_empty()).then(|| truncate_chars(&collapsed, max_chars))
}

fn codex_agent_text(entry: &serde_json::Value) -> Option<String> {
    if entry.get("type").and_then(|t| t.as_str()) != Some("event_msg") {
        return None;
    }
    let payload = entry.get("payload")?;
    match payload.get("type").and_then(|t| t.as_str()) {
        Some("agent_message") => non_empty(payload.get("message")?.as_str()?.to_string()),
        Some("task_complete") => {
            non_empty(payload.get("last_agent_message")?.as_str()?.to_string())
        }
        _ => None,
    }
}

fn assistant_text(entry: &serde_json::Value) -> Option<String> {
    if entry.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    let content = entry.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return non_empty(text.to_string());
    }
    let parts: Vec<&str> = content
        .as_array()?
        .iter()
        .filter(|part| part.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
        .collect();
    non_empty(parts.join(" "))
}

fn non_empty(text: String) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_transcript(lines: &[&str]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("tyba-transcript-{}-{n}.jsonl", std::process::id()));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn picks_last_assistant_text_block() {
        let path = write_transcript(&[
            r#"{"type":"user","message":{"content":"oi"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"primeira"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"resposta  final"},{"type":"text","text":"do turno"}]}}"#,
        ]);
        assert_eq!(
            last_assistant_text(&path, 140).as_deref(),
            Some("resposta final do turno")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn accepts_plain_string_content() {
        let path =
            write_transcript(&[r#"{"type":"assistant","message":{"content":"texto plano"}}"#]);
        assert_eq!(
            last_assistant_text(&path, 140).as_deref(),
            Some("texto plano")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn skips_malformed_lines_and_non_assistant_entries() {
        let path = write_transcript(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"válida"}]}}"#,
            r#"{corrompido"#,
            r#"{"type":"system","message":{"content":"ruído"}}"#,
        ]);
        assert_eq!(last_assistant_text(&path, 140).as_deref(), Some("válida"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_long_text_on_char_boundary() {
        let long = "á".repeat(300);
        let line = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{long}"}}]}}}}"#
        );
        let path = write_transcript(&[&line]);
        let out = last_assistant_text(&path, 140).unwrap();
        assert!(out.chars().count() <= 140);
        assert!(out.ends_with('…'));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn returns_none_for_missing_file_or_no_assistant_text() {
        assert!(last_assistant_text(Path::new("/nope/x.jsonl"), 140).is_none());
        let path = write_transcript(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#,
        ]);
        assert!(last_assistant_text(&path, 140).is_none());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn picks_last_codex_agent_message_from_rollout() {
        let path = write_transcript(&[
            r#"{"timestamp":"2026-06-22T20:54:27.254Z","type":"session_meta","payload":{"id":"019ef11c","cwd":"/wt"}}"#,
            r#"{"timestamp":"2026-06-22T20:55:01.000Z","type":"event_msg","payload":{"type":"agent_message","message":"primeira fala","phase":null}}"#,
            r#"{"timestamp":"2026-06-22T20:55:02.000Z","type":"response_item","payload":{"type":"message"}}"#,
            r#"{"timestamp":"2026-06-22T20:56:00.000Z","type":"event_msg","payload":{"type":"agent_message","message":"resumo  do turno","phase":null,"memory_citation":null}}"#,
            r#"{"timestamp":"2026-06-22T20:56:00.100Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"019eafd2","last_agent_message":null,"duration_ms":68320}}"#,
        ]);
        assert_eq!(
            last_assistant_text(&path, 140).as_deref(),
            Some("resumo do turno")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn prefers_task_complete_last_agent_message_when_present() {
        let path = write_transcript(&[
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"parcial"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"x","last_agent_message":"consolidado","duration_ms":10}}"#,
        ]);
        assert_eq!(
            last_assistant_text(&path, 140).as_deref(),
            Some("consolidado")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn ignores_codex_events_without_agent_text() {
        let path = write_transcript(&[
            r#"{"type":"event_msg","payload":{"type":"task_started","turn_id":"x"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"x","last_agent_message":null}}"#,
        ]);
        assert!(last_assistant_text(&path, 140).is_none());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reads_only_the_tail_of_large_files() {
        let filler = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"antiga {}"}}]}}}}"#,
            "x".repeat(400)
        );
        let mut lines: Vec<String> = (0..2000).map(|_| filler.clone()).collect();
        lines.push(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"recente"}]}}"#
                .to_string(),
        );
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let path = write_transcript(&refs);
        assert_eq!(last_assistant_text(&path, 140).as_deref(), Some("recente"));
        std::fs::remove_file(&path).unwrap();
    }
}
