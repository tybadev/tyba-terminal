//! O canal próprio de um Host: comando curto sobre a conexão que já está
//! aberta, **nunca** pelo PTY.
//!
//! Roda por `files::remote::RemoteFs::exec`, que já executa no host pela
//! conexão multiplexada — nenhum caminho de `ssh` novo nasce aqui (é o achado
//! do scout de impacto, §7 do desenho). O que trafega: os nomes de comando do
//! servidor (uma vez por conexão) e o git dos chips (com teto de frequência).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::error::AppError;
use crate::files::remote::fs::RemoteFs;

/// O que os chips mostram do servidor.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChips {
    pub branch: Option<String>,
    pub changed: u32,
}

/// O que a sessão SSH entrega para os chips (regra 23): tudo do servidor,
/// nunca da máquina local.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteChips {
    pub cwd: Option<String>,
    pub git: GitChips,
}

/// O que o canal pergunta ao servidor antes de a sessão subir: o shell de login
/// e a existência do tmux, numa ida só.
///
/// A marca do tmux vem numa segunda linha porque a primeira pode ser vazia
/// (`$SHELL` em branco) — e uma linha em branco ainda é uma resposta.
const TMUX_MARK: &str = "tmux";
const PROBE_SCRIPT: &str =
    "printf %s \"$SHELL\"; echo; command -v tmux >/dev/null 2>&1 && printf %s tmux";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostProbe {
    pub shell: crate::ssh::remote_rc::RemoteShell,
    pub persistence: crate::ssh::Persistence,
}

impl Default for HostProbe {
    /// Nada apurado: shell desconhecido (regra 8) e persistência desconhecida
    /// (regra 13). Nenhum dos dois vira palpite.
    fn default() -> Self {
        Self {
            shell: crate::ssh::remote_rc::RemoteShell::Unsupported("desconhecido".into()),
            persistence: crate::ssh::Persistence::Unknown,
        }
    }
}

impl HostProbe {
    fn parse(raw: &[u8]) -> Self {
        use crate::ssh::Persistence;
        let text = String::from_utf8_lossy(raw);
        // Sem a quebra de linha o script não chegou ao teste do tmux: o
        // servidor foi mudo, e "não tem tmux" seria conclusão de quem não
        // perguntou.
        let Some((shell_line, tmux_line)) = text.split_once('\n') else {
            return Self::default();
        };
        Self {
            shell: crate::ssh::remote_rc::RemoteShell::from_path(shell_line),
            persistence: if tmux_line.trim() == TMUX_MARK {
                Persistence::Persistent
            } else {
                Persistence::Ephemeral
            },
        }
    }
}

/// Relógio injetado: o teto de 2 s da regra 24 se testa sem dormir.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        crate::approvals::now_ms() as i64
    }
}

/// Teto de frequência e exclusão, por sessão (regra 24).
#[derive(Default)]
struct Gate {
    last_ms: Option<i64>,
    in_flight: bool,
}

/// O intervalo mínimo entre duas consultas da mesma sessão.
pub const MIN_INTERVAL_MS: i64 = 2_000;

/// Tetos da regra 20, sobre o que vem do servidor.
pub const MAX_COMMAND_NAMES: usize = 8_000;
pub const MAX_COMMAND_BYTES: usize = 512 * 1024;

/// Nome de comando plausível: o que o servidor manda é texto de terceiro, e a
/// lista alimenta uma sugestão que o dono pode aceitar com um Tab. Nada de
/// barra (seria caminho), espaço, controle ou nome vazio.
fn plausible(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '/')
}

/// Um nome por linha, com teto, dedup e ordem estável.
fn parse_command_names(raw: &[u8]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in String::from_utf8_lossy(raw).lines() {
        let name = line.trim_end_matches('\r');
        if !plausible(name) || !seen.insert(name.to_string()) {
            continue;
        }
        out.push(name.to_string());
        if out.len() >= MAX_COMMAND_NAMES {
            break;
        }
    }
    out
}

type Connect = Box<dyn Fn(&str) -> Result<Arc<dyn RemoteFs>, AppError> + Send + Sync>;

pub struct HostQuery {
    alias: String,
    connect: Connect,
    clock: Arc<dyn Clock>,
    fs: Mutex<Option<Arc<dyn RemoteFs>>>,
    /// Cache por Host, pela vida da conexão (regra 20).
    names: Mutex<Option<Arc<Vec<String>>>>,
    probe: Mutex<Option<HostProbe>>,
    gates: Mutex<HashMap<uuid::Uuid, Gate>>,
}

