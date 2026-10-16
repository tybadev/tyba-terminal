//! Nó de agente do Maestro: o CLI (Claude, Codex) sobe **livre e observado**,
//! como o claude digitado, na pasta do nó — sem worktree, sem jaula.
//!
//! Aqui fica o que é do nó e não do spawn: o prompt com o bloco "Entradas",
//! o snapshot de diff na largada e o diff no fim, o `context` redigido e a
//! espera pelo primeiro turno. O spawn mora em
//! [`crate::agent::session::spawn_free_agent`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{EdgeKind, Runner};
use crate::agent::session::{SharedTurnTail, TurnTail};
use crate::session::{AgentRunnerKind, SessionStatus};

/// Teto do texto do turno que vira `context.md`.
const CONTEXT_MAX_CHARS: usize = 32_000;
const TURN_POLL: Duration = Duration::from_millis(300);
/// O `Stop` do Claude pode chegar antes de o transcript ter a última fala
/// (mesmo cuidado do aviso de fim de turno).
const TURN_SETTLE: Duration = Duration::from_millis(1500);

pub fn runner_kind(runner: Runner) -> AgentRunnerKind {
    match runner {
        Runner::Claude => AgentRunnerKind::ClaudeCode,
        Runner::Codex => AgentRunnerKind::Codex,
    }
}

fn kind_label(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Context => "contexto",
        EdgeKind::Diff => "diff",
        EdgeKind::File => "arquivo",
        EdgeKind::Log => "log",
        EdgeKind::Json => "json",
        EdgeKind::Url => "url",
    }
}

/// Um arquivo de texto pequeno recebido (uma nota ligada pela saída
/// "arquivo"), redigido, para entrar no prompt como o contexto entra. Binário,
/// texto que não é UTF-8 ou acima do teto fica só listado em "Entradas": o
/// dono ligava a nota pela bolinha de arquivo e o agente ignorava a instrução.
pub fn read_text_input(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > (CONTEXT_MAX_CHARS * 4) as u64 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if text.chars().count() > CONTEXT_MAX_CHARS {
        return None;
    }
    let text = text.trim();
    if text.is_empty() || text.contains('\0') {
        return None;
    }
    Some(context_markdown(text))
}

/// O texto de um `context.md` recebido, até o teto, para entrar no prompt.
pub fn read_context(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(CONTEXT_MAX_CHARS).collect())
}

/// O prompt do nó (variáveis já substituídas), o **contexto** recebido por
/// inteiro e o bloco "Entradas": uma linha por arquivo de `$TYBA_IN`.
///
/// Contexto vai no corpo, e não só como caminho: uma nota ligada a um agente
/// é a instrução dele, e listada como arquivo o agente a ignorava (teste do
/// dono, 15/10: a nota pedia 6 subagentes e ele rodou 3).
pub fn build_prompt(
    prompt: &str,
    contexts: &[(String, String)],
    inputs: &[(EdgeKind, PathBuf)],
) -> String {
    if inputs.is_empty() && contexts.is_empty() {
        return prompt.to_string();
    }
    let mut out = prompt.trim_end().to_string();
    for (heading, body) in contexts {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(heading);
        out.push_str(":\n");
        out.push_str(body);
    }
    if inputs.is_empty() {
        return out;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("Entradas (arquivos em $TYBA_IN):");
    for (kind, path) in inputs {
        out.push_str(&format!("\n- {} ({})", path.display(), kind_label(*kind)));
    }
    out
}

/// O texto que sai como `context`, sempre pela redação.
pub fn context_markdown(text: &str) -> String {
    crate::session::redact::redact(text).into_owned()
}

/// A fala final do turno: a do payload (Codex) ou a última do transcript.
fn turn_text(tail: &TurnTail) -> Option<String> {
    if let Some(m) = &tail.last_message {
        return Some(m.clone());
    }
    tail.transcript_path
        .as_deref()
        .and_then(|p| crate::status::transcript::last_assistant_text(p, CONTEXT_MAX_CHARS))
}

/// Como o primeiro turno do agente acabou.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentTurn {
    /// Turno fechado (`Idle`), com a fala final quando houver.
    Done { text: Option<String> },
    /// Turno acabou por erro da API.
    Failed(String),
    /// O agente saiu antes de fechar o turno.
    Exited,
}

#[derive(Debug, PartialEq)]
enum TurnState {
    Pending,
    Ended,
    Gone,
}

