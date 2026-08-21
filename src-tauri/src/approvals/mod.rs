//! Inbox de aprovações: toda ação de agente que exige decisão humana
//! passa por aqui. Estado vive no core (princípio #1); o webview só
//! reflete via eventos `approvals://requested` e `approvals://resolved`.
//!
//! Classificação de risco por padrões (docs/SECURITY.md):
//! - verde: read-only (auto-aprovável se o usuário configurar)
//! - amarelo: escrita dentro do worktree — o default
//! - vermelho: dano público/irreversível — aprovação humana SEMPRE,
//!   hard-coded, nunca entra em allowlist
//!
//! Push para main/master é RECUSADO pelo core antes de virar pedido.
//! Análise estática de string tem limites e ninguém os cobre hoje: `git push`
//! sem refspec com main em checkout passa, porque a branch corrente não chega
//! até aqui. Enquanto não chegar, a promessa deste módulo é sobre o que está
//! ESCRITO no comando — dizer mais do que isso seria dar por coberto um buraco
//! que continua aberto.

pub mod tool_action;
pub mod tool_risk;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::session::SessionId;

pub type SharedApprovals = Arc<ApprovalsManager>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approved,
    Denied,
    ApprovedAlways,
}

impl Decision {
    pub fn is_approval(self) -> bool {
        matches!(self, Decision::Approved | Decision::ApprovedAlways)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub id: u64,
    pub session_id: SessionId,
    pub command: String,
    pub cwd: Option<String>,
    pub risk: RiskLevel,
    /// O que o agente disse que está tentando fazer (stream-json), quando houver.
    pub context: Option<String>,
    pub requested_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApprovalResolved {
    pub id: u64,
    pub decision: Decision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub decision: Decision,
    pub feedback: Option<String>,
}

impl Resolution {
    pub fn denied() -> Self {
        Self {
            decision: Decision::Denied,
            feedback: None,
        }
    }
}

fn word_tokens(command: &str) -> Vec<&str> {
    command.split_whitespace().collect()
}

/// Separadores de shell: depois de qualquer um deles pode começar um comando
/// novo.
///
/// Casados como CARACTERE, não como token. `echo x;git push origin main` não
/// tem espaço em volta do `;`, então `;git` chega inteiro ao `split_whitespace`
/// — casar o separador por token deixaria a forma colada passar batido.
const COMMAND_SEPARATORS: &[char] = &[';', '&', '|', '\n', '\r', '(', ')', '{', '}', '`'];

/// Palavras que rodam OUTRO comando sem mudar o que ele é.
const COMMAND_PREFIXES: &[&str] = &[
    "env", "command", "exec", "nohup", "time", "sudo", "doas", "nice", "stdbuf", "setsid", "xargs",
];

/// Shells que recebem o comando como argumento (`bash -c "git push …"`).
const SHELL_BINARIES: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish", "ash"];

/// Flags globais do `git` que consomem o argumento SEGUINTE.
///
/// São elas que furavam o casamento: com `git -C /repo push`, o subcomando
/// deixa de ser o token logo depois do `git`, e a comparação ingênua devolvia
/// "não é push" para um push. A forma grudada (`--git-dir=…`) não precisa de
/// lista: é um token só, e cai na regra geral de flag.
const GIT_GLOBAL_FLAGS_WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--config-env",
    "--super-prefix",
    "--attr-source",
];

/// Tira aspas e barras invertidas antes do casamento.
///
/// Sem isso `git push origin "main"` e `git push origin ma\in` chegam ao
/// comparador como refname diferente de `main` e escapam da recusa. O preço é
/// um falso positivo em texto citado que contenha separador (`echo "; git push
/// origin main"`), e esse é o lado certo de errar: recusa a mais custa um
/// "não" ao agente, recusa a menos publica na main.
fn unquote(command: &str) -> String {
    command.replace(['"', '\'', '\\'], "")
}

/// Nome do binário sem diretório nem `.exe`: `/usr/bin/git` é `git`.
fn binary_name(token: &str) -> &str {
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    base.strip_suffix(".exe").unwrap_or(base)
}

/// `FOO=bar` na frente do comando é ambiente, não comando.
fn is_assignment(token: &str) -> bool {
    !token.starts_with('-')
        && token.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// Onde está, neste segmento, o `git` que de fato vai rodar.
fn git_at(tokens: &[&str]) -> Option<usize> {
    let mut at = 0;
    while tokens.get(at).is_some_and(|t| is_assignment(t)) {
        at += 1;
    }
    let name = binary_name(tokens.get(at)?);
    if name == "git" {
        return Some(at);
    }
    if COMMAND_PREFIXES.contains(&name) || SHELL_BINARIES.contains(&name) {
        // `sudo git push`, `env -i git push`, `bash -c "git push …"`: o wrapper
        // não muda o que o comando é. Daqui pra frente o primeiro `git` solto é
        // ele — varrer, em vez de decodificar as flags de cada wrapper, é de
        // propósito: `sudo -u app git push` tem valor de flag no meio, e uma
        // tabela de flags por wrapper seria uma segunda lista para manter certa.
        return tokens[at + 1..]
            .iter()
            .position(|t| binary_name(t) == "git")
            .map(|pos| at + 1 + pos);
    }
    // Qualquer outro binário: o `git` que aparecer daqui pra frente é argumento
    // dele, não comando. É o que impede `echo git push main` de virar recusa.
    None
}

/// Uma invocação de `git` encontrada num segmento.
struct GitCall<'a> {
    subcommand: &'a str,
    args: &'a [&'a str],
}