impl HostQuery {
    pub fn new(alias: &str, connect: Connect, clock: Arc<dyn Clock>) -> Self {
        Self {
            alias: alias.to_string(),
            connect,
            clock,
            fs: Mutex::new(None),
            names: Mutex::new(None),
            probe: Mutex::new(None),
            gates: Mutex::new(HashMap::new()),
        }
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// O git dos chips, sujeito ao teto da regra 24.
    ///
    /// `None` é "agora não" — outra consulta desta sessão está em voo, ou a
    /// anterior foi há menos de 2 s. Não é erro e não vira evento: o chip
    /// continua mostrando o que já mostrava.
    pub fn git_chips_gated(&self, session: uuid::Uuid, cwd: &str) -> Option<GitChips> {
        if !self.enter(session) {
            return None;
        }
        let out = self.git_chips(cwd).ok();
        self.leave(session);
        out
    }

    /// Os chips daquela sessão, do servidor, sujeitos ao mesmo teto.
    ///
    /// A pasta sai do pane do tmux remoto (`#{pane_current_path}`), que é a
    /// fonte que existe com ou sem integração de shell — o `OSC 7` do rc
    /// alimenta o chip de pasta na tela, e esta é a que ancora o git.
    pub fn session_chips(&self, session: uuid::Uuid, tmux_name: &str) -> Option<RemoteChips> {
        if !self.enter(session) {
            return None;
        }
        let chips = self.chips_now(tmux_name);
        self.leave(session);
        Some(chips)
    }

    fn chips_now(&self, tmux_name: &str) -> RemoteChips {
        let cwd = self.remote_cwd(tmux_name);
        let git = match cwd.as_deref() {
            Some(cwd) => self.git_chips(cwd).unwrap_or_default(),
            None => GitChips::default(),
        };
        RemoteChips { cwd, git }
    }

    /// A pasta do pane remoto. Um `exec` só, e o mesmo formato que o painel de
    /// arquivos remoto já usa.
    pub fn remote_cwd(&self, tmux_name: &str) -> Option<String> {
        let fs = self.fs().ok()?;
        let out = fs
            .exec(&[
                "tmux",
                "display-message",
                "-p",
                "-t",
                tmux_name,
                "-F",
                "#{pane_current_path}",
            ])
            .ok()?;
        if !out.ok() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!path.is_empty()).then_some(path)
    }

    fn enter(&self, session: uuid::Uuid) -> bool {
        let now = self.clock.now_ms();
        let mut gates = self.gates.lock();
        let gate = gates.entry(session).or_default();
        if gate.in_flight {
            return false;
        }
        if gate.last_ms.is_some_and(|at| now - at < MIN_INTERVAL_MS) {
            return false;
        }
        gate.in_flight = true;
        gate.last_ms = Some(now);
        true
    }

    fn leave(&self, session: uuid::Uuid) {
        if let Some(gate) = self.gates.lock().get_mut(&session) {
            gate.in_flight = false;
        }
    }

    /// A sessão morreu: o teto dela não tem mais o que segurar.
    pub fn forget(&self, session: uuid::Uuid) {
        self.gates.lock().remove(&session);
    }

    /// Quantas sessões este canal ainda acompanha.
    ///
    /// Existe para a limpeza ser observável: um mapa que só cresce não tem
    /// sintoma — nem erro, nem lentidão visível — e a única forma de provar que
    /// ele encolhe é perguntar o tamanho.
    pub fn tracked_sessions(&self) -> usize {
        self.gates.lock().len()
    }

    /// Os nomes de comando do servidor, uma vez por conexão (regra 20).
    ///
    /// Só **nomes** atravessam o canal, nunca corpo de alias — é o limite que a
    /// tech-spec do primeiro token já fixou. O que vem é texto de terceiro: o
    /// filtro de nome plausível é o mesmo cuidado de `completion::binary`.
    pub fn command_names(&self) -> Result<Arc<Vec<String>>, AppError> {
        if let Some(cached) = self.names.lock().as_ref() {
            return Ok(Arc::clone(cached));
        }
        let fs = self.fs()?;
        // `head -c` no SERVIDOR: leitura limitada na origem, não depois de já
        // ter atravessado a conexão.
        let script = format!(
            "IFS=:; for d in $PATH; do ls -1 \"$d\" 2>/dev/null; done | head -c {MAX_COMMAND_BYTES}"
        );
        let out = fs
            .exec(&["sh", "-c", &script])
            .map_err(|e| AppError::new("ssh.query_failed").with("detail", e.message()))?;
        let names = parse_command_names(&out.stdout);
        let names = Arc::new(names);
        *self.names.lock() = Some(Arc::clone(&names));
        Ok(names)
    }

