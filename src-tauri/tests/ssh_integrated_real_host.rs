//! As costuras da SSH Session INTEGRADA contra um host REAL, pela API pública.
//!
//! Todos os casos são `#[ignore]` e conectam de verdade num host alcançável:
//! rodam só com `cargo test --test ssh_integrated_real_host -- --ignored` e com
//! o ambiente abaixo. Sem ele, cada caso avisa no stderr e sai cedo, sem falhar.
//!
//! - `TYBA_E2E_SSH_ALIAS` — alias do `~/.ssh/config` que alcança o host. É por
//!   ele, e só por ele, que o canal próprio (SFTP/`exec`) conecta, exatamente
//!   como o app faz.
//! - `TYBA_E2E_SSH_USER` — usuário válido no host, usado nas conexões que este
//!   teste monta à mão.
//! - `TYBA_E2E_SSH_IDENTITY_AGENT` (opcional) — socket do agente, passado como
//!   `IdentityAgent` nas conexões montadas aqui.
//!
//! `TYBA_E2E_SSH_HOSTNAME` e `TYBA_E2E_SSH_AGENT_KEY_NAME` — que o irmão
//! `ssh_real_host.rs` exige — não entram aqui: nenhum caso deste arquivo monta
//! formulário de Host nem escolhe chave do agente. Exigi-las seria pedir o que
//! não se lê.
//!
//! O host precisa de **bash** e de **tmux** para os casos 1, 2 e 6 (é a sessão
//! integrada com persistência da regra 1); o caso 4 pula sozinho se não houver
//! `git` no servidor, e o caso 5 pula sozinho se não houver `zsh`.
//!
//! O agente pode pedir aprovação na tela (o do 1Password pede): o dono tem de
//! estar presente, e uma aprovação por conexão. Rodar com `--test-threads=1`
//! deixa os pedidos em fila em vez de todos ao mesmo tempo.
//!
//! Nenhum endereço, usuário ou chave real mora neste arquivo — tudo vem do
//! ambiente. Tudo que o teste cria no servidor (sessão tmux, pasta do repo de
//! teste) é destruído por guarda `Drop`, pânico incluído.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tyba_lib::completion;
use tyba_lib::error::AppError;
use tyba_lib::files::remote::build_panel;
use tyba_lib::files::remote::fs::RemoteFs;
use tyba_lib::files::remote::sftpwire::SshRemote;
use tyba_lib::pty::tmux_control::{
    capture_pane, refresh_client, send_keys, ControlDecoder, ControlEvent, ControlLink,
    CAPTURE_LINES,
};
use tyba_lib::session::cano::CanoWatch;
use tyba_lib::ssh::query::{HostQuery, SystemClock, MAX_COMMAND_NAMES};
use tyba_lib::ssh::remote_rc::{self, remote_command, RemoteShell};
use tyba_lib::ssh::tmux::{self, Probe};
use tyba_lib::ssh::{IntegrationPlan, IntegrationReason, IntegrationState, Persistence};

/// Prefixo das sessões tmux deste arquivo. NUNCA o `tyba-<install_id>-` do app:
/// a coleta de órfãos do core mata o que casa com o dela, e uma sessão de teste
/// no meio do caminho seria uma sessão do dono morrendo por engano.
const PREFIXO: &str = "tyba-e2e05";

/// Prazo para a sessão integrada subir: autenticação, `sh -c` remoto, rc
/// materializado, tmux de pé e primeiro prompt.
const SUBIDA: Duration = Duration::from_secs(60);
/// Prazo de uma ida e volta já com a sessão de pé.
const RESPOSTA: Duration = Duration::from_secs(30);

const COLS: u16 = 100;
const ROWS: u16 = 30;

struct E2eEnv {
    alias: String,
    user: String,
    identity_agent: Option<String>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn e2e_env(case: &str) -> Option<E2eEnv> {
    let found = (|| {
        Some(E2eEnv {
            alias: var("TYBA_E2E_SSH_ALIAS")?,
            user: var("TYBA_E2E_SSH_USER")?,
            identity_agent: var("TYBA_E2E_SSH_IDENTITY_AGENT"),
        })
    })();
    if found.is_none() {
        eprintln!(
            "{case}: pulado — defina TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER para \
             conectar num host real"
        );
    }
    found
}

fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// O `ssh` do core com as opções do Cano. `flags` entram antes do destino:
/// depois dele o `ssh` as leria como comando remoto.
fn ssh_to(env: &E2eEnv, flags: &[&str]) -> Command {
    let mut cmd = tyba_lib::ssh::command::std_command();
    cmd.args(flags);
    for opt in ["BatchMode=yes", "ConnectTimeout=15"] {
        cmd.arg("-o").arg(opt);
    }
    if let Some(agent) = &env.identity_agent {
        cmd.arg("-o").arg(format!("IdentityAgent=\"{agent}\""));
    }
    cmd.args(["-l", &env.user, &env.alias]);
    cmd
}

/// Um comando curto no host, fora do PTY: é o caminho INDEPENDENTE do que os
/// casos medem — conferir o servidor pela mesma peça que está sob teste não
/// provaria nada.
fn ssh_out(env: &E2eEnv, script: &str) -> Option<std::process::Output> {
    ssh_to(env, &[])
        .arg(script)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()
}

/// O servidor tem este programa no PATH?
///
/// A sessão integrada com `RemoteShell::Zsh` executa `zsh` no pane — e não o
/// shell de login do dono —, então sem o binário lá o pane morreria na partida.
/// Mesmo desenho do `command -v git` do caso 4: host que não tem, pula.
fn tem_programa(env: &E2eEnv, programa: &str) -> bool {
    ssh_out(env, &format!("command -v {programa} >/dev/null 2>&1"))
        .is_some_and(|out| out.status.success())
}

fn remote_kill(env: &E2eEnv, name: &str) {
    let _ = ssh_to(env, &[])
        .arg(tmux::kill_command(name))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// A sonda do core, mas por uma conexão com o `-l <usuário>` deste teste: o
/// `tmux::probe` do app usa só o alias, e se o usuário do teste não for o do
/// `~/.ssh/config` ele responderia `Unknown` sobre um host que está de pé.
fn remote_probe(env: &E2eEnv, name: &str) -> Probe {
    let status = ssh_to(env, &[])
        .arg(tmux::has_session_command(name))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(s) => tmux::interpret_has_session(s.code()),
        Err(_) => Probe::Unknown,
    }
}

/// Mata a sessão tmux do teste no host em qualquer caminho, pânico incluído.
struct RemoteSession<'a> {
    env: &'a E2eEnv,
    name: String,
}

