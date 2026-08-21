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
//! Análise estática de string tem limites e ninguém os cobre hoje. Os dois
//! conhecidos, escritos para não serem redescobertos como surpresa:
//!
//! - `git push` sem refspec com main em checkout passa, porque a branch
//!   corrente não chega até aqui.
//! - O nome do binário pode estar em VALOR de flag do runner, separado do
//!   subcomando: `docker run --entrypoint git img push origin main` não é
//!   recusado. Fechar isso pede decodificar a CLI de cada runner — a lista que
//!   [`git_positions`] existe para não ter.
//!
//! Enquanto isso, a promessa deste módulo é sobre o que está ESCRITO no
//! comando — dizer mais do que isso seria dar por coberto um buraco que
//! continua aberto.

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

/// Comandos cujos argumentos são TEXTO, nunca comando.
///
/// É a única exceção à varredura de [`git_positions`], e é a única direção
/// deste módulo que falha ABERTO: o que entra aqui deixa de ser varrido. Por
/// isso a lista é curta e só tem emissor e buscador de texto — programa que não
/// tem, em nenhuma flag documentada, forma de executar um argumento. `sed` e
/// `awk` ficam de fora de propósito: `sed 's/x/y/e'` e `awk 'BEGIN{system(…)}'`
/// executam.
///
/// A lista existe para um caso conhecido e só para ele: `echo git push main` e
/// `grep -r 'git push origin main' docs` não podem virar recusa, porque um
/// `echo` recusado não tem contorno. Deixar um programa de texto DE FORA custa
/// um falso positivo; colocar um que executa custa o push publicado — por isso
/// crescer esta lista é a mudança perigosa deste arquivo, não encurtá-la.
const TEXT_ONLY_COMMANDS: &[&str] = &["echo", "printf", "grep", "egrep", "fgrep", "rg", "ag"];

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

/// Tira aspas e barras invertidas de UM token antes do casamento.
///
/// Sem isso `git push origin "main"` e `git push origin ma\in` chegam ao
/// comparador como refname diferente de `main` e escapam da recusa. O preço é
/// um falso positivo em texto citado que contenha separador (`echo "; git push
/// origin main"`), e esse é o lado certo de errar: recusa a mais custa um
/// "não" ao agente, recusa a menos publica na main.
///
/// **Por token, nunca na linha inteira.** Aplicado na linha, este `replace`
/// comia também a barra que separa diretório no Windows: `C:\Program Files\Git\
/// bin\git.exe push origin main` virava `C:Program FilesGitbingit.exe …`, o
/// nome do programa dava `C:Program`, e o push para main não era recusado nem
/// pintado de vermelho — ficava amarelo, que é o nível que entra na allowlist
/// de "sempre permitir" da sessão.
fn unquote(token: &str) -> String {
    token.replace(['"', '\'', '\\'], "")
}

/// Onde a barra invertida é separador e onde é escape.
///
/// O mesmo caractere é as duas coisas e o mesmo texto não pode ser as duas: no
/// Windows `\` separa diretório (`C:\Git\bin\git.exe`), no shell POSIX escapa o
/// caractere seguinte (`gi\t` é `git`). A divisão adotada é por posição:
///
/// - **Token do programa**: vale a leitura de caminho. Primeiro corta em `/` e
///   `\` e fica com o último pedaço; só então tira aspas e `.exe`. Era essa
///   ordem que o unquote de linha inteira invertia.
/// - **Qualquer outro token**: vale a leitura de escape, feita por [`unquote`].
///   Refname de git não pode conter `\` (`git check-ref-format` recusa), então
///   num refspec a barra só pode ser escape de shell — tirá-la ali não perde
///   informação.
///
/// Como no token do programa as duas leituras se excluem, ele é testado nas
/// duas por [`program_is`] e basta uma casar: fechar o caminho do Windows não
/// pode reabrir o do POSIX.
fn program_name(token: &str) -> &str {
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let base = base.trim_matches(|c| c == '"' || c == '\'');
    strip_exe(base)
}