/// `Idle` é o turno fechado — inclusive o fechamento segurado por
/// subagentes, que só desce a `Idle` quando o último termina. Prompt aberto
/// (`AwaitingInput`) segue esperando o dono responder no terminal do nó.
fn turn_state(status: Option<&SessionStatus>) -> TurnState {
    match status {
        Some(SessionStatus::Idle { .. }) => TurnState::Ended,
        Some(SessionStatus::Running | SessionStatus::AwaitingInput { .. }) => TurnState::Pending,
        Some(SessionStatus::Exited { .. } | SessionStatus::Failed { .. }) | None => TurnState::Gone,
    }
}

fn turn_from_tail(tail: &TurnTail) -> AgentTurn {
    match &tail.failure {
        Some(reason) => AgentTurn::Failed(reason.clone()),
        None => AgentTurn::Done {
            text: turn_text(tail),
        },
    }
}

/// Espera o primeiro turno numa thread e chama `on_turn` uma vez. O agente
/// continua vivo depois; isto só avisa o Run.
pub fn watch_first_turn(
    status: impl Fn() -> Option<SessionStatus> + Send + 'static,
    tail: SharedTurnTail,
    on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
) {
    std::thread::spawn(move || loop {
        match turn_state(status().as_ref()) {
            TurnState::Pending => std::thread::sleep(TURN_POLL),
            TurnState::Gone => return on_turn(AgentTurn::Exited),
            TurnState::Ended => {
                std::thread::sleep(TURN_SETTLE);
                let tail = tail.lock().expect("turn tail lock").clone();
                return on_turn(turn_from_tail(&tail));
            }
        }
    });
}

/// O estado da pasta na largada do nó. Sem repo git não há snapshot (e o nó
/// não produz diff).
#[derive(Debug, Clone)]
pub struct Snapshot {
    root: PathBuf,
    base: String,
    untracked: HashSet<String>,
}

fn git_stdout(mut cmd: std::process::Command, ok_codes: &[i32]) -> Option<Vec<u8>> {
    let out = cmd.output().ok()?;
    ok_codes.contains(&out.status.code()?).then_some(out.stdout)
}