impl Drop for RemoteSession<'_> {
    fn drop(&mut self) {
        remote_kill(self.env, &self.name);
    }
}

/// Uma pasta que o teste cria no servidor (casos 4 e 7), apagada em qualquer
/// caminho.
struct RemoteDir<'a> {
    env: &'a E2eEnv,
    path: String,
}

impl Drop for RemoteDir<'_> {
    fn drop(&mut self) {
        // `rm -rf` remoto só do que ESTE teste criou: o caminho tem de ser o do
        // próprio molde de `mktemp`, nunca o que o servidor devolveu por acaso.
        if !caminho_do_teste(&self.path) {
            return;
        }
        let _ = ssh_out(self.env, &format!("rm -rf {}", self.path));
    }
}

/// O caminho veio de um molde de `mktemp` DESTE arquivo (`-git-`, `-comp-`)?
/// O prefixo é o namespace do arquivo inteiro, e não o de um caso: é ele que
/// garante que o `rm -rf` remoto nunca alcance pasta de terceiro no `/tmp` do
/// dono.
fn caminho_do_teste(path: &str) -> bool {
    path.starts_with("/tmp/tyba-e2e05-")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
}

/// De qual descritor veio o pedaço.
///
/// > [!warning] Os dois NÃO podem ser misturados num buffer só.
/// > No app o Cano é um PTY: stdout e stderr do `ssh` são o mesmo descritor e
/// > chegam em ordem. Aqui são dois canos e duas threads, e um aviso do `ssh`
/// > lido no meio de uma leitura do stdout entraria ENTRE as duas metades do
/// > marco de controle partido — o decodificador soltaria a metade retida como
/// > byte cru e o marco nunca mais apareceria. O protocolo só sai pelo stdout;
/// > o stderr é diagnóstico.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fonte {
    Saida,
    Erro,
}

/// Cano de teste: o `ssh` num grupo próprio, a saída num canal etiquetado pela
/// fonte, e o stdin aberto para escrever no cliente de controle.
struct Cano {
    child: Child,
    stdin: Option<ChildStdin>,
    output: mpsc::Receiver<(Fonte, Vec<u8>)>,
    /// Já foi morto e esperado. Sem isto o `Drop` mandaria `killpg` num grupo
    /// cujo pid já foi colhido — e um pid recém-reusado levaria o sinal.
    morto: bool,
}

impl Cano {
    fn spawn(mut cmd: Command) -> Cano {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = cmd.spawn().expect("o ssh sobe");
        let (tx, output) = mpsc::channel();
        for (fonte, stream) in [
            (
                Fonte::Saida,
                child
                    .stdout
                    .take()
                    .map(|s| Box::new(s) as Box<dyn Read + Send>),
            ),
            (
                Fonte::Erro,
                child
                    .stderr
                    .take()
                    .map(|s| Box::new(s) as Box<dyn Read + Send>),
            ),
        ]
        .into_iter()
        .filter_map(|(fonte, stream)| Some((fonte, stream?)))
        {
            let tx = tx.clone();
            std::thread::spawn(move || pump(fonte, stream, tx));
        }
        let stdin = child.stdin.take();
        Cano {
            child,
            stdin,
            output,
            morto: false,
        }
    }

    fn kill(&mut self) {
        if self.morto {
            return;
        }
        self.morto = true;
        if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
            // SAFETY: sinal para o grupo que este teste criou com `process_group(0)`.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Cano {
    fn drop(&mut self) {
        self.kill();
    }
}

fn pump(fonte: Fonte, mut stream: Box<dyn Read + Send>, tx: mpsc::Sender<(Fonte, Vec<u8>)>) {
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if tx.send((fonte, buf[..n].to_vec())).is_err() {
                    return;
                }
            }
        }
    }
}

/// A sessão integrada vista pelas peças públicas do core: o `ssh` do
/// `ssh::command`, o comando remoto do `remote_rc`, o decodificador do
/// `tmux_control` e o observador de login do Cano.
struct Wire<'a> {
    cano: Cano,
    remota: RemoteSession<'a>,
    decoder: ControlDecoder,
    link: ControlLink,
    watch: CanoWatch,
    nonce: String,
    logado: bool,
    handshake: bool,
    /// O que saiu ANTES do marco de controle — byte cru do ssh.
    raw: Vec<u8>,
    /// O que saiu DEPOIS dele, já desembrulhado de `%output`.
    screen: Vec<u8>,
    /// O stderr do `ssh` local, só para diagnóstico.
    erro: Vec<u8>,
    estranhas: Vec<String>,
    saiu: bool,
}