/// `.exe` sai sem olhar caixa: `GIT.EXE` roda igual a `git.exe` no Windows.
fn strip_exe(name: &str) -> &str {
    name.len()
        .checked_sub(4)
        .filter(|&cut| name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(".exe"))
        .map_or(name, |cut| &name[..cut])
}

/// Roda `pred` sobre os nomes possíveis do programa nomeado por `token`.
fn program_is(token: &str, pred: impl Fn(&str) -> bool) -> bool {
    if pred(program_name(token)) {
        return true;
    }
    // Leitura de escape do POSIX — só difere da de caminho quando há barra
    // invertida no token, daí o teste antes de alocar.
    token.contains('\\') && pred(program_name(&unquote(token)))
}

fn is_git_program(token: &str) -> bool {
    program_is(token, |name| name.eq_ignore_ascii_case("git"))
}

/// `FOO=bar` na frente do comando é ambiente, não comando.
fn is_assignment(token: &str) -> bool {
    !token.starts_with('-')
        && token.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// `FOO=bar` continua sendo ambiente mesmo citado — `"FOO=bar" git push` não
/// engana o casamento só por causa das aspas.
fn is_assignment_token(token: &str) -> bool {
    is_assignment(token) || (token.contains(['"', '\'']) && is_assignment(&unquote(token)))
}

/// Primeiro token que não é atribuição de ambiente — onde o comando começa.
fn command_start(tokens: &[&str]) -> usize {
    let mut at = 0;
    while tokens.get(at).is_some_and(|t| is_assignment_token(t)) {
        at += 1;
    }
    at
}

/// TODA posição em que um `git` pode estar rodando neste segmento.
///
/// A largura desta varredura é a decisão de segurança do módulo, e ela vem de
/// duas perguntas que parecem uma só e não são: "quem é o programa desta
/// linha?" e "esta linha publica na main?". A primeira é um problema de
/// parsing de shell que ninguém resolve com análise estática; a segunda só
/// precisa de uma resposta conservadora.
///
/// A versão anterior respondia a segunda pergunta com a primeira: aceitava
/// `git` na primeira posição, atrás de uma lista de wrappers (`sudo`, `bash
/// -c`, `cmd /c`) ou atrás de um caminho, e devolvia "não é comando" para
/// qualquer outro. `ssh host git push origin main`, `docker exec c git push
/// origin main` e `kubectl exec pod -- git push origin main` caíam no "não" —
/// não eram recusados e ainda saíam AMARELOS, que é o nível que entra na
/// allowlist de "sempre permitir" da sessão. E o app tem broadcast por SSH: o
/// primeiro deles é o caminho normal do produto, não um contorno exótico.
///
/// Completar a lista de runners não é opção — `ssh`, `docker`, `podman`,
/// `kubectl`, `nix-shell`, `flatpak`, `toolbox`, `distrobox`, `lxc`, o próximo
/// —, porque uma lista de quem executa comando falha ABERTO no nome que ela
/// ainda não tem, em silêncio, e foi exatamente assim que este arquivo abriu a
/// porta duas vezes. Aqui a regra é a inversa: qualquer token `git` isolado
/// conta, venha atrás de quem vier. Binário novo no mundo cai na regra sem
/// ninguém precisar saber o nome dele.
///
/// Varre TODAS as posições, não a primeira: em `nix-shell -p git --run git
/// push origin main` o primeiro `git` é valor de flag do wrapper, e parar nele
/// faria o subcomando cair em `--run`.
///
/// O preço são falsos positivos medidos: `git grep 'git push origin main'` e
/// `./tool git push origin main` passam a ser recusados (o segundo era
/// permitido de propósito antes — o `git` ali pode mesmo ser argumento de um
/// binário local). É o lado certo de errar, o mesmo já documentado em
/// [`unquote`]: recusa a mais custa um "não" que o usuário reescreve; recusa a
/// menos publica num repositório público, e isso não volta atrás. A única
/// exceção é [`TEXT_ONLY_COMMANDS`], para o caso em que a recusa não teria
/// contorno nenhum (`echo git push main`).
fn git_positions<'a>(tokens: &'a [&'a str]) -> impl Iterator<Item = usize> + 'a {
    let scan = !tokens.get(command_start(tokens)).is_some_and(|token| {
        program_is(token, |name| {
            TEXT_ONLY_COMMANDS
                .iter()
                .any(|c| c.eq_ignore_ascii_case(name))
        })
    });
    tokens
        .iter()
        .enumerate()
        .filter_map(move |(at, token)| (scan && is_git_program(token)).then_some(at))
}

/// Uma invocação de `git` encontrada num segmento.
struct GitCall<'a> {
    /// Já sem aspas: `git "push"` é push.
    subcommand: String,
    args: &'a [&'a str],
}