    /// O que o canal precisa saber do servidor ANTES de a sessão subir: o shell
    /// de login (regra 8) e se existe tmux ali (regra 13).
    ///
    /// Uma ida só, em cache por Host: a resposta não muda entre sessões, e o
    /// custo é um `exec` na conexão multiplexada. Falhou (sem ControlMaster,
    /// Host de senha sem sessão aberta, servidor mudo) → shell `Unsupported`,
    /// que pela regra 8 é sessão comum com motivo, e persistência `Unknown` —
    /// nunca um palpite de bash nem um palpite de tmux.
    pub fn host_probe(&self) -> HostProbe {
        if let Some(cached) = self.probe.lock().as_ref() {
            return cached.clone();
        }
        let detected = self
            .fs()
            .ok()
            .and_then(|fs| fs.exec(&["sh", "-c", PROBE_SCRIPT]).ok())
            .filter(|out| out.ok())
            .map(|out| HostProbe::parse(&out.stdout))
            .unwrap_or_default();
        *self.probe.lock() = Some(detected.clone());
        detected
    }

    /// Branch e contagem de alterações do servidor, na disciplina do princípio
    /// #8 (`-z`, sem cor, sem quoting do path) — o mesmo molde de
    /// `files/remote/gitexec.rs`.
    pub fn git_chips(&self, cwd: &str) -> Result<GitChips, AppError> {
        let fs = self.fs()?;
        let branch = match fs.exec(&git_argv(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])) {
            Ok(out) if out.ok() => {
                let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // `HEAD` solto é detached: nome nenhum é mais honesto que o
                // literal "HEAD", que pareceria uma branch chamada HEAD.
                (!name.is_empty() && name != "HEAD").then_some(name)
            }
            _ => None,
        };
        let changed = match fs.exec(&git_argv(cwd, &["status", "--porcelain", "-z"])) {
            Ok(out) if out.ok() => crate::repo::count_status_entries(&out.stdout),
            _ => 0,
        };
        Ok(GitChips { branch, changed })
    }

    /// A conexão do canal, construída na primeira pergunta e reusada depois.
    fn fs(&self) -> Result<Arc<dyn RemoteFs>, AppError> {
        let mut slot = self.fs.lock();
        if let Some(fs) = slot.as_ref() {
            return Ok(Arc::clone(fs));
        }
        let fs = (self.connect)(&self.alias)?;
        *slot = Some(Arc::clone(&fs));
        Ok(fs)
    }
}