/// O `ssh` de uma sessão integrada: o comando remoto do `remote_rc` para
/// `shell`, sobre a sessão tmux `name`.
///
/// É o MESMO comando na subida e no reatar — `remote_command` não tem parâmetro
/// que distinga os dois. Quem decide é o servidor: o ramo de anexar pergunta
/// `has-session` antes de escrever qualquer coisa, e só cria quando não há
/// sessão.
fn cano_da_sessao(env: &E2eEnv, shell: RemoteShell, name: &str, nonce: &str) -> Cano {
    let mut cmd = ssh_to(env, &["-tt"]);
    // O comando remoto faz `exec tmux -C`, que exige terminal; o TERM é o que o
    // PTY do app entrega, e não o do processo que roda o `cargo test`.
    cmd.env("TERM", "xterm-256color")
        .arg(remote_command(shell, nonce, name, true));
    Cano::spawn(cmd)
}

fn sessao_integrada<'a>(env: &'a E2eEnv, shell: RemoteShell, sufixo: &str) -> Wire<'a> {
    let nonce = nonce();
    let name = format!("{PREFIXO}-{sufixo}-{}", &nonce[..8]);
    let cano = cano_da_sessao(env, shell, &name, &nonce);
    let decoder = ControlDecoder::with_marker(Some(&remote_rc::control_marker(&nonce)));
    let link = decoder.link();
    Wire {
        cano,
        remota: RemoteSession {
            env,
            name: name.clone(),
        },
        decoder,
        link,
        watch: CanoWatch::new(&nonce),
        nonce,
        logado: false,
        handshake: false,
        raw: Vec::new(),
        screen: Vec::new(),
        erro: Vec::new(),
        estranhas: Vec::new(),
        saiu: false,
    }
}

impl<'a> Wire<'a> {
    fn alvo(&self) -> String {
        self.remota.name.clone()
    }

    /// Reata: derruba o `ssh` LOCAL e liga outro sobre a MESMA sessão do
    /// servidor — que é justamente o que tem de sobreviver.
    ///
    /// Pelo caminho de produção: `SessionManager::spawn_ssh` monta um
    /// **nonce novo a cada spawn** (um marco que sobrou de outro Cano não
    /// conclui o login deste) e chama o mesmo `remote_command`. A guarda
    /// `RemoteSession` fica de pé entre os dois Canos de propósito: deixá-la
    /// cair mataria no servidor a sessão que este caso existe para reatar.
    ///
    /// O que já chegou é zerado aqui: depois disto, tudo que houver em `screen`
    /// veio DESTA conexão — sem isso, o redesenho do `capture-pane` não se
    /// distinguiria do que a conexão anterior já tinha mostrado.
    fn reata(&mut self, shell: RemoteShell) {
        // Morre antes de o próximo subir: dois clientes de controle vivos na
        // mesma sessão fariam o tmux encolher o pane para o menor deles.
        self.cano.kill();
        let nonce = nonce();
        self.cano = cano_da_sessao(self.remota.env, shell, &self.remota.name, &nonce);
        self.decoder = ControlDecoder::with_marker(Some(&remote_rc::control_marker(&nonce)));
        self.link = self.decoder.link();
        self.watch = CanoWatch::new(&nonce);
        self.nonce = nonce;
        self.logado = false;
        self.handshake = false;
        self.raw.clear();
        self.screen.clear();
        self.erro.clear();
        self.estranhas.clear();
        self.saiu = false;
    }

    fn absorve(&mut self, chunk: &[u8]) {
        if self.watch.feed(chunk) {
            self.logado = true;
        }
        if self.link.in_control() {
            let eventos = self.decoder.feed(chunk);
            self.roteia(false, eventos);
        } else {
            // Byte a byte só na fase crua (uns poucos KiB): é o que dá o ponto
            // EXATO em que o marco troca o transporte. Alimentar o pedaço
            // inteiro juntaria banner e protocolo no mesmo lote de eventos, e o
            // caso 1 não poderia afirmar o que veio ANTES do marco.
            for &byte in chunk {
                let cru = !self.link.in_control();
                let eventos = self.decoder.feed(&[byte]);
                self.roteia(cru, eventos);
            }
        }
        if !self.handshake && self.link.in_control() {
            self.handshake = true;
            // O mesmo aperto de mão do core: cliente de modo de controle não
            // tem tamanho vindo do tty, ele declara o seu.
            let linha = refresh_client(COLS, ROWS);
            self.manda(&linha);
        }
    }

    fn roteia(&mut self, cru: bool, eventos: Vec<ControlEvent>) {
        for evento in eventos {
            match evento {
                ControlEvent::Output(bytes) if cru => self.raw.extend_from_slice(&bytes),
                ControlEvent::Output(bytes) => self.screen.extend_from_slice(&bytes),
                ControlEvent::Exit => self.saiu = true,
                ControlEvent::Notification(linha) | ControlEvent::Unparsed(linha) => {
                    if self.estranhas.len() < 40 {
                        self.estranhas.push(linha);
                    }
                }
            }
        }
    }