fn untracked(root: &Path) -> Option<HashSet<String>> {
    let mut cmd = crate::worktree::git_in(root);
    cmd.args(["ls-files", "-z", "--others", "--exclude-standard"]);
    let out = git_stdout(cmd, &[0])?;
    Some(
        out.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

/// `git stash create` (a árvore suja vira um commit solto, sem mexer em nada)
/// ou `HEAD` com a árvore limpa. A identidade fixa só assina esse commit
/// solto, que nunca vai para branch nenhuma.
pub fn snapshot(dir: &Path) -> Option<Snapshot> {
    let root = crate::repo::canonicalize_or(&crate::repo::toplevel(dir)?);
    let mut stash = crate::worktree::git_in_rw(&root);
    stash.args([
        "-c",
        "user.name=TYBA",
        "-c",
        "user.email=maestro@tyba.invalid",
        "stash",
        "create",
    ]);
    let created = git_stdout(stash, &[0])
        .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        .filter(|s| !s.is_empty());
    let base = match created {
        Some(sha) => sha,
        None => {
            let mut head = crate::worktree::git_in(&root);
            head.args(["rev-parse", "--verify", "HEAD"]);
            let out = git_stdout(head, &[0])?;
            String::from_utf8_lossy(&out).trim().to_string()
        }
    };
    let untracked = untracked(&root)?;
    Some(Snapshot {
        root,
        base,
        untracked,
    })
}

/// `git diff <snapshot>` mais cada arquivo não rastreado que surgiu depois da
/// largada, como arquivo novo. Arquivo não rastreado que já existia fica de
/// fora (não é do nó).
pub fn diff_since(snap: &Snapshot) -> String {
    let mut out = Vec::new();
    let mut diff = crate::worktree::git_in(&snap.root);
    diff.args(["diff", "--no-color", "--no-ext-diff", &snap.base, "--"]);
    if let Some(bytes) = git_stdout(diff, &[0]) {
        out.extend(bytes);
    }
    let mut fresh: Vec<String> = untracked(&snap.root)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| !snap.untracked.contains(f))
        .collect();
    fresh.sort();
    for file in fresh {
        let mut cmd = crate::worktree::git_in(&snap.root);
        cmd.args([
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            &file,
        ]);
        // `--no-index` sai 1 quando há diferença.
        if let Some(bytes) = git_stdout(cmd, &[0, 1]) {
            out.extend(bytes);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_lists_each_input_in_one_line() {
        let p = build_prompt(
            "Revise o diff.\n",
            &[],
            &[
                (EdgeKind::Diff, PathBuf::from("/r/n/in/diff.patch")),
                (EdgeKind::Context, PathBuf::from("/r/n/in/context.md")),
            ],
        );
        assert_eq!(
            p,
            "Revise o diff.\n\nEntradas (arquivos em $TYBA_IN):\n\
             - /r/n/in/diff.patch (diff)\n\
             - /r/n/in/context.md (contexto)"
        );
        assert_eq!(build_prompt("só isto", &[], &[]), "só isto");
        assert_eq!(
            build_prompt("", &[], &[(EdgeKind::Log, PathBuf::from("/i/log.txt"))]),
            "Entradas (arquivos em $TYBA_IN):\n- /i/log.txt (log)"
        );
    }

    #[test]
    fn context_goes_into_the_prompt_body() {
        let p = build_prompt(
            "",
            &[("Contexto".into(), "rode 6 subagentes com sleep 10".into())],
            &[(EdgeKind::Context, PathBuf::from("/r/n/in/context.md"))],
        );
        assert_eq!(
            p,
            "Contexto:\nrode 6 subagentes com sleep 10\n\n\
             Entradas (arquivos em $TYBA_IN):\n\
             - /r/n/in/context.md (contexto)"
        );
    }

    #[test]
    fn small_text_file_input_is_read_and_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("instrucao.md");
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        std::fs::write(&note, format!("suba a api\nchave {key}\n")).unwrap();
        let text = read_text_input(&note).unwrap();
        assert!(text.starts_with("suba a api"));
        assert!(!text.contains(&key));

        let bin = dir.path().join("x.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
        assert_eq!(read_text_input(&bin), None);
        let big = dir.path().join("big.txt");
        std::fs::write(&big, "a".repeat(CONTEXT_MAX_CHARS + 1)).unwrap();
        assert_eq!(read_text_input(&big), None);
    }

    #[test]
    fn context_goes_through_redaction() {
        // Fixture sintética.
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        let out = context_markdown(&format!("usei a chave {key} no teste"));
        assert!(!out.contains(&key), "{out}");
        assert!(out.contains("usei a chave"));

        let tail = TurnTail {
            last_message: Some(format!("fim {key}")),
            ..TurnTail::default()
        };
        let AgentTurn::Done { text: Some(text) } = turn_from_tail(&tail) else {
            panic!("turno fechado com texto");
        };
        assert!(!context_markdown(&text).contains(&key));
    }

    #[test]
    fn turn_state_waits_for_idle_and_fails_on_exit() {
        assert_eq!(
            turn_state(Some(&SessionStatus::Running)),
            TurnState::Pending
        );
        assert_eq!(
            turn_state(Some(&SessionStatus::AwaitingInput {
                hint: None,
                reason: Default::default()
            })),
            TurnState::Pending
        );
        assert_eq!(
            turn_state(Some(&SessionStatus::Idle { summary: None })),
            TurnState::Ended
        );
        assert_eq!(
            turn_state(Some(&SessionStatus::Exited { code: 0 })),
            TurnState::Gone
        );
        assert_eq!(turn_state(None), TurnState::Gone);
        let failed = TurnTail {
            failure: Some("API sobrecarregada".into()),
            ..TurnTail::default()
        };
        assert_eq!(
            turn_from_tail(&failed),
            AgentTurn::Failed("API sobrecarregada".into())
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.txt"), "um\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn diff_shows_changed_and_new_files() {
        let dir = repo();
        let snap = snapshot(dir.path()).expect("repo git tem snapshot");
        std::fs::write(dir.path().join("a.txt"), "um\ndois\n").unwrap();
        std::fs::write(dir.path().join("novo.txt"), "oi\n").unwrap();
        let patch = diff_since(&snap);
        assert!(patch.contains("a/a.txt"), "{patch}");
        assert!(patch.contains("+dois"), "{patch}");
        assert!(patch.contains("novo.txt"), "{patch}");
        assert!(patch.contains("new file"), "{patch}");
        assert!(patch.contains("+oi"), "{patch}");
    }

    #[test]
    fn clean_repo_without_changes_gives_empty_patch() {
        let dir = repo();
        let snap = snapshot(dir.path()).unwrap();
        assert_eq!(diff_since(&snap), "");
    }

    #[test]
    fn changes_from_before_the_node_stay_out() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "um\nantes\n").unwrap();
        std::fs::write(dir.path().join("velho.txt"), "x\n").unwrap();
        let snap = snapshot(dir.path()).unwrap();
        assert_eq!(diff_since(&snap), "");
        std::fs::write(dir.path().join("a.txt"), "um\nantes\ndepois\n").unwrap();
        let patch = diff_since(&snap);
        assert!(patch.contains("+depois"), "{patch}");
        assert!(!patch.contains("+antes"), "{patch}");
        assert!(!patch.contains("velho.txt"), "{patch}");
    }

    #[test]
    fn no_git_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        assert!(snapshot(dir.path()).is_none());
    }
}