fn git_call<'a>(tokens: &'a [&'a str]) -> Option<GitCall<'a>> {
    let mut at = git_at(tokens)? + 1;
    while let Some(&token) = tokens.get(at) {
        if !token.starts_with('-') {
            break;
        }
        at += 1;
        if GIT_GLOBAL_FLAGS_WITH_VALUE.contains(&token) {
            at += 1;
        }
    }
    let subcommand = *tokens.get(at)?;
    Some(GitCall {
        subcommand,
        args: &tokens[at + 1..],
    })
}

/// Roda o predicado sobre cada invocação de `git` da linha.
fn any_git_call(command: &str, predicate: impl Fn(&GitCall) -> bool) -> bool {
    let line = unquote(command);
    line.split(|c| COMMAND_SEPARATORS.contains(&c))
        .any(|segment| {
            let tokens = word_tokens(segment);
            git_call(&tokens).is_some_and(|call| predicate(&call))
        })
}

/// O destino de um refspec é o que vem depois do último `:` — `HEAD:main`,
/// `HEAD:refs/heads/main`, `:main` (delete) e `+main` (force) chegam todos em
/// `main`.
fn is_trunk_ref(raw: &str) -> bool {
    let spec = raw.trim_start_matches('+');
    let dest = spec.rsplit(':').next().unwrap_or(spec);
    let dest = dest.strip_prefix("refs/heads/").unwrap_or(dest);
    dest == "main" || dest == "master"
}

fn pushes_to_trunk(args: &[&str]) -> bool {
    args.iter().any(|raw| {
        // `--all` e `--mirror` não nomeiam ref nenhuma e levam TODAS as branches
        // locais junto: é push para main sem escrever "main".
        if *raw == "--all" || *raw == "--mirror" {
            return true;
        }
        !raw.starts_with('-') && is_trunk_ref(raw)
    })
}

/// Regra hard-coded do core: push para main/master nunca vira pedido de
/// aprovação — é recusado na hora. Cobre nome direto, refspec (`HEAD:main`),
/// force-push (`+main`) e as flags globais do git no meio (`git -C /repo push`).
pub fn is_refused_by_core(command: &str) -> bool {
    any_git_call(command, |call| {
        call.subcommand == "push" && pushes_to_trunk(call.args)
    })
}

/// Binários triviais sem efeito colateral: só leem ou esperam e não têm modo
/// de escrita/rede alcançável por argumento. Verde apenas quando é exatamente
/// o binário — qualquer operador de shell tira do fast-path (ver
/// `has_shell_operator`). `date`/`hostname` ficam de fora: têm modo de escrita
/// (`date -s`, `hostname NAME`), então não são comprovadamente sem efeito.
const TRIVIAL_COMMANDS: &[&str] = &["sleep", "echo", "true", "false", "whoami", "uname", "id"];

/// Metacaracteres que habilitam encadeamento, redirecionamento ou substituição.
/// Presença de qualquer um desqualifica o comando do fast-path verde: fail-closed.
const SHELL_METACHARACTERS: &[char] = &[
    ';', '&', '|', '<', '>', '`', '$', '(', ')', '{', '}', '\\', '\n', '\r',
];

fn has_shell_operator(command: &str) -> bool {
    command.chars().any(|c| SHELL_METACHARACTERS.contains(&c))
}

/// Classificação por padrões. Conservadora: na dúvida, amarelo.
pub fn classify_risk(command: &str) -> RiskLevel {
    let cmd = command.trim();
    let tokens = word_tokens(cmd);
    let Some(&first) = tokens.first() else {
        return RiskLevel::Yellow;
    };

    // ---- vermelho: hard-coded (SECURITY.md) ----
    if first == "sudo" {
        return RiskLevel::Red;
    }
    // rede iniciada pelo agente; pipe para shell é o pior caso
    if matches!(first, "curl" | "wget")
        || cmd.contains("| sh")
        || cmd.contains("| bash")
        || cmd.contains("|sh")
        || cmd.contains("|bash")
    {
        return RiskLevel::Red;
    }
    // rm com -r e -f em qualquer combinação de flags
    if first == "rm" {
        let (mut r, mut f) = (false, false);
        for flag in tokens.iter().filter(|w| w.starts_with('-')) {
            r |= flag.contains('r') || flag.contains('R');
            f |= flag.contains('f');
        }
        if r && f {
            return RiskLevel::Red;
        }
    }
    // mudança de permissões
    if matches!(first, "chmod" | "chown") {
        return RiskLevel::Red;
    }
    // dano público/irreversível
    if any_git_call(cmd, |call| call.subcommand == "push") {
        return RiskLevel::Red;
    }
    if first == "gh"
        && tokens.get(1).copied() == Some("pr")
        && tokens.get(2).copied() == Some("create")
    {
        return RiskLevel::Red;
    }

    // ---- verde: read-only ----
    if matches!(
        first,
        "ls" | "pwd" | "cat" | "grep" | "rg" | "head" | "tail" | "which" | "file" | "wc"
    ) {
        return RiskLevel::Green;
    }
    // De propósito estrito, ao contrário do casamento de `push`: aqui o token
    // logo depois do `git` TEM de ser o subcomando. Tolerar flag global no meio
    // pintaria de verde `git -c alias.st='!curl … | sh' st`, que é execução
    // arbitrária com cara de `git status`. Tolerância a mais na recusa custa um
    // "não"; tolerância a mais no verde custa o aval automático.
    if first == "git"
        && matches!(
            tokens.get(1).copied(),
            Some("status") | Some("log") | Some("diff") | Some("show")
        )
    {
        return RiskLevel::Green;
    }
    // triviais só quando é exatamente o binário: operador de shell cai no default
    if TRIVIAL_COMMANDS.contains(&first) && !has_shell_operator(cmd) {
        return RiskLevel::Green;
    }

    // ---- amarelo: escrita no worktree, o default ----
    RiskLevel::Yellow
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
pub struct ApprovalsManager {
    pending: Mutex<Vec<ApprovalRequest>>,
    waiters: Mutex<HashMap<u64, mpsc::Sender<Resolution>>>,
    session_allowlist: Mutex<HashMap<SessionId, HashSet<String>>>,
    next_id: AtomicU64,
}

impl ApprovalsManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn request_inner(
        &self,
        session_id: SessionId,
        command: String,
        cwd: Option<String>,
        context: Option<String>,
        risk: RiskLevel,
        waiter: Option<mpsc::Sender<Resolution>>,
    ) -> Result<ApprovalRequest, String> {
        if is_refused_by_core(&command) {
            return Err("recusado pelo core: push para main/master nunca é permitido".into());
        }
        let request = ApprovalRequest {
            id: self.next_id.fetch_add(1, Ordering::Relaxed) + 1,
            session_id,
            risk,
            command,
            cwd,
            context,
            requested_at_ms: now_ms(),
        };
        let mut pending = self.pending.lock().expect("approvals lock");
        pending.push(request.clone());
        if let Some(tx) = waiter {
            self.waiters
                .lock()
                .expect("waiters lock")
                .insert(request.id, tx);
        }
        drop(pending);
        Ok(request)
    }

    pub fn request(
        &self,
        app: &AppHandle,
        session_id: SessionId,
        command: String,
        cwd: Option<String>,
        context: Option<String>,
    ) -> Result<ApprovalRequest, String> {
        let risk = classify_risk(&command);
        let request = self.request_inner(session_id, command, cwd, context, risk, None)?;
        let _ = app.emit("approvals://requested", request.clone());
        Ok(request)
    }

    pub fn request_blocking(
        &self,
        app: &AppHandle,
        session_id: SessionId,
        command: String,
        cwd: Option<String>,
        context: Option<String>,
        risk: RiskLevel,
    ) -> Result<(ApprovalRequest, Resolution), String> {
        let (tx, rx) = mpsc::channel();
        let request = self.request_inner(session_id, command, cwd, context, risk, Some(tx))?;
        let _ = app.emit("approvals://requested", request.clone());
        let resolution = rx.recv().unwrap_or_else(|_| Resolution::denied());
        Ok((request, resolution))
    }

    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        self.pending.lock().expect("approvals lock").clone()
    }

    pub fn is_session_allowed(&self, session_id: SessionId, command: &str) -> bool {
        self.session_allowlist
            .lock()
            .expect("session allowlist lock")
            .get(&session_id)
            .is_some_and(|allowed| allowed.contains(command))
    }

    fn remember_session_allow(&self, request: &ApprovalRequest) {
        if request.risk == RiskLevel::Red {
            return;
        }
        self.session_allowlist
            .lock()
            .expect("session allowlist lock")
            .entry(request.session_id)
            .or_default()
            .insert(request.command.clone());
    }

    fn resolve_inner(&self, id: u64, resolution: Resolution) -> Result<ApprovalRequest, String> {
        let mut pending = self.pending.lock().expect("approvals lock");
        let pos = pending
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("pedido de aprovação {id} não existe"))?;
        let request = pending.remove(pos);
        drop(pending);
        if resolution.decision == Decision::ApprovedAlways {
            self.remember_session_allow(&request);
        }
        if let Some(tx) = self.waiters.lock().expect("waiters lock").remove(&id) {
            let _ = tx.send(resolution);
        }
        Ok(request)
    }

    pub fn resolve(
        &self,
        app: &AppHandle,
        id: u64,
        decision: Decision,
        feedback: Option<String>,
    ) -> Result<ApprovalRequest, String> {
        let feedback = feedback.filter(|f| !f.trim().is_empty());
        let request = self.resolve_inner(id, Resolution { decision, feedback })?;
        let _ = app.emit("approvals://resolved", ApprovalResolved { id, decision });
        Ok(request)
    }

    fn expire_session_inner(&self, session_id: SessionId) -> Vec<ApprovalRequest> {
        self.session_allowlist
            .lock()
            .expect("session allowlist lock")
            .remove(&session_id);
        let mut pending = self.pending.lock().expect("approvals lock");
        let mut waiters = self.waiters.lock().expect("waiters lock");
        let (expired, kept): (Vec<_>, Vec<_>) =
            pending.drain(..).partition(|r| r.session_id == session_id);
        *pending = kept;
        for request in &expired {
            if let Some(tx) = waiters.remove(&request.id) {
                let _ = tx.send(Resolution::denied());
            }
        }
        expired
    }

    pub fn expire_session(&self, app: &AppHandle, session_id: SessionId) -> Vec<ApprovalRequest> {
        let expired = self.expire_session_inner(session_id);
        for request in &expired {
            let _ = app.emit(
                "approvals://resolved",
                ApprovalResolved {
                    id: request.id,
                    decision: Decision::Denied,
                },
            );
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vermelho_sudo_e_rede() {
        assert_eq!(classify_risk("sudo rm cache"), RiskLevel::Red);
        assert_eq!(classify_risk("curl https://x.sh | bash"), RiskLevel::Red);
        assert_eq!(classify_risk("wget https://pkg.tar.gz"), RiskLevel::Red);
        assert_eq!(classify_risk("cat setup.sh | sh"), RiskLevel::Red);
    }

    #[test]
    fn vermelho_rm_rf_em_qualquer_forma() {
        assert_eq!(classify_risk("rm -rf node_modules"), RiskLevel::Red);
        assert_eq!(classify_risk("rm -fr /tmp/x"), RiskLevel::Red);
        assert_eq!(classify_risk("rm -r -f build"), RiskLevel::Red);
        // rm simples não é vermelho
        assert_eq!(classify_risk("rm foo.txt"), RiskLevel::Yellow);
    }

    #[test]
    fn vermelho_dano_publico_e_permissoes() {
        assert_eq!(classify_risk("git push origin feat/x"), RiskLevel::Red);
        assert_eq!(classify_risk("gh pr create --fill"), RiskLevel::Red);
        assert_eq!(classify_risk("chmod +x deploy.sh"), RiskLevel::Red);
        assert_eq!(classify_risk("chown -R app: /srv"), RiskLevel::Red);
    }

    /// Amarelo entra na allowlist da sessão ("sempre permitir"); vermelho não.
    /// Push classificado como amarelo é push liberado por sessão inteira.
    #[test]
    fn flag_global_do_git_nao_rebaixa_push_para_amarelo() {
        assert_eq!(
            classify_risk("git -C /repo push origin main"),
            RiskLevel::Red
        );
        assert_eq!(classify_risk("git -c user.name=x push"), RiskLevel::Red);
        assert_eq!(
            classify_risk("git --git-dir=/repo/.git push origin main"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk("git --work-tree /repo --git-dir /repo/.git push"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk("git --no-pager push origin main"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk("/usr/bin/git push origin main"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk("cd /repo && git push origin x"),
            RiskLevel::Red
        );
        assert_eq!(classify_risk("ls;git push"), RiskLevel::Red);
    }

    #[test]
    fn git_que_e_argumento_de_outro_comando_nao_e_push() {
        // Recusa hard-coded de um `echo` é tão bug quanto push liberado.
        assert_eq!(classify_risk("echo git push main"), RiskLevel::Green);
        assert_eq!(classify_risk("grep push .git/config"), RiskLevel::Green);
    }

    #[test]
    fn core_recusa_push_para_main_master() {
        assert!(is_refused_by_core("git push origin main"));
        assert!(is_refused_by_core("git push --force origin master"));
        assert!(is_refused_by_core("git push origin HEAD:main"));
        assert!(is_refused_by_core("git push origin +main"));
    }

    /// A tabela de fuga: tudo aqui rodava um push para main e passava batido
    /// pelo casamento `tokens[git+1] == "push"`.
    #[test]
    fn core_recusa_push_com_flag_global_no_meio() {
        assert!(is_refused_by_core("git -C /repo push origin main"));
        assert!(is_refused_by_core(
            "git -c push.default=current push origin main"
        ));
        assert!(is_refused_by_core(
            "git --git-dir=/repo/.git push origin main"
        ));
        assert!(is_refused_by_core(
            "git --work-tree /repo push origin master"
        ));
        assert!(is_refused_by_core(
            "git -C /repo -c user.name=x --no-pager push origin main"
        ));
        assert!(is_refused_by_core("git --namespace ns push origin main"));
    }

    #[test]
    fn core_recusa_push_atras_de_wrapper_ou_separador() {
        assert!(is_refused_by_core("cd /repo && git push origin main"));
        assert!(is_refused_by_core("ls;git push origin main"));
        assert!(is_refused_by_core("false || git push origin main"));
        assert!(is_refused_by_core("$(git push origin main)"));
        assert!(is_refused_by_core("bash -c \"git push origin main\""));
        assert!(is_refused_by_core("sudo git push origin main"));
        assert!(is_refused_by_core("GIT_SSH_COMMAND=x git push origin main"));
        assert!(is_refused_by_core("env -i git push origin main"));
        assert!(is_refused_by_core("/usr/bin/git push origin main"));
    }

    #[test]
    fn core_recusa_refspec_que_nao_soletra_main() {
        assert!(is_refused_by_core("git push origin HEAD:refs/heads/main"));
        assert!(is_refused_by_core("git push origin \"main\""));
        assert!(is_refused_by_core("git push origin 'main'"));
        assert!(is_refused_by_core(r"git push origin ma\in"));
        assert!(is_refused_by_core("git push origin refs/heads/master"));
        assert!(is_refused_by_core("git push origin :master"));
        // `--all`/`--mirror` levam todas as branches locais, main junto.
        assert!(is_refused_by_core("git push --all origin"));
        assert!(is_refused_by_core("git push --mirror origin"));
    }

    #[test]
    fn core_nao_recusa_push_para_feature() {
        assert!(!is_refused_by_core("git push origin feat/x"));
        assert!(!is_refused_by_core("git push origin fix/main-menu"));
        assert!(!is_refused_by_core("echo main"));
        assert!(!is_refused_by_core("git status"));
        assert!(!is_refused_by_core("git -C /repo push origin feat/x"));
        // Mandar o conteúdo da main para uma branch de feature é o inverso do
        // dano que a regra existe para impedir.
        assert!(!is_refused_by_core("git push origin main:feat/x"));
        assert!(!is_refused_by_core("git -C /repo status"));
    }

    /// O `git` que é argumento de outro binário não é comando: recusar aqui é
    /// bloquear, sem contorno possível, um `echo`.
    #[test]
    fn core_nao_recusa_git_que_e_argumento() {
        assert!(!is_refused_by_core("echo git push main"));
        assert!(!is_refused_by_core("echo git push origin main"));
        assert!(!is_refused_by_core("grep -r 'git push origin main' docs"));
        assert!(!is_refused_by_core("cat CONTRIBUTING.md"));
    }

    #[test]
    fn verde_read_only() {
        assert_eq!(classify_risk("ls -la"), RiskLevel::Green);
        assert_eq!(classify_risk("git status"), RiskLevel::Green);
        assert_eq!(classify_risk("git log --oneline -5"), RiskLevel::Green);
        assert_eq!(classify_risk("git diff HEAD~1"), RiskLevel::Green);
        assert_eq!(classify_risk("grep -r TODO src"), RiskLevel::Green);
    }

    #[test]
    fn amarelo_e_o_default() {
        assert_eq!(classify_risk("bun add left-pad"), RiskLevel::Yellow);
        assert_eq!(classify_risk("git commit -m 'x'"), RiskLevel::Yellow);
        assert_eq!(classify_risk("cargo build"), RiskLevel::Yellow);
        assert_eq!(classify_risk(""), RiskLevel::Yellow);
    }

    #[test]
    fn verde_comandos_triviais_sozinhos() {
        assert_eq!(classify_risk("sleep 15"), RiskLevel::Green);
        assert_eq!(classify_risk("echo hello world"), RiskLevel::Green);
        assert_eq!(classify_risk("pwd"), RiskLevel::Green);
        assert_eq!(classify_risk("true"), RiskLevel::Green);
        assert_eq!(classify_risk("false"), RiskLevel::Green);
        assert_eq!(classify_risk("whoami"), RiskLevel::Green);
        assert_eq!(classify_risk("uname -a"), RiskLevel::Green);
        assert_eq!(classify_risk("id -u"), RiskLevel::Green);
    }

    #[test]
    fn trivial_com_operador_de_shell_nao_e_verde() {
        assert_ne!(classify_risk("sleep 15; rm x"), RiskLevel::Green);
        assert_ne!(classify_risk("sleep 1 && rm x"), RiskLevel::Green);
        assert_ne!(classify_risk("echo x > file"), RiskLevel::Green);
        assert_ne!(classify_risk("echo x | grep y"), RiskLevel::Green);
        assert_ne!(classify_risk("echo $(whoami)"), RiskLevel::Green);
        assert_ne!(classify_risk("echo `id`"), RiskLevel::Green);
        assert_ne!(classify_risk("echo a > b < c"), RiskLevel::Green);
    }

    #[test]
    fn comando_com_efeito_ou_desconhecido_nao_e_trivial() {
        assert_eq!(classify_risk("date -s 2020-01-01"), RiskLevel::Yellow);
        assert_eq!(classify_risk("hostname new-name"), RiskLevel::Yellow);
        assert_eq!(
            classify_risk("some-random-binary --flag"),
            RiskLevel::Yellow
        );
    }

    fn approval(decision: Decision) -> Resolution {
        Resolution {
            decision,
            feedback: None,
        }
    }

    fn pending_request(
        manager: &ApprovalsManager,
        session_id: SessionId,
        command: &str,
    ) -> (ApprovalRequest, mpsc::Receiver<Resolution>) {
        let (tx, rx) = mpsc::channel();
        let request = manager
            .request_inner(
                session_id,
                command.to_string(),
                None,
                None,
                RiskLevel::Yellow,
                Some(tx),
            )
            .expect("request deve entrar");
        (request, rx)
    }

    #[test]
    fn resolve_entrega_decisao_ao_waiter_e_devolve_o_pedido() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (request, rx) = pending_request(&manager, session, "cargo build");

        let resolved = manager
            .resolve_inner(request.id, approval(Decision::Approved))
            .expect("resolve deve achar o pedido");

        assert_eq!(resolved.command, "cargo build");
        assert_eq!(rx.recv().unwrap().decision, Decision::Approved);
        assert!(manager.list_pending().is_empty());
    }

    #[test]
    fn waiters_concorrentes_resolvem_fora_de_ordem() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (first, rx_first) = pending_request(&manager, session, "a");
        let (second, rx_second) = pending_request(&manager, session, "b");
        let (third, rx_third) = pending_request(&manager, session, "c");

        manager
            .resolve_inner(third.id, Resolution::denied())
            .unwrap();
        manager
            .resolve_inner(first.id, approval(Decision::Approved))
            .unwrap();
        manager
            .resolve_inner(second.id, Resolution::denied())
            .unwrap();

        assert_eq!(rx_first.recv().unwrap().decision, Decision::Approved);
        assert_eq!(rx_second.recv().unwrap().decision, Decision::Denied);
        assert_eq!(rx_third.recv().unwrap().decision, Decision::Denied);
    }

    #[test]
    fn expire_nega_so_os_pendentes_da_sessao() {
        let manager = ApprovalsManager::new();
        let dying = SessionId::new_v4();
        let alive = SessionId::new_v4();
        let (_dying_req, rx_dying) = pending_request(&manager, dying, "a");
        let (_alive_req, rx_alive) = pending_request(&manager, alive, "b");

        let expired = manager.expire_session_inner(dying);

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].session_id, dying);
        assert_eq!(rx_dying.recv().unwrap().decision, Decision::Denied);
        assert!(rx_alive.try_recv().is_err());
        assert_eq!(manager.list_pending().len(), 1);
        assert_eq!(manager.list_pending()[0].session_id, alive);
    }

    #[test]
    fn sempre_permitir_memoriza_o_comando_exato_da_sessao() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (request, rx) = pending_request(&manager, session, "cargo test");

        manager
            .resolve_inner(request.id, approval(Decision::ApprovedAlways))
            .unwrap();

        assert_eq!(rx.recv().unwrap().decision, Decision::ApprovedAlways);
        assert!(manager.is_session_allowed(session, "cargo test"));
        assert!(!manager.is_session_allowed(session, "cargo test --release"));
        assert!(!manager.is_session_allowed(SessionId::new_v4(), "cargo test"));
    }

    #[test]
    fn sempre_permitir_nunca_memoriza_vermelho() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (tx, _rx) = mpsc::channel();
        let request = manager
            .request_inner(
                session,
                "rm -rf build".to_string(),
                None,
                None,
                RiskLevel::Red,
                Some(tx),
            )
            .unwrap();

        manager
            .resolve_inner(request.id, approval(Decision::ApprovedAlways))
            .unwrap();

        assert!(!manager.is_session_allowed(session, "rm -rf build"));
    }

    #[test]
    fn allowlist_morre_com_a_sessao() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (request, _rx) = pending_request(&manager, session, "cargo test");
        manager
            .resolve_inner(request.id, approval(Decision::ApprovedAlways))
            .unwrap();
        assert!(manager.is_session_allowed(session, "cargo test"));

        manager.expire_session_inner(session);

        assert!(!manager.is_session_allowed(session, "cargo test"));
    }

    #[test]
    fn negar_com_feedback_entrega_o_texto_ao_waiter() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (request, rx) = pending_request(&manager, session, "rm build");

        manager
            .resolve_inner(
                request.id,
                Resolution {
                    decision: Decision::Denied,
                    feedback: Some("nao apague o build, rode cargo check".into()),
                },
            )
            .unwrap();

        let resolution = rx.recv().unwrap();
        assert_eq!(resolution.decision, Decision::Denied);
        assert_eq!(
            resolution.feedback.as_deref(),
            Some("nao apague o build, rode cargo check")
        );
    }

    #[test]
    fn approved_always_conta_como_aprovacao() {
        assert!(Decision::ApprovedAlways.is_approval());
        assert!(Decision::Approved.is_approval());
        assert!(!Decision::Denied.is_approval());
    }

    #[test]
    fn resolve_de_id_inexistente_erra_sem_tocar_pendentes() {
        let manager = ApprovalsManager::new();
        let session = SessionId::new_v4();
        let (_req, _rx) = pending_request(&manager, session, "a");

        assert!(manager
            .resolve_inner(999, approval(Decision::Approved))
            .is_err());
        assert_eq!(manager.list_pending().len(), 1);
    }
}