    /// Uma linha de comando para o cliente de controle.
    fn manda(&mut self, comando: &str) {
        if let Some(stdin) = self.cano.stdin.as_mut() {
            let _ = stdin.write_all(format!("{comando}\n").as_bytes());
            let _ = stdin.flush();
        }
    }

    /// Lê até `pred` valer sobre o que já chegou, ou até o prazo.
    fn ate(&mut self, limite: Duration, pred: impl Fn(&Wire<'a>) -> bool) -> bool {
        if pred(self) {
            return true;
        }
        let prazo = Instant::now() + limite;
        loop {
            let falta = prazo.saturating_duration_since(Instant::now());
            if falta.is_zero() {
                return false;
            }
            let Ok((fonte, chunk)) = self.cano.output.recv_timeout(falta) else {
                return false;
            };
            match fonte {
                Fonte::Saida => self.absorve(&chunk),
                Fonte::Erro => self.erro.extend_from_slice(&chunk),
            }
            if pred(self) {
                return true;
            }
        }
    }

    /// O shell remoto chegou ao prompt: o rc emitiu os sinais que o TYBA local
    /// também emite (regra 10).
    fn ate_o_prompt(&mut self) -> bool {
        self.ate(SUBIDA, |w| {
            contem(&w.screen, b"\x1b]133;A\x07") && contem(&w.screen, b"\x1b]633;P;tyba-prompt=")
        })
    }

    /// Digita no shell remoto pelo protocolo — nunca byte cru no PTY.
    fn digita(&mut self, texto: &str) {
        let comando = send_keys(&self.alvo(), texto.as_bytes());
        self.manda(&comando);
    }

    fn diag(&self) -> String {
        format!(
            "logado={} controle={} saiu={} cru=…{} tela=…{} stderr=…{} estranhas={:?}",
            self.logado,
            self.link.in_control(),
            self.saiu,
            cauda(&self.raw),
            cauda(&self.screen),
            cauda(&self.erro),
            self.estranhas,
        )
    }
}

fn cauda(bytes: &[u8]) -> String {
    let de = bytes.len().saturating_sub(400);
    String::from_utf8_lossy(&bytes[de..])
        .escape_debug()
        .to_string()
}

fn contem(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len().max(1))
        .any(|janela| janela == needle)
}

// ---------------------------------------------------------------------------
// Caso 1 — a sessão integrada sobe e fala o protocolo (regras 1, 5, 10)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn a_sessao_integrada_sobe_e_fala_o_protocolo_de_controle() {
    let Some(env) = e2e_env("sessao_integrada") else {
        return;
    };
    let mut wire = sessao_integrada(&env, RemoteShell::Bash, "proto");

    // (a) o trecho cru traz o marco de login, inteiro e sem passar pelo protocolo.
    let subiu = wire.ate_o_prompt();
    assert!(
        wire.logado,
        "o marco de login não chegou — o host precisa de bash e tmux. {}",
        wire.diag()
    );
    assert!(
        contem(&wire.raw, &tmux::login_marker(&wire.nonce)),
        "o marco de login tem de atravessar a fase crua tal como o ssh o escreveu. {}",
        wire.diag()
    );
    assert!(
        !contem(&wire.raw, b"tyba-ctl=") && !contem(&wire.screen, b"tyba-ctl="),
        "o marco de controle é consumido pelo transporte e nunca chega à tela. {}",
        wire.diag()
    );

    // (b) depois do marco vêm `%output` com os sinais do rc.
    assert!(
        subiu,
        "os sinais do rc remoto não chegaram em {SUBIDA:?}. {}",
        wire.diag()
    );
    assert!(
        wire.link.in_control(),
        "passado o marco, o transporte é o protocolo. {}",
        wire.diag()
    );
    assert!(
        contem(&wire.screen, b"\x1b]7;file://"),
        "o rc remoto anuncia a pasta por OSC 7 (regra 10). {}",
        wire.diag()
    );

    // (c) `send-keys` chega ao shell e a saída volta por `%output`.
    // O token só existe na SAÍDA do `printf`: a linha digitada carrega `%s`, e
    // eco de comando não pode passar por prova de que o comando rodou.
    let semente = nonce()[..8].to_string();
    let token = format!("TYBA{semente}OK");
    let antes = wire.screen.len();
    wire.digita(&format!("printf 'TYBA%sOK\\n' {semente}\r"));
    // Espera-se pelo FIM do bloco, não pelo token: o `133;D` do rc só sai no
    // prompt seguinte, então quem espera só a saída pode conferir o código de
    // saída antes de ele existir — e o teste passaria ou falharia por corrida.
    let fechou = wire.ate(RESPOSTA, |w| {
        contem(&w.screen[antes..], b"\x1b]133;D;0\x07")
    });
    assert!(
        fechou,
        "o bloco do comando digitado tinha de fechar com o código de saída do \
         servidor. {}",
        wire.diag()
    );
    assert!(
        contem(&wire.screen[antes..], token.as_bytes()),
        "a saída do printf remoto tinha de voltar por %output. {}",
        wire.diag()
    );

    // (d) `capture-pane` redesenha o que já estava na tela — é o reatar da regra 5.
    let antes_da_captura = wire.screen.len();
    wire.link.arm_capture();
    let captura = capture_pane(&wire.alvo(), CAPTURE_LINES);
    wire.manda(&captura);
    let redesenhou = wire.ate(RESPOSTA, |w| {
        contem(&w.screen[antes_da_captura..], token.as_bytes())
    });
    assert!(
        redesenhou,
        "reatar redesenha do capture-pane sem rodar nada de novo. {}",
        wire.diag()
    );
    assert!(
        !wire.saiu,
        "a sessão segue viva depois de tudo isso. {}",
        wire.diag()
    );

    // Encerrado à mão para conferir que a limpeza do teste funciona — a guarda
    // `Drop` roda de novo logo abaixo e é idempotente.
    wire.cano.kill();
    remote_kill(&env, &wire.remota.name);
    assert!(
        matches!(
            remote_probe(&env, &wire.remota.name),
            Probe::Gone | Probe::NoTmux
        ),
        "a sessão tmux {} ficou viva no host",
        wire.remota.name
    );
}