/// O argv do git remoto, com as flags do princípio #8. `color.ui=false` no
/// lugar de um `--no-color` por subcomando: o `status` não tem essa flag, e uma
/// config vale para os dois.
fn git_argv<'a>(cwd: &'a str, sub: &[&'a str]) -> Vec<&'a str> {
    let mut argv = vec![
        "git",
        "-C",
        cwd,
        "-c",
        "core.quotePath=false",
        "-c",
        "color.ui=false",
    ];
    argv.extend_from_slice(sub);
    argv
}

/// Um canal por Host, vivo enquanto o app estiver de pé.
///
/// O cache é por Host e não por sessão de propósito: duas sessões do mesmo
/// servidor perguntam a mesma coisa, e pagar dois `ssh` por resposta idêntica é
/// o que a conexão multiplexada existe para evitar.
#[derive(Default)]
pub struct HostQueries {
    by_host: Mutex<HashMap<String, Arc<HostQuery>>>,
}

pub type SharedHostQueries = Arc<HostQueries>;

impl HostQueries {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, host_id: &str, alias: &str) -> Arc<HostQuery> {
        let mut all = self.by_host.lock();
        if let Some(found) = all.get(host_id) {
            if found.alias() == alias {
                return Arc::clone(found);
            }
        }
        let fresh = Arc::new(HostQuery::new(
            alias,
            Box::new(|alias| {
                crate::ssh::command::require_session_if_password(alias)?;
                let fs = crate::files::remote::sftpwire::SshRemote::connect(alias)
                    .map_err(|e| AppError::new("ssh.query_failed").with("detail", e.message()))?;
                Ok(Arc::new(fs) as Arc<dyn RemoteFs>)
            }),
            Arc::new(SystemClock),
        ));
        all.insert(host_id.to_string(), Arc::clone(&fresh));
        fresh
    }

    /// O Host mudou de forma (alias, autenticação) ou sumiu: a conexão em cache
    /// não fala mais dele.
    pub fn forget_host(&self, host_id: &str) {
        self.by_host.lock().remove(host_id);
    }

    /// Chamado de onde a sessão morre (`SessionManager::dispose`): o canal é
    /// por Host e vive enquanto o app estiver de pé, então o que é por sessão
    /// precisa sair explicitamente.
    pub fn forget_session(&self, session: uuid::Uuid) {
        for q in self.by_host.lock().values() {
            q.forget(session);
        }
    }

    /// Um canal de mentira no mapa, para provar a limpeza sem abrir conexão.
    #[cfg(test)]
    pub(crate) fn track_for_test(&self, host_id: &str, query: Arc<HostQuery>) {
        self.by_host.lock().insert(host_id.to_string(), query);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::remote::fs::{
        ExecOutput, RemoteDirEntry, RemoteError, RemoteResult, RemoteStat,
    };

    type Hold = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

    #[derive(Default)]
    struct FakeRemote {
        calls: Mutex<Vec<Vec<String>>>,
        replies: Mutex<HashMap<String, ExecOutput>>,
        /// Segura a PRIMEIRA consulta dentro do servidor, para que a segunda
        /// encontre a primeira em voo de verdade. Uma só: as consultas
        /// seguintes passam direto.
        hold: Mutex<Option<Hold>>,
    }

    impl FakeRemote {
        fn hold(&self, entrou: std::sync::mpsc::Sender<()>, solta: std::sync::mpsc::Receiver<()>) {
            *self.hold.lock() = Some((entrou, solta));
        }

        fn reply(&self, contains: &str, code: i32, stdout: &[u8]) {
            self.replies.lock().insert(
                contains.to_string(),
                ExecOutput {
                    code,
                    stdout: stdout.to_vec(),
                    stderr: Vec::new(),
                },
            );
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().clone()
        }
    }

    impl RemoteFs for FakeRemote {
        fn host(&self) -> &str {
            "vps"
        }
        fn realpath(&self, _: &str) -> RemoteResult<String> {
            Err(RemoteError::Disconnected)
        }
        fn readdir(&self, _: &str) -> RemoteResult<Vec<RemoteDirEntry>> {
            Err(RemoteError::Disconnected)
        }
        fn lstat(&self, _: &str) -> RemoteResult<RemoteStat> {
            Err(RemoteError::Disconnected)
        }
        fn stat(&self, _: &str) -> RemoteResult<RemoteStat> {
            Err(RemoteError::Disconnected)
        }
        fn read_chunk(&self, _: &str, _: u64, _: usize) -> RemoteResult<Vec<u8>> {
            Err(RemoteError::Disconnected)
        }
        fn write_new(&self, _: &str, _: &[u8], _: u32) -> RemoteResult<()> {
            Err(RemoteError::Disconnected)
        }
        fn posix_rename(&self, _: &str, _: &str) -> RemoteResult<()> {
            Err(RemoteError::Disconnected)
        }
        fn rename(&self, _: &str, _: &str) -> RemoteResult<()> {
            Err(RemoteError::Disconnected)
        }
        fn remove(&self, _: &str) -> RemoteResult<()> {
            Err(RemoteError::Disconnected)
        }
        fn rmdir(&self, _: &str) -> RemoteResult<()> {
            Err(RemoteError::Disconnected)
        }
        fn mkdir(&self, _: &str, _: u32) -> RemoteResult<()> {
            Err(RemoteError::Disconnected)
        }
        fn exec(&self, argv: &[&str]) -> RemoteResult<ExecOutput> {
            let joined = argv.join(" ");
            self.calls
                .lock()
                .push(argv.iter().map(|a| a.to_string()).collect());
            // `take` FORA do `if let`: o guard do mutex sobrevive até o fim do
            // `if let`, e bloquear com ele na mão travaria a outra thread no
            // próprio fake em vez de no portão que se quer medir.
            let hold = self.hold.lock().take();
            if let Some((entrou, solta)) = hold {
                let _ = entrou.send(());
                let _ = solta.recv();
            }
            for (needle, reply) in self.replies.lock().iter() {
                if joined.contains(needle.as_str()) {
                    return Ok(reply.clone());
                }
            }
            Ok(ExecOutput {
                code: 1,
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    struct FakeClock(Mutex<i64>);

    impl FakeClock {
        fn advance(&self, ms: i64) {
            *self.0.lock() += ms;
        }
    }

    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            *self.0.lock()
        }
    }

    fn query(fs: Arc<FakeRemote>, clock: Arc<FakeClock>) -> HostQuery {
        let handle = Arc::clone(&fs);
        HostQuery::new(
            "vps",
            Box::new(move |_| Ok(Arc::clone(&handle) as Arc<dyn RemoteFs>)),
            clock,
        )
    }

    fn session() -> uuid::Uuid {
        uuid::Uuid::from_u128(0x9f3a)
    }

    /// Regra 20: o que o canal pergunta ao servidor **nunca** atravessa o PTY.
    ///
    /// No molde do guarda `nenhum_ssh_nasce_fora_do_construtor`
    /// (`ssh/command.rs`): a garantia é estrutural, e o teste que a defende tem
    /// de ser estrutural também. Uma linha de comando escrita no PTY apareceria
    /// na tela do dono, entraria na captura e no scrollback persistido — e o
    /// nome de um binário do servidor não é saída de comando nenhum.
    ///
    /// A varredura para no `#[cfg(test)]`: daqui para baixo as agulhas
    /// aparecem no texto do próprio teste.
    #[test]
    fn os_nomes_do_servidor_nunca_passam_pelo_pty() {
        let arquivo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("ssh")
            .join("query.rs");
        let fonte = std::fs::read_to_string(&arquivo).unwrap();
        let producao = fonte.split("#[cfg(test)]").next().unwrap();
        assert!(
            producao.contains("fn command_names"),
            "a varredura tem de alcançar o corpo do canal, não só o cabeçalho"
        );
        for agulha in ["PtyPool", "pty_pool", "crate::pty", "PtyId"] {
            assert!(
                !producao.contains(agulha),
                "o canal tocou no PTY (`{agulha}`): o que ele pergunta vai pela \
                 conexão multiplexada, nunca pela tela da sessão"
            );
        }
    }

    #[test]
    fn os_nomes_do_servidor_vem_uma_vez_por_conexao_e_com_teto() {
        let fs = Arc::new(FakeRemote::default());
        let mut listagem = Vec::new();
        for i in 0..(MAX_COMMAND_NAMES + 500) {
            listagem.extend_from_slice(format!("cmd{i}\n").as_bytes());
        }
        // Nome hostil e linha vazia no meio: o que vem do servidor é texto de
        // terceiro, não uma lista confiável.
        listagem.extend_from_slice(b"\nnome com espaco\n../escapa\nok-tool\n");
        fs.reply("for d in $PATH", 0, &listagem);
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        let nomes = q.command_names().unwrap();

        assert_eq!(
            nomes.len(),
            MAX_COMMAND_NAMES,
            "o teto da regra 20 vale sobre o que o servidor devolveu"
        );
        assert!(
            !nomes.iter().any(|n| n.contains(' ') || n.contains('/')),
            "nome com espaço ou barra não é nome de comando: não pode virar sugestão"
        );

        let chamadas = fs.calls().len();
        assert_eq!(q.command_names().unwrap().len(), MAX_COMMAND_NAMES);
        assert_eq!(
            fs.calls().len(),
            chamadas,
            "uma vez por conexão: a segunda pergunta sai do cache"
        );
    }

    /// Regra 13: a pergunta do tmux viaja na MESMA ida que detecta o shell.
    /// Duas idas seriam uma consulta a mais por sessão para uma resposta que
    /// nunca muda dentro da mesma conexão.
    #[test]
    fn o_tmux_e_perguntado_na_mesma_ida_que_detecta_o_shell() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("printf %s \"$SHELL\"", 0, b"/bin/bash\ntmux\n");
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        let sonda = q.host_probe();

        assert_eq!(sonda.shell, crate::ssh::remote_rc::RemoteShell::Bash);
        assert_eq!(sonda.persistence, crate::ssh::Persistence::Persistent);
        assert_eq!(
            fs.calls().len(),
            1,
            "shell e tmux numa ida só: {:?}",
            fs.calls()
        );

        q.host_probe();
        assert_eq!(fs.calls().len(), 1, "a segunda pergunta sai do cache");
    }

    /// Regra 13: servidor sem tmux abre integrado e sem persistência — e é o
    /// canal quem descobre isso ANTES da sessão subir.
    #[test]
    fn host_sem_tmux_responde_sessao_sem_persistencia() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("printf %s \"$SHELL\"", 0, b"/bin/bash\n");
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        let sonda = q.host_probe();

        assert_eq!(
            sonda.shell,
            crate::ssh::remote_rc::RemoteShell::Bash,
            "sem tmux o shell continua servindo: a sessão é integrada"
        );
        assert_eq!(sonda.persistence, crate::ssh::Persistence::Ephemeral);
    }

    #[test]
    fn o_shell_remoto_e_lido_pelo_canal_e_fica_em_cache() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("printf %s \"$SHELL\"", 0, b"/usr/bin/zsh\n");
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        assert_eq!(
            q.host_probe().shell,
            crate::ssh::remote_rc::RemoteShell::Zsh
        );
        let chamadas = fs.calls().len();
        assert_eq!(
            q.host_probe().shell,
            crate::ssh::remote_rc::RemoteShell::Zsh
        );
        assert_eq!(
            fs.calls().len(),
            chamadas,
            "um probe por Host, não por sessão"
        );
    }

    /// Regra 25: sem conexão compartilhada o canal não existe — e o pane não
    /// pode esperar por ele. Ausente é resposta; travar não é.
    #[test]
    fn sem_canal_a_resposta_e_ausente_e_nunca_um_erro_que_trava() {
        let q = HostQuery::new(
            "vps",
            Box::new(|_| Err(AppError::new("ssh.password_needs_session"))),
            Arc::new(FakeClock(Mutex::new(0))),
        );

        assert!(q.git_chips_gated(session(), "/srv").is_none());
        assert!(q.command_names().is_err());
        assert_eq!(
            q.host_probe().shell,
            crate::ssh::remote_rc::RemoteShell::Unsupported("desconhecido".into()),
            "detecção que não aconteceu é sessão comum com motivo, não palpite"
        );
        assert_eq!(
            q.host_probe().persistence,
            crate::ssh::Persistence::Unknown,
            "sem canal ninguém perguntou pelo tmux: o pane não pode afirmar \
             persistência que o core não apurou"
        );
    }

    /// Regra 23: numa sessão SSH os chips são do SERVIDOR. A pasta sai do pane
    /// do tmux remoto, e é dali que o git é perguntado — nunca de um caminho
    /// local que por acaso exista nos dois lados.
    #[test]
    fn os_chips_da_sessao_saem_todos_do_servidor() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("pane_current_path", 0, b"/srv/app\n");
        fs.reply("rev-parse --abbrev-ref HEAD", 0, b"main\n");
        fs.reply("status --porcelain -z", 0, b" M a.rs\0");
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        let chips = q.session_chips(session(), "tyba-a3f-9f3a").unwrap();

        assert_eq!(chips.cwd.as_deref(), Some("/srv/app"));
        assert_eq!(chips.git.branch.as_deref(), Some("main"));
        assert_eq!(chips.git.changed, 1);
        assert!(
            fs.calls()
                .iter()
                .any(|c| c.contains(&"-C".to_string()) && c.contains(&"/srv/app".to_string())),
            "o git tem de ser perguntado na pasta do servidor: {:?}",
            fs.calls()
        );
    }

    /// Regra 24: **uma** consulta em voo por sessão.
    ///
    /// O teto de 2 s não responde por isto — uma consulta lenta (servidor
    /// distante, git grande) dura mais que o teto, e sem a exclusão a segunda
    /// entraria em cima da primeira. O relógio anda muito além do teto de
    /// propósito: a recusa aqui só pode ser da exclusão.
    #[test]
    fn com_uma_consulta_em_voo_a_segunda_da_mesma_sessao_nao_entra() {
        let (entrou_tx, entrou_rx) = std::sync::mpsc::channel();
        let (solta_tx, solta_rx) = std::sync::mpsc::channel();
        let fs = Arc::new(FakeRemote::default());
        fs.reply("rev-parse --abbrev-ref HEAD", 0, b"main\n");
        fs.hold(entrou_tx, solta_rx);
        let clock = Arc::new(FakeClock(Mutex::new(0)));
        let q = Arc::new(query(Arc::clone(&fs), Arc::clone(&clock)));

        let primeira = {
            let q = Arc::clone(&q);
            std::thread::spawn(move || q.git_chips_gated(session(), "/srv").is_some())
        };
        entrou_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a primeira consulta tem de chegar ao servidor");

        clock.advance(60_000);
        assert!(
            q.git_chips_gated(session(), "/srv").is_none(),
            "a segunda consulta da MESMA sessão entrou com a primeira em voo"
        );
        assert!(
            q.git_chips_gated(uuid::Uuid::from_u128(0xbeef), "/srv")
                .is_some(),
            "a exclusão é por sessão: a consulta de uma não pode calar a de outra"
        );

        let _ = solta_tx.send(());
        assert!(primeira.join().unwrap(), "a primeira consulta respondeu");

        clock.advance(60_000);
        assert!(
            q.git_chips_gated(session(), "/srv").is_some(),
            "terminada a consulta, a sessão volta a poder perguntar"
        );
    }

    #[test]
    fn os_chips_tambem_obedecem_ao_teto_de_dois_segundos() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("pane_current_path", 0, b"/srv/app\n");
        let clock = Arc::new(FakeClock(Mutex::new(0)));
        let q = query(Arc::clone(&fs), Arc::clone(&clock));

        assert!(q.session_chips(session(), "tyba-a3f-9f3a").is_some());
        clock.advance(1_999);
        assert!(q.session_chips(session(), "tyba-a3f-9f3a").is_none());
    }

    #[test]
    fn o_git_dos_chips_le_branch_e_contagem_do_servidor() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("rev-parse --abbrev-ref HEAD", 0, b"feat/ssh\n");
        fs.reply(
            "status --porcelain -z",
            0,
            b"R  novo name.rs\0antigo.rs\0?? outro.txt\0 M src/keep.rs\0",
        );
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        let chips = q.git_chips("/srv/app").unwrap();

        assert_eq!(chips.branch.as_deref(), Some("feat/ssh"));
        assert_eq!(chips.changed, 3, "o rename conta uma vez, não duas");
        let argv = fs.calls();
        assert!(
            argv.iter()
                .all(|c| c.contains(&"core.quotePath=false".to_string())
                    && c.contains(&"color.ui=false".to_string())),
            "princípio #8 vale igual no git remoto: {argv:?}"
        );
    }

    #[test]
    fn head_solto_nao_vira_uma_branch_chamada_head() {
        let fs = Arc::new(FakeRemote::default());
        fs.reply("rev-parse --abbrev-ref HEAD", 0, b"HEAD\n");
        let q = query(Arc::clone(&fs), Arc::new(FakeClock(Mutex::new(0))));

        assert_eq!(q.git_chips("/srv/app").unwrap().branch, None);
    }

    #[test]
    fn o_canal_nao_pergunta_duas_vezes_dentro_de_dois_segundos() {
        let fs = Arc::new(FakeRemote::default());
        let clock = Arc::new(FakeClock(Mutex::new(10_000)));
        let q = query(Arc::clone(&fs), Arc::clone(&clock));

        assert!(q.git_chips_gated(session(), "/srv/app").is_some());
        let depois_da_primeira = fs.calls().len();

        // Números literais, e não `MIN_INTERVAL_MS`: um teste que se mede pela
        // própria constante passa com ela valendo zero — ele estaria medindo a
        // si mesmo. Verificado: com o teto em 0 esta versão FALHA, como deve.
        clock.advance(1_999);
        assert!(
            q.git_chips_gated(session(), "/srv/app").is_none(),
            "dentro do teto a resposta é 'agora não' — o chip segue com o que tinha"
        );
        assert_eq!(
            fs.calls().len(),
            depois_da_primeira,
            "recusada pelo teto, a consulta não pode ter tocado no servidor"
        );

        clock.advance(1);
        assert!(
            q.git_chips_gated(session(), "/srv/app").is_some(),
            "passados os 2 s o canal volta a perguntar"
        );
    }
}