/// Último token do valor da flag em `at`.
///
/// Valor citado e com espaço (`-C "C:\meu repo"`) chega partido pelo
/// `split_whitespace`. Consumir só o primeiro token faria o subcomando cair no
/// miolo do caminho — `git -C "C:\meu repo" push origin main` deixava de ser
/// push. Aspas que nunca fecham devolvem o próprio `at`: engolir o resto da
/// linha esconderia o push em vez de achá-lo.
fn quoted_value_end(tokens: &[&str], at: usize) -> usize {
    let Some(value) = tokens.get(at) else {
        return at;
    };
    let Some(quote) = value.chars().next().filter(|c| *c == '"' || *c == '\'') else {
        return at;
    };
    if value.len() > 1 && value.ends_with(quote) {
        return at;
    }
    tokens[at + 1..]
        .iter()
        .position(|t| t.ends_with(quote))
        .map_or(at, |pos| at + 1 + pos)
}

/// A invocação de `git` que começa na posição `git_at`.
fn git_call_at<'a>(tokens: &'a [&'a str], git_at: usize) -> Option<GitCall<'a>> {
    let mut at = git_at + 1;
    while let Some(&token) = tokens.get(at) {
        let flag = unquote(token);
        if !flag.starts_with('-') {
            break;
        }
        at += 1;
        if GIT_GLOBAL_FLAGS_WITH_VALUE.contains(&flag.as_str()) {
            at = quoted_value_end(tokens, at) + 1;
        }
    }
    let subcommand = unquote(tokens.get(at)?);
    Some(GitCall {
        subcommand,
        args: &tokens[at + 1..],
    })
}

/// Roda o predicado sobre cada invocação de `git` da linha.
///
/// O corte em separadores é por CARACTERE e ignora aspas de propósito: `echo
/// "; git push origin main"` vira dois segmentos e é recusado. Falso positivo
/// conhecido e aceito — ver [`unquote`].
///
/// Basta UMA invocação casar. Um segmento pode ter várias (`git grep … && git
/// push …` já vem partido pelo separador, mas `nix-shell -p git --run git
/// push` não), e exigir que fosse a primeira era como o push se escondia.
fn any_git_call(command: &str, predicate: impl Fn(&GitCall) -> bool) -> bool {
    for segment in command.split(|c| COMMAND_SEPARATORS.contains(&c)) {
        let tokens = word_tokens(segment);
        for at in git_positions(&tokens) {
            if git_call_at(&tokens, at).is_some_and(|call| predicate(&call)) {
                return true;
            }
        }
    }
    false
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
        // Argumento é o território da leitura de escape da barra invertida —
        // ver [`program_name`].
        let arg = unquote(raw);
        // `--all` e `--mirror` não nomeiam ref nenhuma e levam TODAS as branches
        // locais junto: é push para main sem escrever "main".
        if arg == "--all" || arg == "--mirror" {
            return true;
        }
        !arg.starts_with('-') && is_trunk_ref(&arg)
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

    /// A barra invertida do Windows separa diretório, e tirá-la da linha inteira
    /// antes de resolver o nome do binário desmontava o caminho: o basename de
    /// `C:\Program Files\Git\bin\git.exe` virava `C:Program`, o push deixava de
    /// ser push e o princípio 5 do CLAUDE.md ("recusado pelo core. Sempre")
    /// valia só no POSIX.
    #[test]
    fn core_recusa_push_com_caminho_do_windows() {
        assert!(is_refused_by_core(
            r"C:\Program Files\Git\bin\git.exe push origin main"
        ));
        assert!(is_refused_by_core(
            r#""C:\Program Files\Git\bin\git.exe" push origin main"#
        ));
        assert!(is_refused_by_core(
            r"C:\tools\git\bin\git.exe push origin master"
        ));
        assert!(is_refused_by_core(
            r"C:\tools\git\cmd\GIT.EXE push origin main"
        ));
        assert!(is_refused_by_core(
            r"C:\tools\git\bin\git.exe -C C:\repo push origin main"
        ));
        // Valor de flag citado e com espaço chega partido em dois tokens.
        assert!(is_refused_by_core(
            r#"git -C "C:\meu repo" push origin main"#
        ));
        assert!(is_refused_by_core("cmd /c git push origin main"));
        assert!(is_refused_by_core(
            r#"powershell -Command "git push origin main""#
        ));
    }

    /// A mesma barra invertida é escape no shell POSIX. Como o token do programa
    /// não pode ser caminho e escape ao mesmo tempo, ele é lido das duas formas
    /// e basta uma casar — fechar a leitura de caminho não pode abrir a de
    /// escape.
    #[test]
    fn core_recusa_push_com_binario_escapado_no_posix() {
        assert!(is_refused_by_core(r"gi\t push origin main"));
        assert!(is_refused_by_core(r"/usr/bin/gi\t push origin main"));
        assert!(is_refused_by_core(r"\git push origin main"));
    }

    #[test]
    fn core_nao_recusa_caminho_do_windows_que_nao_e_push() {
        assert!(!is_refused_by_core(
            r"C:\Program Files\Git\bin\git.exe push origin feat/x"
        ));
        assert!(!is_refused_by_core(
            r"C:\Program Files\Git\bin\git.exe status"
        ));
        assert!(!is_refused_by_core(r"echo C:\Users\me\main"));
        assert!(!is_refused_by_core(r"copy C:\tools\git.exe D:\bin"));
        assert!(!is_refused_by_core(r"C:\Users\me\projects\main\build.bat"));
    }

    /// Amarelo entra na allowlist de "sempre permitir" da sessão. Push do
    /// Windows classificado como amarelo é push liberado por sessão inteira.
    #[test]
    fn caminho_do_windows_nao_rebaixa_push_para_amarelo() {
        assert_eq!(
            classify_risk(r"C:\Program Files\Git\bin\git.exe push origin main"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk(r#""C:\Program Files\Git\bin\git.exe" push"#),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk(r"C:\tools\git\bin\git.exe push"),
            RiskLevel::Red
        );
    }

    /// Runner remoto ou em container é o MESMO push, só que numa máquina que o
    /// usuário não está olhando — e o app tem broadcast por SSH, então `ssh
    /// host git push origin main` não é hipótese de laboratório.
    ///
    /// Os três primeiros foram medidos devolvendo `refused=false risk=Yellow`
    /// depois do conserto do caminho do Windows: amarelo é o nível que entra na
    /// allowlist de "sempre permitir" da sessão, então era push liberado em
    /// bloco.
    #[test]
    fn core_recusa_push_atras_de_runner_remoto_ou_container() {
        assert!(is_refused_by_core("ssh host git push origin main"));
        assert!(is_refused_by_core("docker exec c git push origin main"));
        assert!(is_refused_by_core(
            "kubectl exec pod -- git push origin main"
        ));
        assert_eq!(
            classify_risk("ssh host git push origin main"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk("docker exec c git push origin main"),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk("kubectl exec pod -- git push origin main"),
            RiskLevel::Red
        );
    }

    /// O conserto não pode ser uma lista de runners conhecidos.
    ///
    /// O buraco não é `ssh`, nem `docker`, nem `kubectl`: é "binário que este
    /// arquivo ainda não ouviu falar". Uma lista de runners fecha os três nomes
    /// e continua aberta para o quarto — e o quarto chega sem avisar. Este
    /// teste falha em qualquer implementação baseada em enumerar quem roda
    /// comando, e é de propósito.
    #[test]
    fn runner_que_ninguem_listou_nao_e_porta_de_saida() {
        let cmd = "runner-que-ainda-nao-existe --flag git push origin main";
        assert!(is_refused_by_core(cmd));
        assert_eq!(classify_risk(cmd), RiskLevel::Red);
    }

    /// O primeiro `git` da linha pode não ser o que roda: em `nix-shell -p git
    /// --run git push`, o primeiro é VALOR de flag do wrapper. Parar no
    /// primeiro faz o subcomando cair em `--run` e o push some.
    #[test]
    fn primeiro_git_da_linha_pode_nao_ser_o_que_roda() {
        assert!(is_refused_by_core(
            "nix-shell -p git --run git push origin main"
        ));
        assert!(is_refused_by_core(
            "apt install git && git push origin main"
        ));
    }

    /// Formas de chamar quem roda o comando.
    ///
    /// Coluna estreitada pelo SEGUNDO conserto deste arquivo: ao fechar o
    /// caminho do Windows, a varredura passou a exigir que o `git` estivesse na
    /// primeira posição, atrás de um prefixo conhecido ou atrás de um caminho —
    /// e todo runner remoto/em container caiu fora.
    const LAUNCHERS: &[&str] = &[
        "",
        "sudo ",
        "env -i ",
        "nohup ",
        "time ",
        "xargs ",
        "bash -c ",
        "sh -c ",
        "cmd /c ",
        "ssh host ",
        "ssh -t user@host ",
        "docker exec c ",
        "docker run --rm img ",
        "podman exec c ",
        "kubectl exec pod -- ",
        "nix-shell -p git --run ",
        "timeout 60 ",
        "toolbox run ",
        "distrobox enter -- ",
        "runner-que-ainda-nao-existe --flag ",
    ];

    /// Formas de nomear o binário do git.
    const GIT_PROGRAMS: &[&str] = &[
        "git",
        "/usr/bin/git",
        "git.exe",
        r"C:\Program Files\Git\bin\git.exe",
        r"gi\t",
        "\"git\"",
    ];

    /// Flags globais entre o `git` e o subcomando.
    ///
    /// Coluna estreitada pelo PRIMEIRO conserto deste arquivo: `git push` era
    /// recusado e `git -C /repo push` passava, porque o casamento olhava só o
    /// token logo depois do `git`.
    const GLOBAL_FLAGS: &[&str] = &[
        "",
        "-C /repo ",
        "-c push.default=current ",
        "--git-dir=/repo/.git ",
        "--work-tree /repo ",
        "--no-pager ",
        "-C /repo -c user.name=x --no-pager ",
    ];

    const PUSH_SPELLINGS: &[&str] = &["push", "\"push\""];

    /// Formas de escrever "main"/"master" como destino.
    const TRUNK_TARGETS: &[&str] = &[
        "origin main",
        "origin master",
        "origin HEAD:main",
        "origin +main",
        "origin HEAD:refs/heads/main",
        "origin :master",
        "origin \"main\"",
        r"origin ma\in",
        "--all origin",
        "--mirror origin",
    ];

    /// Destino que NÃO é trunk. Sensor de discriminação: sem esta coluna,
    /// "recusa tudo" passaria na matriz positiva.
    const NON_TRUNK_TARGETS: &[&str] = &[
        "origin feat/x",
        "origin fix/main-menu",
        "origin main:feat/x",
        "origin HEAD:refs/heads/release",
    ];

    /// Comando de texto que MENCIONA um push: recusar aqui bloqueia um `echo`
    /// sem contorno possível.
    const TEXT_ONLY_MENTIONS: &[&str] = &[
        "echo git push main",
        "echo git push origin main",
        "printf 'git push origin main'",
        "grep -r 'git push origin main' docs",
        "grep push .git/config",
        "egrep 'git push origin main' log",
        "rg 'git push origin main' --files-with-matches",
        "/usr/bin/grep -rn 'git push origin main' .",
        "cat CONTRIBUTING.md",
    ];

    fn push_commands(targets: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for launcher in LAUNCHERS {
            for program in GIT_PROGRAMS {
                for flags in GLOBAL_FLAGS {
                    for push in PUSH_SPELLINGS {
                        for target in targets {
                            out.push(format!("{launcher}{program} {flags}{push} {target}"));
                        }
                    }
                }
            }
        }
        out
    }

    /// A tabela que os dois consertos anteriores não tinham.
    ///
    /// Cada coluna é uma forma de escrever a MESMA coisa — um push para a
    /// trunk — e o produto cartesiano é o teste. Caso solto só pega a linha que
    /// o autor lembrou de escrever; foi assim que o primeiro conserto estreitou
    /// [`GLOBAL_FLAGS`] com a suíte verde, e o segundo estreitou [`LAUNCHERS`]
    /// com a suíte verde de novo. Cruzar as colunas faz cada estreitamento
    /// derrubar centenas de casos de uma vez, em vez de nenhum.
    ///
    /// **Regra para o próximo conserto deste arquivo**: forma nova de invocar
    /// entra como LINHA numa coluna daqui, não como `assert!` avulso no fim do
    /// arquivo. Coluna nova (um jeito novo de escrever a mesma coisa) entra
    /// como coluna.
    #[test]
    fn matriz_de_push_para_trunk_e_sempre_recusada_e_vermelha() {
        for cmd in push_commands(TRUNK_TARGETS) {
            assert!(
                is_refused_by_core(&cmd),
                "push para trunk não recusado pelo core: {cmd:?}"
            );
            assert_eq!(
                classify_risk(&cmd),
                RiskLevel::Red,
                "push para trunk não é vermelho: {cmd:?}"
            );
        }
    }

    /// O outro lado da matriz: recusa cega também é bug. Push para branch de
    /// feature é vermelho (aprovação humana), nunca recusa.
    #[test]
    fn matriz_de_push_fora_da_trunk_nunca_e_recusada() {
        for cmd in push_commands(NON_TRUNK_TARGETS) {
            assert!(
                !is_refused_by_core(&cmd),
                "recusa cega: push fora da trunk foi recusado: {cmd:?}"
            );
            assert_eq!(
                classify_risk(&cmd),
                RiskLevel::Red,
                "todo push é vermelho, mesmo fora da trunk: {cmd:?}"
            );
        }
    }

    #[test]
    fn mencao_a_push_em_comando_de_texto_nao_e_recusada() {
        for cmd in TEXT_ONLY_MENTIONS {
            assert!(
                !is_refused_by_core(cmd),
                "texto que menciona push virou recusa: {cmd:?}"
            );
        }
    }

    /// Invariante entre as duas perguntas que este módulo responde.
    ///
    /// Recusa e classificação usam a mesma varredura, e podem divergir de novo
    /// se alguém estreitar uma só. Divergência na direção perigosa é a mesma
    /// linha sendo negada por um caminho e liberada em BLOCO pelo outro:
    /// amarelo entra na allowlist de "sempre permitir" da sessão.
    #[test]
    fn recusado_pelo_core_nunca_e_allowlistavel() {
        let mut corpus = push_commands(TRUNK_TARGETS);
        corpus.extend(push_commands(NON_TRUNK_TARGETS));
        corpus.extend(TEXT_ONLY_MENTIONS.iter().map(|c| (*c).to_string()));
        for cmd in corpus {
            if is_refused_by_core(&cmd) {
                assert_eq!(
                    classify_risk(&cmd),
                    RiskLevel::Red,
                    "recusado pelo core mas não vermelho: {cmd:?}"
                );
            }
        }
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