// ---------------------------------------------------------------------------
// Caso 2 — o rc não deixa rastro no servidor (regra 9)
// ---------------------------------------------------------------------------

/// A pasta do comando remoto é `${XDG_RUNTIME_DIR:-/tmp}/tyba-<token>`, e o
/// token é sorteado pelo TYBA — não tem tamanho fixo. O glob é `tyba-*` de
/// propósito: `tyba-??????` era o molde do `mktemp`, e quando o nome mudou o
/// glob deixou de casar qualquer coisa, o que fazia esta conferência passar sem
/// ter medido nada. Casa também as pastas do formato antigo.
/// A conferência olha os dois caminhos porque o `XDG_RUNTIME_DIR` de um `exec`
/// não interativo pode não ser o da sessão de login.
const LISTA_RASTRO: &str = "for d in \"${XDG_RUNTIME_DIR:-/tmp}\" /tmp; do \
                            ls -1d \"$d\"/tyba-* 2>/dev/null; done; true";

/// As pastas do molde do rc que existem no servidor AGORA, pelo caminho de
/// fora. Falhar em perguntar não pode virar lista vazia: isso deixaria a
/// asserção passar sem ter medido nada.
fn pastas_do_tyba(env: &E2eEnv) -> Vec<String> {
    let listagem = ssh_out(env, LISTA_RASTRO).expect("o ssh de conferência roda");
    String::from_utf8_lossy(&listagem.stdout)
        .lines()
        .map(str::trim)
        .filter(|linha| !linha.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn o_rc_remoto_nao_deixa_rastro_depois_que_a_sessao_sobe() {
    let Some(env) = e2e_env("rc_sem_rastro") else {
        return;
    };
    let mut wire = sessao_integrada(&env, RemoteShell::Bash, "rastro");
    assert!(
        wire.ate_o_prompt(),
        "a sessão integrada não chegou ao prompt em {SUBIDA:?}. {}",
        wire.diag()
    );

    // O próprio shell remoto responde primeiro: a variável que apontava para a
    // pasta foi desfeita pelo rc logo depois de ele ser lido. A linha digitada
    // carrega `%s`, então o eco dela não se confunde com a resposta.
    let antes = wire.screen.len();
    wire.digita("printf 'TYBA_RC=[%s]\\n' \"$TYBA_RC_DIR\"\r");
    let respondeu = wire.ate(RESPOSTA, |w| contem(&w.screen[antes..], b"TYBA_RC=[]"));
    assert!(
        respondeu,
        "regra 9: depois de lido, o rc desfaz TYBA_RC_DIR no shell remoto. {}",
        wire.diag()
    );

    // E o servidor confirma pelo caminho de fora: nenhuma pasta do TYBA ficou.
    let achados = pastas_do_tyba(&env);
    assert!(
        achados.is_empty(),
        "regra 9: nada do TYBA pode ficar no servidor depois que a sessão sobe; \
         sobrou {achados:?}"
    );
}

// ---------------------------------------------------------------------------
// Caso 3 — a sonda do host (regras 8 e 13)
// ---------------------------------------------------------------------------

/// O canal próprio do Host, ligado como o app o liga: pelo alias do
/// `~/.ssh/config`, por cima da conexão multiplexada, sem `ssh` novo de PTY.
fn host_query(env: &E2eEnv) -> HostQuery {
    HostQuery::new(
        &env.alias,
        Box::new(|alias| {
            SshRemote::connect(alias)
                .map(|fs| Arc::new(fs) as Arc<dyn RemoteFs>)
                .map_err(|e| AppError::new("ssh.query_failed").with("detail", e.message()))
        }),
        Arc::new(SystemClock),
    )
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn a_sonda_do_host_devolve_shell_e_persistencia_e_o_plano_sai_integrado() {
    let Some(env) = e2e_env("sonda_do_host") else {
        return;
    };
    let q = host_query(&env);

    let sonda = q.host_probe();

    assert!(
        sonda.shell.integrable(),
        "regra 8: o shell de login do servidor tem de ser bash ou zsh para a \
         sessão nascer integrada; veio {:?}",
        sonda.shell.label()
    );

    // A persistência é conferida por um caminho INDEPENDENTE do canal: medir a
    // sonda com a própria sonda não diria nada sobre o servidor.
    let tmux_ali = remote_probe(&env, &format!("{PREFIXO}-inexistente"));
    assert_ne!(
        tmux_ali,
        Probe::Unknown,
        "a conferência independente do tmux não respondeu: sem ela não dá para \
         dizer o que a sonda deveria ter achado"
    );
    let esperada = if tmux_ali == Probe::NoTmux {
        Persistence::Ephemeral
    } else {
        Persistence::Persistent
    };
    assert_eq!(
        sonda.persistence, esperada,
        "regra 13: a persistência sai da mesma resposta que traz o shell"
    );

    assert_eq!(
        q.host_probe(),
        sonda,
        "a segunda pergunta sai do cache por Host, não de outra ida ao servidor"
    );

    let plano = IntegrationPlan::from_probe(true, sonda.clone());
    assert_eq!(plano.integration.state, IntegrationState::Integrated);
    assert_eq!(plano.integration.reason, IntegrationReason::Ok);
    assert_eq!(plano.integration.persistence, esperada);
    assert_eq!(
        plano.shell, sonda.shell,
        "o plano leva o shell que a sonda apurou: é ele que monta o rc remoto"
    );
}

// ---------------------------------------------------------------------------
// Caso 4 — chips e nomes de comando do servidor (regras 20 e 24)
// ---------------------------------------------------------------------------

const BRANCH_DO_TESTE: &str = "tyba-e2e05-branch";

/// Um repo git descartável no servidor, com branch e duas alterações
/// conhecidas. Montado por um `ssh` próprio, e não pelo canal: conferir o canal
/// com o próprio canal não mediria o servidor.
fn repo_de_teste(env: &E2eEnv) -> Option<RemoteDir<'_>> {
    let script = format!(
        "command -v git >/dev/null 2>&1 || exit 3; \
         d=$(mktemp -d /tmp/tyba-e2e05-git-XXXXXX) || exit 1; \
         cd \"$d\" || exit 1; \
         git init -q . >/dev/null 2>&1 || exit 1; \
         git -c user.email=e2e@tyba.invalid -c user.name=tyba commit -q \
             --allow-empty -m base >/dev/null 2>&1 || exit 1; \
         git checkout -q -b {BRANCH_DO_TESTE} >/dev/null 2>&1 || exit 1; \
         : > a.txt; : > b.txt; \
         printf %s \"$d\""
    );
    let out = ssh_out(env, &script).expect("o ssh que monta o repo de teste roda");
    if out.status.code() == Some(3) {
        eprintln!("chips_e_nomes: parte do git pulada — o servidor não tem git");
        return None;
    }
    // A guarda nasce ANTES de qualquer asserção sobre o caminho: daqui em diante
    // um pânico ainda apaga do servidor o que este teste criou lá.
    let guarda = RemoteDir {
        env,
        path: String::from_utf8_lossy(&out.stdout).trim().to_string(),
    };
    assert!(
        out.status.success(),
        "não deu para montar o repo de teste no servidor: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        caminho_do_teste(&guarda.path),
        "o mktemp do servidor devolveu um caminho fora do molde do teste: {:?}",
        guarda.path
    );
    Some(guarda)
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn o_canal_devolve_os_nomes_de_comando_e_o_git_do_servidor() {
    let Some(env) = e2e_env("chips_e_nomes") else {
        return;
    };
    let q = host_query(&env);

    let nomes = q
        .command_names()
        .expect("o canal responde os nomes de comando do servidor");

    assert!(
        !nomes.is_empty(),
        "o PATH do servidor não pode voltar vazio"
    );
    assert!(
        nomes.len() <= MAX_COMMAND_NAMES,
        "regra 20: o teto vale sobre o que o servidor devolveu, veio {}",
        nomes.len()
    );
    assert!(
        nomes.iter().any(|nome| nome == "sh"),
        "todo servidor POSIX tem `sh` no PATH; vieram {} nomes",
        nomes.len()
    );
    assert!(
        !nomes
            .iter()
            .any(|nome| nome.contains('/') || nome.chars().any(char::is_whitespace)),
        "o que vem do servidor é texto de terceiro e vira sugestão de Tab: \
         nada de caminho nem de espaço"
    );

    let Some(guarda) = repo_de_teste(&env) else {
        return;
    };

    let chips = q
        .git_chips(&guarda.path)
        .expect("o canal responde o git do servidor");

    assert_eq!(
        chips.branch.as_deref(),
        Some(BRANCH_DO_TESTE),
        "regra 24: a branch é a do SERVIDOR, na pasta que o teste criou lá"
    );
    assert_eq!(
        chips.changed, 2,
        "duas alterações no repo que o teste montou no servidor"
    );
}

// ---------------------------------------------------------------------------
// Caso 5 — a mesma sessão integrada, com zsh (regras 1, 8, 10, 14)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn a_sessao_integrada_com_zsh_sobe_e_fala_o_protocolo_de_controle() {
    let Some(env) = e2e_env("sessao_zsh") else {
        return;
    };
    if !tem_programa(&env, "zsh") {
        eprintln!("sessao_zsh: pulado — o servidor não tem zsh no PATH");
        return;
    }
    let mut wire = sessao_integrada(&env, RemoteShell::Zsh, "zsh");

    let subiu = wire.ate_o_prompt();
    assert!(
        wire.logado,
        "o marco de login não chegou — o host precisa de zsh e tmux. {}",
        wire.diag()
    );
    assert!(
        subiu,
        "regra 8: com zsh o rc remoto emite os mesmos sinais do bash, e eles não \
         chegaram em {SUBIDA:?}. {}",
        wire.diag()
    );
    assert!(
        wire.link.in_control(),
        "passado o marco, o transporte é o protocolo. {}",
        wire.diag()
    );
    assert!(
        contem(&wire.screen, b"\x1b]7;file://"),
        "o rc do zsh remoto anuncia a pasta por OSC 7 (regra 10). {}",
        wire.diag()
    );

    // Quem está do outro lado é o zsh, e não o shell de login do dono:
    // `ZSH_VERSION` só existe dentro do zsh. A linha digitada carrega `%s` e o
    // nome da variável, então `ZSH=[nao]` na tela seria o bash respondendo —
    // eco de comando não se confunde com resposta.
    let antes = wire.screen.len();
    wire.digita("printf 'ZSH=[%s]\\n' \"${ZSH_VERSION:-nao}\"\r");
    // Pelo FIM do bloco, não pela saída: o `133;D` do rc só sai no prompt
    // seguinte, e quem espera só o texto confere o código de saída antes de ele
    // existir.
    let fechou = wire.ate(RESPOSTA, |w| {
        contem(&w.screen[antes..], b"\x1b]133;D;0\x07")
    });
    assert!(
        fechou,
        "regra 14: o bloco do comando digitado no zsh remoto tinha de fechar com \
         o código de saída do servidor. {}",
        wire.diag()
    );
    assert!(
        contem(&wire.screen[antes..], b"ZSH=["),
        "a saída do printf remoto tinha de voltar por %output. {}",
        wire.diag()
    );
    assert!(
        !contem(&wire.screen[antes..], b"ZSH=[nao]"),
        "o pane da sessão com RemoteShell::Zsh tem de ser zsh, não o shell de \
         login do host. {}",
        wire.diag()
    );
    assert!(
        !wire.saiu,
        "a sessão segue viva depois de tudo isso. {}",
        wire.diag()
    );
}

// ---------------------------------------------------------------------------
// Caso 6 — reatar não refaz nada nem deixa pasta no servidor (regras 2, 5, 9)
// ---------------------------------------------------------------------------

/// Duas voltas, e não uma: o defeito que o bloco 2 corrigiu — `new-session -A`
/// anexando sem que ninguém leia o rc, e a pasta ficando no servidor — se
/// acumulava por reatar. Medido na VPS do dono: 12 pastas em `/run/user/0`.
const VOLTAS: u32 = 2;

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn reatar_uma_sessao_viva_redesenha_sem_reexecutar_e_sem_deixar_pasta() {
    let Some(env) = e2e_env("reatar") else {
        return;
    };

    // Linha de base ANTES de este teste existir no host. O que a asserção mede
    // é a pasta que ESTE teste fez aparecer: o host é do dono e pode ter uma
    // sessão real de pé, e culpar o reatar pelo que já estava lá seria medir o
    // servidor, não o que está sob teste.
    let base = pastas_do_tyba(&env);

    let mut wire = sessao_integrada(&env, RemoteShell::Bash, "reatar");
    assert!(
        wire.ate_o_prompt(),
        "a sessão integrada não chegou ao prompt em {SUBIDA:?}. {}",
        wire.diag()
    );

    // A subida tem de sair limpa ANTES de o primeiro reatar acontecer. Sem esta
    // conferência, uma pasta deixada pela subida apareceria lá embaixo como se
    // fosse do reatar — e o caso acusaria a peça errada.
    let da_subida = pastas_novas(&env, &base);
    assert!(
        da_subida.is_empty(),
        "regra 9: a subida já tinha de ter apagado a própria pasta; sobrou \
         {da_subida:?}"
    );

    // A marca vive no PROCESSO do shell remoto, e é o que separa reatar de
    // recomeçar: um pane recriado traria um shell novo, com a variável vazia. O
    // token da tela só existe na SAÍDA do `printf` — a linha digitada carrega
    // `%s` e o nome da variável.
    let semente = nonce()[..8].to_string();
    let na_tela = format!("TELA{semente}OK");
    let antes = wire.screen.len();
    wire.digita(&format!(
        "TYBA_E2E_MARCA={semente}; printf 'TELA%sOK\\n' \"$TYBA_E2E_MARCA\"\r"
    ));
    assert!(
        wire.ate(RESPOSTA, |w| contem(&w.screen[antes..], na_tela.as_bytes())),
        "a marca tinha de estar na tela antes de o Cano cair. {}",
        wire.diag()
    );

    for volta in 1..=VOLTAS {
        wire.reata(RemoteShell::Bash);
        assert!(
            wire.ate(SUBIDA, |w| w.link.in_control()),
            "volta {volta}: reatar anuncia a troca de protocolo como a subida \
             (regra 1). {}",
            wire.diag()
        );
        assert!(
            wire.logado,
            "volta {volta}: o marco de login sai também no ramo de anexar. {}",
            wire.diag()
        );

        // Regra 5: quem reata redesenha do `capture-pane`. `screen` foi zerado
        // no `reata`, então o token só pode ter vindo deste redesenho.
        wire.link.arm_capture();
        let captura = capture_pane(&wire.alvo(), CAPTURE_LINES);
        wire.manda(&captura);
        assert!(
            wire.ate(RESPOSTA, |w| contem(&w.screen, na_tela.as_bytes())),
            "volta {volta}: o redesenho tinha de trazer a tela anterior, sem \
             reexecutar nada. {}",
            wire.diag()
        );

        // Regra 2: o shell do pane é o MESMO processo de antes da queda.
        let antes = wire.screen.len();
        wire.digita("printf 'MARCA=[%s]\\n' \"$TYBA_E2E_MARCA\"\r");
        let marca = format!("MARCA=[{semente}]");
        assert!(
            wire.ate(RESPOSTA, |w| contem(&w.screen[antes..], marca.as_bytes())),
            "volta {volta}: reatar não pode recriar o pane — a marca do shell \
             anterior tinha de continuar lá. {}",
            wire.diag()
        );

        // Regra 9: quem anexa não materializa rc nenhum. Conferido a cada
        // volta, e não só no fim, porque é assim que se sabe QUAL reatar
        // deixou rastro.
        let novas = pastas_novas(&env, &base);
        assert!(
            novas.is_empty(),
            "volta {volta}: reatar não pode materializar o rc — quem anexa não \
             lê o rc, então a pasta ficaria para sempre; apareceu {novas:?}"
        );
    }

    assert!(
        !wire.saiu,
        "a sessão segue viva depois de {VOLTAS} voltas. {}",
        wire.diag()
    );

    wire.cano.kill();
    remote_kill(&env, &wire.remota.name);
    let novas = pastas_novas(&env, &base);
    assert!(
        novas.is_empty(),
        "nada do TYBA pode ficar no servidor depois que a sessão morre; \
         sobrou {novas:?}"
    );
}

/// O que apareceu no servidor DEPOIS da linha de base.
fn pastas_novas(env: &E2eEnv, base: &[String]) -> Vec<String> {
    pastas_do_tyba(env)
        .into_iter()
        .filter(|pasta| !base.contains(pasta))
        .collect()
}

// ---------------------------------------------------------------------------
// Caso 7 — a completação de caminho traz o servidor, não este disco (regra 21)
// ---------------------------------------------------------------------------

/// Uma pasta descartável no servidor com `dir` e `arquivo` dentro. Montada por
/// um `ssh` próprio, e não pelo canal: conferir o canal com o próprio canal não
/// mediria o servidor. Mesmo desenho do `repo_de_teste`.
fn pasta_de_completacao<'a>(env: &'a E2eEnv, dir: &str, arquivo: &str) -> RemoteDir<'a> {
    let script = format!(
        "d=$(mktemp -d /tmp/tyba-e2e05-comp-XXXXXX) || exit 1; \
         mkdir \"$d/{dir}\" || exit 1; \
         : > \"$d/{arquivo}\" || exit 1; \
         printf %s \"$d\""
    );
    let out = ssh_out(env, &script).expect("o ssh que monta a pasta de completação roda");
    // A guarda nasce ANTES de qualquer asserção sobre o caminho: daqui em diante
    // um pânico ainda apaga do servidor o que este teste criou lá.
    let guarda = RemoteDir {
        env,
        path: String::from_utf8_lossy(&out.stdout).trim().to_string(),
    };
    assert!(
        out.status.success(),
        "não deu para montar a pasta de completação no servidor: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        caminho_do_teste(&guarda.path),
        "o mktemp do servidor devolveu um caminho fora do molde do teste: {:?}",
        guarda.path
    );
    guarda
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS e TYBA_E2E_SSH_USER"]
fn a_completacao_de_caminho_traz_as_entradas_do_servidor() {
    let Some(env) = e2e_env("completacao_remota") else {
        return;
    };

    // Os nomes são sorteados AGORA: não existem no disco desta máquina, então
    // uma entrada com um deles só pode ter vindo do host. O diretório ordena
    // depois do arquivo de propósito — assim "diretório antes de arquivo" não
    // pode passar por acaso alfabético.
    let semente = nonce()[..8].to_string();
    let dir = format!("tyba{semente}-z-dir");
    let arquivo = format!("tyba{semente}-a-file.txt");
    let guarda = pasta_de_completacao(&env, &dir, &arquivo);

    // O caminho de produção: o mesmo `build_panel` pelo alias que o
    // `suggest_line` usa numa sessão SSH, e o `complete_path` do painel. Nada de
    // listagem remontada aqui — o que está sob teste é o SFTP do app.
    let panel =
        build_panel(&env.alias, None).expect("o painel remoto sobe pelo alias, como no app");

    let token = format!("tyba{semente}-");
    let achados = panel.complete_path(&guarda.path, &token);
    assert_eq!(
        achados,
        vec![format!("{dir}/"), arquivo],
        "regra 21: as entradas são as que este teste criou NO SERVIDOR, e o \
         diretório vem antes do arquivo mesmo ordenando depois dele"
    );

    // A prova de que não é este disco: a mesma pergunta à completação LOCAL não
    // tem o que responder, porque nem a pasta nem os nomes existem aqui.
    assert!(
        completion::complete_path(Path::new(&guarda.path), &token).is_empty(),
        "a pasta e os nomes foram sorteados no servidor: a completação local não \
         pode ter o que dizer sobre eles"
    );

    // A outra metade da regra: caminho absoluto não é resolvido contra o cwd. O
    // cwd daqui nem existe no servidor — se ele entrasse na conta, o `readdir`
    // falharia e a lista voltaria vazia.
    let cwd_inexistente = format!("/tmp/tyba-e2e05-cwd-{semente}");
    let absoluto = format!("{}/tyba{semente}-z", guarda.path);
    assert_eq!(
        panel.complete_path(&cwd_inexistente, &absoluto),
        vec![format!("{}/{dir}/", guarda.path)],
        "regra 21: token absoluto se resolve no servidor por si só, nunca contra \
         o cwd da sessão"
    );
}
