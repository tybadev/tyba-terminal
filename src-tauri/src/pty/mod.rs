use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::{Condvar, Mutex};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use uuid::Uuid;

use crate::agent::auth_watch::AuthWatch;
use crate::session::cano::{CanoOutcome, CanoWatch};
use crate::session::SessionKind;
use crate::status::observer::ScreenObserver;

use observe::ScreenPipe;

mod capture;
mod holdback;
mod observe;
pub mod tmux_control;

#[cfg(target_os = "windows")]
pub mod conpty_jailed;

pub const EVENT_CWD_CHANGED: &str = "session://cwd-changed";

const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const READ_BUF_SIZE: usize = 8 * 1024;
const SCROLLBACK_LINES: usize = 1000;
const CHANNEL_CAPACITY: usize = 128;

pub type PtyId = Uuid;

/// "O shell já chegou ao editor de linha dele?"
///
/// Serve a quem vai INJETAR uma linha. Antes desse instante o tty está em modo
/// canônico, e escrever ali não perde nada — o driver enfileira, e o zsh executa
/// quando assume o terminal — mas custa duas coisas:
///
/// - **o driver ECOA os bytes crus na tela**, então a injeção aparece como
///   `^[=<comando>` no topo da sessão, antes de qualquer `133;A`: lixo solto,
///   fora de qualquer bloco (verificado em pty real, 2026-08-22);
/// - **o comando só roda quando o shell termina de carregar**, o que no
///   `.zshrc` do dono é 1,4 s. Da cadeira, é apertar Enter e não acontecer nada.
///
/// > [!warning] O portão NÃO existe para impedir perda de byte. Isso foi
/// > medido e é falso: `tcsetattr` do zsh não descarta a fila de entrada, e a
/// > linha escrita em t=0 executa (4/4 em pty real, em t=0, 50 ms e 200 ms).
/// > Quem "otimizar" isto de volta para uma escrita direta não vai ver teste
/// > quebrar por perda — vai ver o eco cru voltar ao topo da sessão.
///
/// O sinal é o `633;P` do `precmd`, que sai imediatamente antes de o zle
/// assumir: medindo o `ECHO` do termios do master a cada 5 ms, ele cai no MESMO
/// milissegundo em que o `633;P` chega. Por isso o portão abre com o `633;P`
/// **em qualquer modo** — ele responde "o editor de linha está vivo", não "o
/// modo prompt está ligado". Um shell em modo clássico também aceita a injeção:
/// o `bindkey '\e='` é instalado pelo rc de qualquer jeito.
///
/// `Mutex` próprio, e não o da tela: o único caminho que o toma é este, sempre
/// como folha — quem espera aqui nunca segura a tela, e quem abre já está
/// dentro dela. Inverter isso é o que criaria ordem de lock.
///
/// Mesmo desenho do [`crate::boot::BootGate`], e pela mesma razão: quem só quer
/// *perguntar* é síncrono, quem vai *escrever* espera.
#[derive(Default)]
pub struct LineEditorGate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl LineEditorGate {
    fn mark_open(&self) {
        let mut open = self.open.lock();
        if *open {
            return;
        }
        *open = true;
        self.changed.notify_all();
    }

    /// Segura até o shell alcançar o editor de linha, e devolve se alcançou.
    ///
    /// `false` é o teto estourado com o portão ainda fechado: shell sem
    /// integração (nunca emite `633;P`), ou que morreu carregando. Quem chama
    /// **escreve assim mesmo** — a espera é para não ecoar lixo, não para
    /// evitar perda, e recusar deixaria de rodar um comando que hoje roda.
    ///
    /// Bloqueante de propósito: o chamador é um comando `async` e paga isto num
    /// `spawn_blocking`, como o `wait_for_boot`.
    pub fn wait_open(&self, timeout: Duration) -> bool {
        let mut open = self.open.lock();
        if *open {
            return true;
        }
        // Laço com prazo, e não um `wait_for` só: `Condvar` acorda espúrio, e
        // uma volta a mais devolveria `false` antes da hora — que aqui não é
        // "esperei demais", é escrever no tty canônico e ecoar a injeção crua,
        // exatamente o que o portão existe para evitar. Mesmo desenho do
        // `BootGate::wait_ready`.
        let deadline = Instant::now() + timeout;
        while !*open {
            if self.changed.wait_until(&mut open, deadline).timed_out() {
                return *open;
            }
        }
        true
    }
}

struct ScreenState {
    parser: vt100::Parser,
    pending: Vec<u8>,
    attachers: HashMap<String, usize>,
    /// Última resposta do shell sobre o modo prompt (`633;P`). Guardada para
    /// poder ser CONSULTADA: evento só é entregue a quem já estava ouvindo.
    prompt_mode: bool,
    /// O hook chegou a ser injetado nesta sessão?
    ///
    /// Existe porque `prompt_mode: false` é ambíguo e responde a duas
    /// perguntas diferentes: "o shell ainda não reportou" e "o shell nunca vai
    /// reportar". A integração só existe para `bash` e `zsh`; num `fish` ou
    /// PowerShell nenhum `633;P` chega jamais, e sem este campo a interface não
    /// tem como saber a diferença — ela mostraria para sempre uma linha
    /// dizendo que o shell está carregando.
    hook_expected: bool,
    /// Aberto pelo mesmo `633;P` que atualiza o campo acima — ver
    /// [`LineEditorGate`].
    line_editor: Arc<LineEditorGate>,
}

impl ScreenState {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
            pending: Vec::with_capacity(READ_BUF_SIZE),
            attachers: HashMap::new(),
            prompt_mode: false,
            hook_expected: false,
            line_editor: Arc::new(LineEditorGate::default()),
        }
    }

    fn attached(&self) -> bool {
        !self.attachers.is_empty()
    }

    /// O modo prompt reposto de fora, sem `633;P` no fluxo — reatar uma sessão
    /// integrada, ou religar o Cano no mesmo id. Nos dois casos quem repõe é a
    /// camada de sessão, no marco de LOGIN: o respawn em si não herda nada
    /// disso, porque até o login o terminal ainda pode estar pedindo senha.
    ///
    /// Ligado, abre o portão junto: o `633;P` que o shell remoto emitiu abriu o
    /// portão da tela ANTERIOR, e repor um sem o outro deixaria a sessão num
    /// estado que shell nenhum produz — modo prompt ligado com o editor de
    /// linha dado como ausente, o que faz a primeira submissão esperar o teto
    /// inteiro. Desligar não afirma nada: o portão responde "o editor de linha
    /// está vivo", não "o modo prompt está ligado".
    fn restore_prompt_mode(&mut self, on: bool) {
        self.prompt_mode = on;
        if on {
            self.line_editor.mark_open();
        }
    }

    fn attach(&mut self, window: &str) {
        *self.attachers.entry(window.to_string()).or_insert(0) += 1;
    }

    fn detach(&mut self, window: &str) {
        let Some(count) = self.attachers.get_mut(window) else {
            return;
        };
        if *count > 1 {
            *count -= 1;
        } else {
            self.drop_window(window);
        }
    }

    fn drop_window(&mut self, window: &str) {
        self.attachers.remove(window);
        if !self.attached() {
            self.pending.clear();
        }
    }

    fn take_pending(&mut self) -> Option<Vec<u8>> {
        if !self.attached() || self.pending.is_empty() {
            self.pending.clear();
            return None;
        }
        Some(std::mem::replace(
            &mut self.pending,
            Vec::with_capacity(READ_BUF_SIZE),
        ))
    }
}

type SharedScreen = Arc<Mutex<ScreenState>>;

/// O que sobrevive a um respawn no mesmo id — ver [`PtyPool::inherited_screen`].
///
/// Nem `prompt_mode` nem `hook_expected` entram, e pelo mesmo motivo: a camada
/// de sessão é quem os reafirma depois do spawn — `hook_expected` já no spawn
/// (`mark_hook_expected`) e o modo prompt só no marco de LOGIN
/// (`restore_integrated_prompt_mode`, que chama [`PtyPool::set_prompt_mode`]).
/// Repor o modo prompt aqui seria repor na fase CRUA do religar, antes do
/// login: com o teclado na linha de comando do TYBA, o pedido de senha do `ssh`
/// cairia numa caixa que segura o que foi digitado em vez de entregar ao
/// terminal.
struct Inherited {
    size: (u16, u16),
    attachers: HashMap<String, usize>,
}

/// Par (master, child) de um spawn enjaulado. Alias porque a tupla de dois trait
/// objects boxed dispara `clippy::type_complexity` no gate.
type JailedPtyPair = (Box<dyn MasterPty + Send>, Box<dyn Child + Send + Sync>);

/// Estratégia de spawn enjaulado (Camada A do Windows, decisão de integração
/// Opção B). Quando o `PtyPool` recebe uma, sobe o processo por ela — ConPTY sob
/// token restrito — em vez do `portable-pty` nativo. A trait é cross-platform de
/// propósito (só o Windows a implementa hoje) para não espalhar `cfg` pelas
/// assinaturas da camada de sessão.
pub trait JailedSpawner: Send {
    fn spawn_jailed(&self, cmd: &CommandBuilder, size: PtySize) -> Result<JailedPtyPair, String>;
}

fn now_ms() -> i64 {
    crate::approvals::now_ms() as i64
}

/// Apaga tela e scrollback e volta o cursor ao topo.
const CLEAR_SCREEN: &[u8] = b"\x1b[H\x1b[2J\x1b[3J";

/// Volta do buffer alternativo ao normal — ver `Action::ResetScreen` em
/// [`apply_screen`].
const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?1049l";

fn emit_pending<R: Runtime>(state: &mut ScreenState, app: &AppHandle<R>, event: &str) {
    if let Some(bytes) = state.take_pending() {
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let _ = app.emit(event, PtyOutputPayload { data });
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open pty: {0}")]
    Open(String),
    #[error("failed to spawn command: {0}")]
    Spawn(String),
    #[error("pty not found: {0}")]
    NotFound(PtyId),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Serialize)]
pub struct PtyOutputPayload {
    pub data: String,
}

#[derive(Clone, Serialize)]
pub struct PtyExitPayload {
    pub code: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SessionCommandPayload {
    /// Linha de comando em execução (shell integration), ou `None` quando ocioso.
    pub command: Option<String>,
    pub running: bool,
    pub agent_match: bool,
    /// O shell está em prompt de continuação (`PS2`): a última linha submetida
    /// não fechou o comando — `for`, `while`, `if`, `cat <<EOF`, aspas abertas.
    ///
    /// Nunca vem junto de `running: true`: são estados diferentes do mesmo
    /// ciclo. Enquanto for `true`, o que o usuário mandar é MAIS LINHA do
    /// mesmo comando, e não comando novo — sem isto o front só vê
    /// `running: false` e oferece a linha como se fosse começar do zero.
    pub continuation: bool,
}

/// Diretório de trabalho reportado via `OSC 7`.
///
/// Atacante-controlável: qualquer processo que escreva no tty pode forjar.
/// Uso exclusivo de exibição — nunca embasa decisão de segurança.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SessionCwdPayload {
    pub cwd: String,
    pub canonical: String,
}

impl SessionCwdPayload {
    pub fn of(path: &std::path::Path) -> Self {
        Self {
            cwd: path.to_string_lossy().into_owned(),
            canonical: crate::repo::canonicalize_or(path)
                .to_string_lossy()
                .into_owned(),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct SessionBracketedPayload {
    pub bracketed_paste: bool,
}

/// O shell confirmando se o `PS1` saiu da tela. Só o hook sabe — o app pediu,
/// mas quem responde é o shell.
#[derive(Clone, Serialize)]
pub struct SessionPromptModePayload {
    pub prompt_mode: bool,
}

/// Por onde a sessão fala, do ponto de vista de quem renderiza.
///
/// Existe porque o front precisa PARAR de responder às consultas do terminal
/// (DA, DSR, DECRQM) quando o transporte é de controle: o tmux remoto responde
/// a elas sozinho e ainda encaminha os bytes crus da consulta ao cliente de
/// controle. A resposta do xterm.js seria a segunda, e só pode voltar como
/// `send-keys` — digitação no pane (visto na tela: `1;2c0;276;0c` no prompt).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Raw,
    TmuxControl,
}

impl TransportKind {
    fn of(in_control: bool) -> Self {
        if in_control {
            Self::TmuxControl
        } else {
            Self::Raw
        }
    }
}

#[derive(Clone, Serialize)]
pub struct SessionTransportPayload {
    pub transport: TransportKind,
}

/// Por onde a sessão fala com o processo do outro lado.
///
/// O `PtyPool` é quem possui o handle e quem troca de transporte; nenhum outro
/// módulo escreve no PTY de uma sessão em modo de controle (ver §7 do desenho).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// O PTY como sempre foi: byte entra, byte sai.
    Raw,
    /// Modo de controle do tmux (`tmux -C`): a saída vem embrulhada em
    /// `%output` e a entrada vai por comando.
    TmuxControl {
        /// Alvo de `send-keys`/`capture-pane`. Vale qualquer alvo que o tmux
        /// entenda — o nome da sessão resolve para o pane ativo dela, o que
        /// evita depender do `%N` que só se descobre em voo.
        pane: String,
        /// Nome da sessão remota, para quem precisar identificá-la.
        session: String,
        /// O marco que anuncia a troca de protocolo (regra 1 da spec).
        /// `None` = o primeiro byte já é protocolo (um `tmux -C` local);
        /// `Some(m)` = tudo antes de `m` é byte cru do ssh (banner, senha e o
        /// marco de login do Cano) e só depois dele começa o protocolo.
        control_marker: Option<String>,
    },
}

impl Transport {
    fn tmux_target(&self) -> Option<&str> {
        match self {
            Transport::Raw => None,
            Transport::TmuxControl { pane, .. } => Some(pane),
        }
    }
}

/// O escritor do PTY é compartilhado porque a thread leitora também escreve:
/// é ela que descobre a troca de protocolo e manda ali mesmo o `refresh-client`
/// (e a captura que ficou pendurada). Ela nunca toma o lock do mapa de PTYs,
/// então não há ciclo com quem chama `write`.
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: SharedWriter,
    child: Box<dyn Child + Send + Sync>,
    leader_pid: Option<u32>,
    leader_start: Option<u64>,
    screen: SharedScreen,
    size: (u16, u16),
    /// Por onde o poll de processo acorda a thread emissora desta sessão.
    ///
    /// Existe porque o observador de tela vive DENTRO daquela thread e ela fica
    /// bloqueada no `recv` enquanto não há saída. Um agente que subiu e deixou
    /// a tela parada seria descoberto pelo poll e nunca reavaliado — ver
    /// [`PtyPool::nudge_screen`].
    ///
    /// **Fraca, e isso não é detalhe.** A thread emissora termina quando o
    /// canal desconecta, e desconectar exige que a ÚLTIMA ponta emissora morra
    /// junto com a thread leitora. Uma ponta forte aqui segurava o canal aberto
    /// para sempre: o laço nunca saía, `ScreenPipe::finish` nunca rodava, e o
    /// palpite de um agente que já morreu ficava no quadro — além de vazar uma
    /// thread por sessão. Pego pelo teste que afirma que a morte do PTY leva o
    /// palpite junto.
    nudge: std::sync::Weak<std::sync::mpsc::SyncSender<Vec<u8>>>,
    transport: Transport,
    /// A ponta compartilhada com o decodificador desta sessão. `None` no
    /// transporte cru.
    control: Option<tmux_control::ControlLink>,
}

impl PtyHandle {
    fn write_raw(&self, data: &[u8]) -> Result<(), PtyError> {
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    /// O alvo do tmux, só quando o protocolo já começou. Enquanto a sessão
    /// está no prelúdio cru — banner, pedido de senha — a resposta é `None` e
    /// o que o dono digita vai como byte, que é o que o `ssh` espera ali.
    fn tmux_target(&self) -> Option<&str> {
        if !self.control.as_ref()?.in_control() {
            return None;
        }
        self.transport.tmux_target()
    }
}

/// Um `send-keys` por lote: colar 200 KB numa linha só daria um comando de
/// meio mega para o lexer do tmux. A ordem entre lotes é a da escrita.
const SEND_KEYS_CHUNK: usize = 512;

/// Como uma sessão ganha (ou não) um observador de tela.
///
/// Fábrica, e não um observador pronto por parâmetro, porque quem sabe montá-lo
/// — registro de manifestos, `SessionManager`, `AppHandle`, prober de processo —
/// só existe junto no `setup` do app. O `PtyPool` não conhece nenhum deles.
pub type ScreenObserverFactory =
    Arc<dyn Fn(PtyId, &SessionKind) -> Option<ScreenObserver> + Send + Sync>;

/// Entrega C — gêmeo de `ScreenObserverFactory`: quem sabe montar o
/// `AuthWatch` (`SessionManager`, `AppHandle`) só existe junto no `setup` do
/// app; o `PtyPool` não conhece nenhum dos dois.
pub type AuthWatchFactory = Arc<dyn Fn(PtyId, &SessionKind) -> Option<AuthWatch> + Send + Sync>;

/// O observador do marco de login de um Cano, com o que fazer quando ele
/// aparece e quando o processo acaba. Roda na thread leitora: `on_finish` é
/// chamado antes do `on_exit` da sessão, sempre.
pub struct LoginPipe {
    pub watch: CanoWatch,
    pub on_login: Box<dyn FnOnce() + Send>,
    pub on_finish: Box<dyn FnOnce(CanoOutcome) + Send>,
}

#[derive(Default)]
pub struct PtyPool {
    ptys: Mutex<HashMap<PtyId, PtyHandle>>,
    observers: Mutex<Option<ScreenObserverFactory>>,
    auth_watchers: Mutex<Option<AuthWatchFactory>>,
}

/// A parte visual das ações, sob um lock só.
///
/// `chunk.get` em vez de índice: o recorte vem da máquina e o chunk é o mesmo
/// que a alimentou, mas um descompasso aqui viraria pânico numa thread de PTY —
/// e thread de PTY que morre leva a sessão junto, em silêncio.
fn apply_screen(state: &mut ScreenState, chunk: &[u8], actions: &[capture::Action]) {
    for action in actions {
        match action {
            capture::Action::Live(range) => {
                if state.attached() {
                    if let Some(bytes) = chunk.get(range.clone()) {
                        state.pending.extend_from_slice(bytes);
                    }
                }
            }
            // Bytes próprios, e não recorte: o eco resgatado veio de um chunk
            // que já passou. Vai para a fila sem limpar nada — o prompt
            // primário que o precede continua desenhado, e o `PS2` que o segue
            // entra depois. O core já tem a linha (ele vê o chunk inteiro), o
            // que faltava era a janela anexada.
            capture::Action::LiveEcho(bytes) => {
                if state.attached() {
                    state.pending.extend_from_slice(bytes);
                }
            }
            capture::Action::LiveRestart(range) => {
                if state.attached() {
                    if let Some(bytes) = chunk.get(range.clone()) {
                        state.pending.clear();
                        state.pending.extend_from_slice(CLEAR_SCREEN);
                        state.pending.extend_from_slice(bytes);
                    }
                }
            }
            // Sequência em vez de parser novo: recriar perderia os modos (como
            // o bracketed paste).
            capture::Action::ClearCoreScreen => state.parser.process(CLEAR_SCREEN),
            capture::Action::ResetScreen => {
                // A fila ao vivo é esvaziada porque o bloco assume a saída —
                // mas ela carrega junto as trocas de MODO que o comando fez, e
                // essas não são saída: são estado do terminal.
                //
                // A que dói é a tela alternativa. O `?1049l` do app que morreu
                // costuma cair no MESMO chunk do `133;D` — no transporte de
                // controle é a regra, porque uma leitura do socket traz vários
                // `%output` de uma vez — e some aqui dentro. O core, que viu o
                // chunk inteiro, volta à tela normal; o webview fica desenhando
                // no buffer alternativo: pane em branco, e o front (que lê
                // `buffer.type` do xterm.js) continua dando o teclado a um app
                // que já não existe. Medido com um tmux ANINHADO dentro de uma
                // sessão SSH integrada, contra o VPS do dono, 2026-09-18.
                //
                // O caminho contrário é silêncio de propósito: repetir um
                // `?1049h` salvaria o cursor de novo (DECSC) e estragaria a
                // volta seguinte, e um bloco que termina DENTRO da tela
                // alternativa já deixa os dois lados no mesmo buffer.
                let left_alt_screen = !state.parser.screen().alternate_screen();
                state.parser.process(CLEAR_SCREEN);
                state.pending.clear();
                if state.attached() {
                    if left_alt_screen {
                        state.pending.extend_from_slice(LEAVE_ALT_SCREEN);
                    }
                    state.pending.extend_from_slice(CLEAR_SCREEN);
                }
            }
            // O portão abre com o `633;P` em QUALQUER modo: ele diz que o
            // editor de linha assumiu o terminal, não que o modo prompt está
            // ligado. Ver [`LineEditorGate`].
            capture::Action::PromptMode(on) => {
                state.prompt_mode = *on;
                state.line_editor.mark_open();
            }
            _ => {}
        }
    }
}

/// Um chunk inteiro sob UM lock só: o parse, a decisão da máquina e o que vai
/// para a fila da tela ao vivo.
///
/// Estar tudo aqui dentro é a invariante, não arrumação. `PtyPool::attach`
/// tranca ESTA mesma tela e faz, nesta ordem: drena a fila, fotografa
/// `contents_formatted()` e só então registra a janela em `attachers`. A foto
/// começa limpando a tela do destino, então tudo que a janela recebeu ANTES
/// dela é inofensivo — mas o que chegar DEPOIS e já estiver dentro da foto é
/// desenhado duas vezes.
///
/// Com o parse e o enfileiramento em seções críticas separadas, um `attach`
/// cabe entre os dois: a foto já traz o efeito do chunk, a janela entra em
/// `attachers`, e o `apply_screen` seguinte vê `attached()` e empurra o MESMO
/// chunk para a fila. O caminho é real — `attach_session` é `async` e roda num
/// worker do runtime, em paralelo com esta thread.
///
/// Passar um `attached()` fotografado no primeiro lock fecharia o caso de UMA
/// janela, mas não o de duas: com outra já anexada, a fotografia diz `true` e
/// o `Live` sai assim mesmo — para quem acabou de anexar, duplicado. O único
/// jeito de a fresta não existir é não haver dois locks.
///
/// O custo cabe: esta seção crítica já carrega o `emit_pending`, que faz base64
/// e atravessa o IPC. Ao lado disso, varrer o chunk atrás de OSC e copiá-lo
/// para a captura não muda de ordem de grandeza — e a tela só tem um usuário
/// quente, que é esta thread.
///
/// O vt100 do core vê o chunk INTEIRO: o recorte da máquina decide o que vira
/// bloco, não o que o terminal desenha.
fn ingest_chunk(
    state: &mut ScreenState,
    machine: &mut capture::CaptureMachine,
    chunk: &[u8],
    now_ms: i64,
) -> Vec<capture::Action> {
    state.parser.process(chunk);
    let alt_screen = state.parser.screen().alternate_screen();
    let actions = machine.on_chunk(chunk, now_ms, alt_screen);
    apply_screen(state, chunk, &actions);
    actions
}

/// O lado não-visual das ações: eventos para o webview, histórico e blocos.
/// Fora do lock de tela — `emit` atravessa IPC e não pode segurar o terminal.
struct ActionSink<R: Runtime> {
    app: AppHandle<R>,
    session_id: PtyId,
    command_event: String,
    cwd_event: String,
    prompt_mode_event: String,
}

impl<R: Runtime> ActionSink<R> {
    /// Devolve `true` quando um comando começou — é o sinal para reiniciar o
    /// relógio do checkpoint.
    fn run(&self, actions: Vec<capture::Action>, cols: u16, rows: u16) -> bool {
        let mut started = false;
        for action in actions {
            match action {
                // Um payload por estado, e o estado vem pronto da máquina. Já
                // foram três ações — `Running`, `Idle`, `Continuation` — e cada
                // uma montava aqui um `SessionCommandPayload` COMPLETO com os
                // campos que não conhecia zerados. Como o front substitui o
                // objeto da sessão a cada payload, a última a sair apagava as
                // anteriores: o `Continuation` do fim do chunk derrubava o
                // `Running` empurrado um instante antes.
                capture::Action::CommandState(state) => {
                    started |= state.running;
                    let _ = self.app.emit(&self.command_event, state);
                }
                capture::Action::Record(record) => crate::history::record(record),
                capture::Action::ShellPath(path) => {
                    crate::completion::binary::set_path(&self.session_id.to_string(), &path);
                }
                capture::Action::ShellCommands(batch) => {
                    crate::completion::binary::absorb_reported(
                        &self.session_id.to_string(),
                        &batch,
                    );
                }
                capture::Action::Wipe => {
                    crate::blocks::submit(crate::blocks::Work::Wipe(self.session_id.to_string()))
                }
                capture::Action::Finalize(block) => {
                    crate::blocks::finalize(crate::blocks::Finished {
                        session_id: self.session_id.to_string(),
                        command: block.command,
                        exit_code: block.exit_code,
                        cwd: block.cwd,
                        started_at_ms: block.started_at_ms,
                        finished_at_ms: block.finished_at_ms,
                        bytes: block.bytes,
                        cols,
                        rows,
                        dropped: block.dropped,
                        alt_screen: block.alt_screen,
                    })
                }
                // Emitido a CADA prompt, não só na mudança: um evento só chega
                // a quem já estava ouvindo, e quem assinou tarde ficaria sem
                // saber para sempre — foi o que deixou a linha de comando sem
                // aparecer.
                capture::Action::PromptMode(on) => {
                    let _ = self.app.emit(
                        &self.prompt_mode_event,
                        SessionPromptModePayload { prompt_mode: on },
                    );
                }
                capture::Action::Cwd(payload) => {
                    let _ = self.app.emit(&self.cwd_event, payload);
                    let _ = self.app.emit(EVENT_CWD_CHANGED, self.session_id);
                }
                capture::Action::Live(_)
                | capture::Action::LiveEcho(_)
                | capture::Action::LiveRestart(_)
                | capture::Action::ClearCoreScreen
                | capture::Action::ResetScreen => {}
            }
        }
        started
    }
}

impl PtyPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Instalada uma vez, no `setup`. Sem ela nenhuma sessão observa tela — que
    /// é o estado de um `PtyPool` de teste.
    pub fn set_screen_observers(&self, factory: ScreenObserverFactory) {
        *self.observers.lock() = Some(factory);
    }

    /// Entrega C — gêmeo de `set_screen_observers`: instalada uma vez, no
    /// `setup`. Sem ela nenhuma sessão de agente Claude ganha o scanner de
    /// auth de runtime.
    pub fn set_auth_watchers(&self, factory: AuthWatchFactory) {
        *self.auth_watchers.lock() = Some(factory);
    }

    /// Acorda a thread emissora para reavaliar a tela desta sessão.
    ///
    /// Chamado quando o poll de processo descobre (ou perde) o binário de um
    /// agente: a decisão de identidade tem duas entradas e só uma delas — a
    /// tela — gera flush sozinha. Sem esta cutucada, `claude` cru que sobe e
    /// deixa a tela parada nunca entraria na lista.
    ///
    /// Chunk vazio de propósito: atravessa o mesmo caminho de sempre, não
    /// escreve byte nenhum no parser e faz o laço chegar ao recorte. `try_send`
    /// porque fila cheia significa saída correndo — a reavaliação já vem por
    /// conta própria, e bloquear a thread do poll para dizer "reavalie" seria
    /// pagar caro para não mudar nada.
    pub fn nudge_screen(&self, id: PtyId) -> bool {
        let ptys = self.ptys.lock();
        let Some(handle) = ptys.get(&id) else {
            return false;
        };
        // `upgrade` falha quando a thread leitora já morreu — sessão encerrada
        // que ainda não saiu do mapa não é cutucada.
        let Some(nudge) = handle.nudge.upgrade() else {
            return false;
        };
        nudge.try_send(Vec::new()).is_ok()
    }

    /// O que observa a tela desta sessão, se a fábrica estiver instalada e o
    /// tipo da sessão admitir palpite de tela.
    ///
    /// Resolvido aqui dentro, e não recebido pronto de quem chama o `spawn`:
    /// um parâmetro a mais numa função de doze é um parâmetro que alguém passa
    /// `None` sem que nada acuse.
    fn screen_pipe(&self, id: PtyId, kind: &SessionKind) -> Option<ScreenPipe> {
        let factory = self.observers.lock().clone()?;
        factory(id, kind).map(ScreenPipe::new)
    }

    /// Entrega C — o que escuta o stream cru desta sessão pra auth de
    /// runtime, se a fábrica estiver instalada e a sessão admitir
    /// (`SessionKind::Agent` com tabela não-vazia — ver `AuthWatch::new` /
    /// `patterns_for`, R10 do contrato de cobertura).
    fn auth_pipe(&self, id: PtyId, kind: &SessionKind) -> Option<AuthWatch> {
        let factory = self.auth_watchers.lock().clone()?;
        factory(id, kind)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn<R: Runtime>(
        &self,
        app: AppHandle<R>,
        session_id: PtyId,
        cmd: CommandBuilder,
        env: Option<&HashMap<String, String>>,
        jail: Option<Box<dyn JailedSpawner>>,
        cols: u16,
        rows: u16,
        kind: &SessionKind,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<(), PtyError> {
        self.spawn_inner(
            app,
            session_id,
            cmd,
            env,
            jail,
            cols,
            rows,
            kind,
            None,
            Transport::Raw,
            on_exit,
        )
    }

    /// O `spawn` com o transporte na mão — é por aqui que uma SSH Session
    /// integrada nasce falando o protocolo de controle do tmux. `spawn` e
    /// `spawn_cano` são este mesmo caminho com [`Transport::Raw`].
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_transport<R: Runtime>(
        &self,
        app: AppHandle<R>,
        session_id: PtyId,
        cmd: CommandBuilder,
        env: Option<&HashMap<String, String>>,
        jail: Option<Box<dyn JailedSpawner>>,
        cols: u16,
        rows: u16,
        kind: &SessionKind,
        login: Option<LoginPipe>,
        transport: Transport,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<(), PtyError> {
        self.spawn_inner(
            app, session_id, cmd, env, jail, cols, rows, kind, login, transport, on_exit,
        )
    }

    /// Spawn de um Cano: igual ao [`Self::spawn`], com o observador do marco de
    /// login na thread leitora.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_cano<R: Runtime>(
        &self,
        app: AppHandle<R>,
        session_id: PtyId,
        cmd: CommandBuilder,
        cols: u16,
        rows: u16,
        kind: &SessionKind,
        login: LoginPipe,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<(), PtyError> {
        self.spawn_inner(
            app,
            session_id,
            cmd,
            None,
            None,
            cols,
            rows,
            kind,
            Some(login),
            Transport::Raw,
            on_exit,
        )
    }

    /// Um Cano religado no mesmo id é a mesma SSH Session na tela: o pane
    /// continua anexado e o PTY novo nasce do tamanho que o pane tem agora.
    /// Sem isso o `ScreenState` novo nasce sem janelas e o pane fica mudo.
    fn inherited_screen(&self, id: PtyId, kind: &SessionKind) -> Option<Inherited> {
        if !matches!(kind, SessionKind::Ssh { .. }) {
            return None;
        }
        let ptys = self.ptys.lock();
        let previous = ptys.get(&id)?;
        let screen = previous.screen.lock();
        Some(Inherited {
            size: previous.size,
            attachers: screen.attachers.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_inner<R: Runtime>(
        &self,
        app: AppHandle<R>,
        session_id: PtyId,
        mut cmd: CommandBuilder,
        env: Option<&HashMap<String, String>>,
        jail: Option<Box<dyn JailedSpawner>>,
        cols: u16,
        rows: u16,
        kind: &SessionKind,
        login: Option<LoginPipe>,
        transport: Transport,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<(), PtyError> {
        let inherited = self.inherited_screen(session_id, kind);
        let (cols, rows) = inherited.as_ref().map_or((cols, rows), |i| i.size);
        // O palpite de tela nasce com o PTY e morre com ele. Sessão de agente
        // do TYBA não recebe nenhum: onde há hook, a tela não opina.
        let pipe = self.screen_pipe(session_id, kind);
        // Entrega C: o inverso do palpite de tela acima — só sessão de
        // AGENTE (Claude Code, hoje) ganha o scanner de auth de runtime.
        let auth_watch = self.auth_pipe(session_id, kind);
        if let Some(env) = env {
            cmd.env_clear();
            for (k, v) in env {
                cmd.env(k, v);
            }
        }

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        // Camada A do Windows: quando há jaula, o agente sobe pelo spawn enjaulado
        // (ConPTY sob token restrito), não pelo PTY nativo. Reader/writer/child
        // seguem idênticos daqui pra baixo — a trait devolve os mesmos objetos.
        let (master, child) = match jail {
            Some(spawner) => spawner.spawn_jailed(&cmd, size).map_err(PtyError::Spawn)?,
            None => {
                // Windows: o ConPTY do portable-pty usa PSEUDOCONSOLE_INHERIT_CURSOR,
                // que faz o conhost mandar `ESC[6n` e TRAVAR esperando a resposta da
                // posição do cursor neste build (26200) — o shell nunca renderiza.
                // Usamos nosso próprio spawn (`conpty_jailed`, flags=0) sem token.
                #[cfg(windows)]
                {
                    let command_line =
                        conpty_jailed::encode_command_line(&cmd).map_err(PtyError::Spawn)?;
                    let env_block = conpty_jailed::encode_env_block(&cmd, &[]);
                    let cwd = conpty_jailed::encode_cwd(&cmd);
                    conpty_jailed::spawn(conpty_jailed::JailSpawnParams {
                        token: std::ptr::null_mut(),
                        command_line,
                        env_block,
                        cwd,
                        size,
                        mitigation: None,
                    })
                    .map_err(PtyError::Spawn)?
                }
                #[cfg(not(windows))]
                {
                    let pair = portable_pty::native_pty_system()
                        .openpty(size)
                        .map_err(|e| PtyError::Open(e.to_string()))?;
                    let child = pair
                        .slave
                        .spawn_command(cmd)
                        .map_err(|e| PtyError::Spawn(e.to_string()))?;
                    drop(pair.slave);
                    (pair.master, child)
                }
            }
        };

        let leader_pid = child.process_id();
        let leader_start = leader_pid.and_then(crate::repo::process_start_time);

        let mut reader = master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let writer: SharedWriter = Arc::new(Mutex::new(
            master
                .take_writer()
                .map_err(|e| PtyError::Open(e.to_string()))?,
        ));

        // Nasce antes do handle porque o handle guarda a outra ponta: quem
        // chama `write` precisa saber em que fase o fluxo está.
        let mut decoder = match &transport {
            Transport::Raw => None,
            Transport::TmuxControl { control_marker, .. } => Some(
                tmux_control::ControlDecoder::with_marker(control_marker.as_deref()),
            ),
        };
        let control = decoder.as_ref().map(|d| d.link());
        if let Some(link) = control.as_ref() {
            link.set_size(cols, rows);
        }
        let control_writer = Arc::clone(&writer);
        let tmux_target = transport.tmux_target().unwrap_or_default().to_string();
        let control_link = control.clone();
        // Um `tmux -C` local nasce falando o protocolo (sem marco): ali não há
        // transição para anunciar, e o estado inicial já é o definitivo.
        let starts_in_control = control.as_ref().is_some_and(|link| link.in_control());

        let mut state = ScreenState::new(rows, cols);
        if let Some(inherited) = inherited {
            state.attachers = inherited.attachers;
        }
        let screen: SharedScreen = Arc::new(Mutex::new(state));
        let reader_screen = Arc::clone(&screen);

        // Criado antes da inserção porque o handle guarda uma ponta dele. O
        // `Arc` existe só para o handle poder guardar uma ponta FRACA: a única
        // ponta forte vai para a thread leitora e morre com ela.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);
        let tx = std::sync::Arc::new(tx);
        let nudge = std::sync::Arc::downgrade(&tx);

        self.ptys.lock().insert(
            session_id,
            PtyHandle {
                master,
                writer,
                child,
                leader_pid,
                leader_start,
                screen,
                size: (cols, rows),
                nudge,
                transport,
                control,
            },
        );

        let output_event = format!("pty://output/{session_id}");
        let exit_event = format!("pty://exit/{session_id}");
        let command_event = format!("session://command/{session_id}");
        let cwd_event = format!("session://cwd/{session_id}");
        let bracketed_event = format!("session://bracketed/{session_id}");
        let prompt_mode_event = format!("session://prompt-mode/{session_id}");
        let transport_event = format!("session://transport/{session_id}");
        // O estado inicial sai aqui, e não na primeira leitura: sem ele, uma
        // sessão crua — que nunca transiciona — jamais anunciaria nada, e o
        // front ficaria sem resposta para sempre.
        let _ = app.emit(
            &transport_event,
            SessionTransportPayload {
                transport: TransportKind::of(starts_in_control),
            },
        );
        let reader_app = app.clone();
        let reader_transport_event = transport_event.clone();
        std::thread::Builder::new()
            .name(format!("pty-reader-{session_id}"))
            .spawn(move || {
                let mut buf = [0u8; READ_BUF_SIZE];
                let mut hold_back = holdback::HoldBack::new();
                let mut auth_watch = auth_watch;
                let mut login = login;
                let mut on_login = login
                    .as_mut()
                    .map(|l| std::mem::replace(&mut l.on_login, Box::new(|| {})));
                let mut handshaked = false;
                let mut announced_oddity = false;
                let mut decoded = Vec::new();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            // O transporte é o primeiro a ver o byte: no modo
                            // de controle, o que os três ouvintes abaixo
                            // esperam é o fluxo do PANE, não o protocolo.
                            // Um `Vec` por leitura, e não um evento por
                            // `%output`: o lote de ~16 ms para o webview
                            // continua sendo do emissor (princípio #3).
                            let bytes: &[u8] = match decoder.as_mut() {
                                None => &buf[..n],
                                Some(decoder) => {
                                    decoded.clear();
                                    for event in decoder.feed(&buf[..n]) {
                                        match event {
                                            tmux_control::ControlEvent::Output(chunk) => {
                                                decoded.extend_from_slice(&chunk)
                                            }
                                            // Regra 6: uma linha por sessão no
                                            // log e a sessão segue viva.
                                            tmux_control::ControlEvent::Notification(what)
                                            | tmux_control::ControlEvent::Unparsed(what) => {
                                                if !announced_oddity {
                                                    announced_oddity = true;
                                                    eprintln!(
                                                        "tyba: sessão {session_id} — modo de \
                                                         controle do tmux com linha não \
                                                         reconhecida (a sessão segue): {what}"
                                                    );
                                                }
                                            }
                                            tmux_control::ControlEvent::Exit => {}
                                        }
                                    }
                                    &decoded
                                }
                            };
                            if let Some(link) = control_link.as_ref() {
                                if !handshaked && link.in_control() {
                                    handshaked = true;
                                    let (cols, rows) = link.size();
                                    let mut commands =
                                        format!("{}\n", tmux_control::refresh_client(cols, rows));
                                    // O pedido de redesenho de quem reatou
                                    // esperou aqui: mandá-lo antes do marco
                                    // seria escrever comando de tmux no meio
                                    // do prelúdio do ssh.
                                    if link.take_queued_capture() {
                                        link.arm_capture();
                                        commands.push_str(&tmux_control::capture_pane(
                                            &tmux_target,
                                            tmux_control::CAPTURE_LINES,
                                        ));
                                        commands.push('\n');
                                    }
                                    let mut writer = control_writer.lock();
                                    let _ = writer.write_all(commands.as_bytes());
                                    let _ = writer.flush();
                                    drop(writer);
                                    // Uma vez por sessão, na transição — nunca
                                    // por `%output` (princípio #3). Quem já
                                    // nasceu em modo de controle foi anunciado
                                    // no spawn e não anuncia de novo aqui.
                                    if !starts_in_control {
                                        let _ = reader_app.emit(
                                            &reader_transport_event,
                                            SessionTransportPayload {
                                                transport: TransportKind::TmuxControl,
                                            },
                                        );
                                    }
                                }
                            }
                            // Bytes crus também: o marco não pode depender do
                            // que a retenção de OSC faz com ele.
                            if let Some(l) = login.as_mut() {
                                if l.watch.feed(bytes) {
                                    if let Some(notify) = on_login.take() {
                                        notify();
                                    }
                                }
                            }
                            // Entrega C: bytes CRUS, antes de qualquer coisa
                            // que `hold_back` faça com eles — o scanner
                            // precisa do stream tal como o processo escreveu,
                            // não do que sobra depois da retenção de OSC.
                            if let Some(w) = auth_watch.as_mut() {
                                w.feed(bytes);
                            }
                            if bytes.is_empty() {
                                continue;
                            }
                            let ready = hold_back.feed(bytes);
                            if !ready.is_empty() && tx.send(ready).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                let tail = hold_back.flush();
                if !tail.is_empty() {
                    let _ = tx.send(tail);
                }
                // Antes do `tx` cair: é a queda do canal que leva a thread
                // emissora ao `on_exit`, e o desfecho tem de estar pronto antes.
                if let Some(l) = login {
                    (l.on_finish)(l.watch.finish());
                }
                drop(tx);
            })
            .map_err(|e| {
                let _ = self.kill(session_id);
                PtyError::Spawn(format!("pty reader thread: {e}"))
            })?;

        std::thread::Builder::new()
            .name(format!("pty-emitter-{session_id}"))
            .spawn(move || {
                let mut queued = false;
                let mut last_flush = Instant::now();
                let mut last_bracketed = false;
                let mut last_checkpoint = Instant::now();
                let mut machine = capture::CaptureMachine::new(session_id.to_string());
                let mut pipe = pipe;
                let sink = ActionSink {
                    app: app.clone(),
                    session_id,
                    command_event,
                    cwd_event,
                    prompt_mode_event,
                };

                loop {
                    // O despertar extra é do assentamento: ver `observe`.
                    let settling = pipe.as_ref().is_some_and(ScreenPipe::wants_settle);
                    let chunk = if !queued && !settling {
                        match rx.recv() {
                            Ok(chunk) => Some(chunk),
                            Err(_) => break,
                        }
                    } else {
                        match rx.recv_timeout(FLUSH_INTERVAL) {
                            Ok(chunk) => Some(chunk),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    };
                    match chunk {
                        Some(chunk) => {
                            let due = last_flush.elapsed() >= FLUSH_INTERVAL;
                            // Um lock só do parse até a fila: ver `ingest_chunk`.
                            let (actions, bracketed, cols, rows, snapshot) = {
                                let mut screen = reader_screen.lock();
                                let actions =
                                    ingest_chunk(&mut screen, &mut machine, &chunk, now_ms());
                                if due {
                                    emit_pending(&mut screen, &app, &output_event);
                                }
                                queued = !screen.pending.is_empty();
                                let (rows, cols) = screen.parser.screen().size();
                                let snapshot = pipe.as_mut().and_then(|p| p.cut(&screen, due));
                                (
                                    actions,
                                    screen.parser.screen().bracketed_paste(),
                                    cols,
                                    rows,
                                    snapshot,
                                )
                            };
                            // FORA do lock e FORA da main thread, de propósito.
                            //
                            // Sob o lock sai só o RECORTE — título e até oito
                            // linhas —, porque avaliar manifesto ali seria
                            // segurar a tela inteira (o `attach`, o `resize`, o
                            // `write`) por regra de terceiro. E aqui, na thread
                            // emissora desta sessão, e não num comando do
                            // Tauri: comando síncrono roda na main thread do
                            // macOS, onde qualquer microssegundo é frame de UI.
                            //
                            // A alternativa considerada — uma thread só para
                            // observar, comum a todas as sessões — foi
                            // descartada por raio de alcance: aqui, um
                            // manifesto patológico atrasa a sessão que o
                            // carrega; lá, atrasaria a detecção de todas.
                            if let (Some(pipe), Some(snapshot)) = (pipe.as_mut(), snapshot) {
                                pipe.feed(&snapshot);
                            }
                            if due {
                                last_flush = Instant::now();
                            }
                            if bracketed != last_bracketed {
                                last_bracketed = bracketed;
                                let _ = app.emit(
                                    &bracketed_event,
                                    SessionBracketedPayload {
                                        bracketed_paste: bracketed,
                                    },
                                );
                            }
                            if sink.run(actions, cols, rows) {
                                last_checkpoint = Instant::now();
                            }
                            if last_checkpoint.elapsed() >= crate::blocks::CHECKPOINT_EVERY {
                                // Sem isto, um crash no meio de um comando longo
                                // perde a saída inteira: o bloco só nasce no
                                // `133;D`.
                                if let Some(snapshot) = machine.checkpoint(now_ms()) {
                                    last_checkpoint = Instant::now();
                                    crate::blocks::submit(crate::blocks::Work::Save(
                                        crate::blocks::Checkpoint {
                                            session_id: session_id.to_string(),
                                            command: snapshot.command,
                                            started_at_ms: snapshot.started_at_ms,
                                            bytes: snapshot.bytes,
                                            cols,
                                            rows,
                                        },
                                    ));
                                }
                            }
                        }
                        None => {
                            let snapshot = {
                                let mut screen = reader_screen.lock();
                                emit_pending(&mut screen, &app, &output_event);
                                queued = false;
                                pipe.as_mut().and_then(|p| p.cut_pending(&screen))
                            };
                            if let (Some(pipe), Some(snapshot)) = (pipe.as_mut(), snapshot) {
                                pipe.feed(&snapshot);
                            }
                            last_flush = Instant::now();
                        }
                    }
                }
                // O PTY morreu. Se havia comando em voo, o `133;D` nunca vai
                // chegar e este é o último ponto que ainda sabe disso: sem
                // fechar aqui, o bloco fica pulsando para sempre e o front
                // nunca ouve `running: false` — a linha do TYBA fica
                // desabilitada pelo resto da vida da aba.
                let actions = machine.on_eof(now_ms());
                let (cols, rows) = {
                    let mut screen = reader_screen.lock();
                    apply_screen(&mut screen, &[], &actions);
                    emit_pending(&mut screen, &app, &output_event);
                    let (rows, cols) = screen.parser.screen().size();
                    (cols, rows)
                };
                sink.run(actions, cols, rows);
                if let Some(pipe) = pipe.as_mut() {
                    pipe.finish();
                }
                // O que o shell contou morre com a sessão. Sem isto o mapa
                // cresce por toda sessão aberta na vida do app, e o alias de um
                // worktree fechado continuaria sendo sugerido em outro.
                crate::completion::binary::forget_reported(&session_id.to_string());
                let _ = app.emit(&exit_event, PtyExitPayload { code: None });
                on_exit();
            })
            .map_err(|e| {
                let _ = self.kill(session_id);
                PtyError::Spawn(format!("pty emitter thread: {e}"))
            })?;

        Ok(())
    }

    /// Quem chama continua escrevendo bytes; o transporte decide se eles vão
    /// como bytes ou como `send-keys`.
    pub fn write(&self, id: PtyId, data: &[u8]) -> Result<(), PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        let Some(target) = handle.tmux_target() else {
            return handle.write_raw(data);
        };
        if data.is_empty() {
            return Ok(());
        }
        let mut commands = String::new();
        for chunk in data.chunks(SEND_KEYS_CHUNK) {
            commands.push_str(&tmux_control::send_keys(target, chunk));
            commands.push('\n');
        }
        handle.write_raw(commands.as_bytes())
    }

    pub fn resize(&self, id: PtyId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let mut ptys = self.ptys.lock();
        let handle = ptys.get_mut(&id).ok_or(PtyError::NotFound(id))?;
        if handle.size == (cols, rows) {
            return Ok(());
        }
        // No modo de controle o tamanho do cliente NÃO vem do tty (`man tmux`,
        // `refresh-client -C`): mexer no master faria o `ssh` levar um
        // SIGWINCH que briga com o tamanho declarado pelo comando.
        match handle.control.clone() {
            None => handle
                .master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| PtyError::Open(e.to_string()))?,
            Some(link) => {
                link.set_size(cols, rows);
                // Antes do marco não há tmux para ouvir: o tamanho viaja no
                // `refresh-client` que a troca de protocolo dispara.
                if link.in_control() {
                    handle.write_raw(
                        format!("{}\n", tmux_control::refresh_client(cols, rows)).as_bytes(),
                    )?;
                }
            }
        }
        handle.size = (cols, rows);
        handle.screen.lock().parser.set_size(rows, cols);
        Ok(())
    }

    /// Redesenha a tela de uma sessão reatada a partir do que o tmux guardou
    /// (regra 5) — um cliente de controle que ataca não recebe nada por conta
    /// própria (medido em 2026-09-17).
    ///
    /// O texto capturado volta como corpo de bloco `%begin`/`%end`, e é por
    /// isso que a captura é *armada* antes de o comando sair: só com a captura
    /// armada o decodificador deixa um corpo de bloco virar tela. Chamar isto
    /// num transporte cru é um no-op.
    pub fn redraw_from_capture(&self, id: PtyId) -> Result<(), PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        let (Some(target), Some(link)) = (handle.transport.tmux_target(), handle.control.as_ref())
        else {
            return Ok(());
        };
        if !link.in_control() {
            link.queue_capture();
            return Ok(());
        }
        link.arm_capture();
        handle.write_raw(
            format!(
                "{}\n",
                tmux_control::capture_pane(target, tmux_control::CAPTURE_LINES)
            )
            .as_bytes(),
        )
    }

    fn screen_of(&self, id: PtyId) -> Option<SharedScreen> {
        Some(Arc::clone(&self.ptys.lock().get(&id)?.screen))
    }

    pub fn attach(&self, app: &AppHandle, window: &str, id: PtyId) -> Result<(), PtyError> {
        let screen = self.screen_of(id).ok_or(PtyError::NotFound(id))?;
        let event = format!("pty://output/{id}");
        let mut state = screen.lock();

        emit_pending(&mut state, app, &event);

        let snapshot = state.parser.screen().contents_formatted();
        if !snapshot.is_empty() {
            let data = base64::engine::general_purpose::STANDARD.encode(&snapshot);
            let _ = app.emit_to(window, &event, PtyOutputPayload { data });
        }
        state.attach(window);
        Ok(())
    }

    pub fn detach(&self, window: &str, id: PtyId) {
        let Some(screen) = self.screen_of(id) else {
            return;
        };
        screen.lock().detach(window);
    }

    pub fn drop_window_attachers(&self, window: &str) {
        for handle in self.ptys.lock().values() {
            handle.screen.lock().drop_window(window);
        }
    }

    /// O portão do editor de linha desta sessão — ver [`LineEditorGate`].
    ///
    /// Devolve o `Arc` em vez de esperar aqui dentro: esperar seguraria o lock
    /// do pool inteiro, e o portão de UMA sessão fecharia todas as outras.
    pub fn line_editor_gate(&self, id: PtyId) -> Option<Arc<LineEditorGate>> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id)?;
        let gate = Arc::clone(&handle.screen.lock().line_editor);
        Some(gate)
    }

    /// Esta sessão subiu com o hook injetado?
    ///
    /// `false` aqui significa que `prompt_mode` nunca vai virar `true` — não
    /// que ele ainda não virou. Ver [`ScreenState::hook_expected`].
    pub fn hook_expected(&self, id: PtyId) -> Option<bool> {
        let ptys = self.ptys.lock();
        let expected = ptys.get(&id)?.screen.lock().hook_expected;
        Some(expected)
    }

    /// Marcado pelo `SessionManager` logo depois do spawn: só ele sabe qual
    /// shell subiu e se o arquivo de integração existia.
    pub fn set_hook_expected(&self, id: PtyId, expected: bool) {
        let ptys = self.ptys.lock();
        if let Some(handle) = ptys.get(&id) {
            handle.screen.lock().hook_expected = expected;
        }
    }

    /// O par de escrita de [`Self::prompt_mode`], para quem REATA uma sessão
    /// integrada: o `capture-pane` devolve só o texto do pane, e o `633;P` que
    /// o shell remoto emitiu ficou no tmux — a tela nova nasceria em modo
    /// clássico até o próximo prompt de verdade.
    pub fn set_prompt_mode(&self, id: PtyId, on: bool) {
        let ptys = self.ptys.lock();
        if let Some(handle) = ptys.get(&id) {
            handle.screen.lock().restore_prompt_mode(on);
        }
    }

    /// O modo prompt reportado pelo shell, para quem chegou depois do evento.
    pub fn prompt_mode(&self, id: PtyId) -> Option<bool> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id)?;
        let mode = handle.screen.lock().prompt_mode;
        Some(mode)
    }

    /// `ECHO` do termios do PTY — o tty está entregando LINHAS, não teclas.
    ///
    /// Ligado, o driver só devolve a linha ao dar Enter, trata apenas
    /// backspace/kill e ecoa o resto: seta vira byte literal no meio da linha,
    /// que nenhum leitor de linha interpreta. Ela não serve para o programa e
    /// ainda é ecoada — vira `^[[A` na saída e no bloco gravado no disco.
    ///
    /// Desligado (raw), quem lê tecla a tecla precisa das setas de verdade: é o
    /// menu do `npm create`, o `vim`, o `htop`.
    ///
    /// Não confundir com "ninguém está lendo": o `Ok to proceed? (y)` do npm é
    /// canônico COM eco, e é por isso que o `y` digitado aparece. Segurar todo
    /// o teclado neste estado impediria responder ao prompt — por isso só as
    /// setas param aqui.
    ///
    /// Windows não tem termios e devolve `None`: o ConPTY fica como sempre foi.
    #[cfg(unix)]
    pub fn line_echo(&self, id: PtyId) -> Option<bool> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id)?;
        let fd = handle.master.as_raw_fd()?;
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `fd` é o master deste pty, vivo enquanto o handle existir, e
        // o lock acima garante que ele não é fechado no meio da chamada.
        if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } != 0 {
            return None;
        }
        let termios = unsafe { termios.assume_init() };
        Some(termios.c_lflag & libc::ECHO != 0)
    }

    #[cfg(not(unix))]
    pub fn line_echo(&self, _id: PtyId) -> Option<bool> {
        None
    }

    pub fn bracketed_paste(&self, id: PtyId) -> Option<bool> {
        let screen = self.screen_of(id)?;
        let enabled = screen.lock().parser.screen().bracketed_paste();
        Some(enabled)
    }

    /// Derruba o processo e deixa o handle no mapa: o próximo spawn no mesmo id
    /// herda as janelas (ver [`Self::inherited_screen`]).
    pub fn terminate(&self, id: PtyId) -> Result<(), PtyError> {
        let mut ptys = self.ptys.lock();
        let handle = ptys.get_mut(&id).ok_or(PtyError::NotFound(id))?;
        if let Some(pid) = handle.leader_pid {
            let _ = kill_process_group(pid);
        }
        let _ = handle.child.kill();
        Ok(())
    }

    pub fn kill(&self, id: PtyId) -> Result<(), PtyError> {
        let handle = {
            let mut ptys = self.ptys.lock();
            ptys.remove(&id).ok_or(PtyError::NotFound(id))?
        };
        kill_handle(handle);
        Ok(())
    }

    pub fn kill_all(&self) {
        let handles: Vec<PtyHandle> = self.ptys.lock().drain().map(|(_, h)| h).collect();
        for handle in handles {
            kill_handle(handle);
        }
    }

    /// Pid do líder da sessão, só se o processo por trás dele ainda é o
    /// original: pid morto ou reusado pelo SO devolve `None`, nunca um pid
    /// que aponta para um processo alheio.
    pub fn leader_pid(&self, id: PtyId) -> Option<u32> {
        let mut ptys = self.ptys.lock();
        let handle = ptys.get_mut(&id)?;
        let pid = handle.leader_pid?;
        let current = crate::repo::process_start_time(pid);
        if handle.leader_start.is_none() && current.is_some() {
            handle.leader_start = current;
        }
        (current == handle.leader_start).then_some(pid)
    }

    pub fn is_alive(&self, id: PtyId) -> bool {
        self.ptys.lock().contains_key(&id)
    }
}

fn kill_handle(mut handle: PtyHandle) {
    if let Some(pid) = handle.leader_pid {
        let _ = kill_process_group(pid);
    }
    let _ = handle.child.kill();
}

#[cfg(unix)]
fn kill_process_group(leader_pid: u32) -> std::io::Result<()> {
    let pgid = unsafe { libc::getpgid(leader_pid as libc::pid_t) };
    if pgid < 0 {
        let err = std::io::Error::last_os_error();
        return match err.raw_os_error() {
            Some(libc::ESRCH) => Ok(()),
            _ => Err(err),
        };
    }
    let rc = unsafe { libc::killpg(pgid, libc::SIGKILL) };
    if rc == -1 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn kill_process_group(_leader_pid: u32) -> std::io::Result<()> {
    Ok(())
}

pub type SharedPtyPool = Arc<PtyPool>;

#[cfg(test)]
mod screen_state_tests {
    use super::ScreenState;

    #[test]
    fn detached_screen_never_queues_bytes() {
        let mut state = ScreenState::new(24, 80);
        state.parser.process(b"hello");
        assert!(state.take_pending().is_none());
    }

    #[test]
    fn bytes_queued_while_detached_are_discarded() {
        let mut state = ScreenState::new(24, 80);
        state.pending.extend_from_slice(b"stale");
        assert!(state.take_pending().is_none());
        assert!(state.pending.is_empty());
    }

    #[test]
    fn attached_screen_hands_over_queued_bytes_once() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        state.pending.extend_from_slice(b"live");
        assert_eq!(state.take_pending().as_deref(), Some(&b"live"[..]));
        assert!(state.take_pending().is_none());
    }

    #[test]
    fn taking_pending_keeps_the_buffer_reusable() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        state.pending.extend_from_slice(b"a");
        state.take_pending();
        state.pending.extend_from_slice(b"b");
        assert_eq!(state.take_pending().as_deref(), Some(&b"b"[..]));
    }

    #[test]
    fn a_second_window_keeps_the_stream_alive_after_the_first_detaches() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        state.attach("tyba-2");
        state.detach("main");
        state.pending.extend_from_slice(b"still live");
        assert_eq!(state.take_pending().as_deref(), Some(&b"still live"[..]));
    }

    #[test]
    fn detaching_an_unknown_window_is_a_noop() {
        let mut state = ScreenState::new(24, 80);
        state.detach("ghost");
        assert!(!state.attached());
    }

    #[test]
    fn last_detach_of_a_window_discards_pending_bytes() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        state.pending.extend_from_slice(b"stale");
        state.detach("main");
        assert!(!state.attached());
        assert!(state.pending.is_empty());
    }

    #[test]
    fn dropping_a_window_clears_every_attachment_it_held() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        state.attach("main");
        state.pending.extend_from_slice(b"orphaned");
        state.drop_window("main");
        assert!(!state.attached());
        assert!(state.pending.is_empty());
    }

    #[test]
    fn dropping_one_window_keeps_the_other_attached() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        state.attach("tyba-2");
        state.drop_window("tyba-2");
        state.pending.extend_from_slice(b"live");
        assert_eq!(state.take_pending().as_deref(), Some(&b"live"[..]));
    }
}

#[cfg(test)]
mod ingest_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use parking_lot::Mutex;

    use super::capture::CaptureMachine;
    use super::{apply_screen, ingest_chunk, LineEditorGate, ScreenState, SCROLLBACK_LINES};

    /// O que uma janela recebe ao anexar, na ordem de `PtyPool::attach`: o que
    /// estava na fila (que sai em broadcast, e ela já está ouvindo) e depois a
    /// foto da tela. A foto começa limpando o destino — por isso o que veio
    /// antes dela não conta, e o que vier depois conta duas vezes.
    fn attach_bytes(state: &mut ScreenState, window: &str) -> Vec<u8> {
        let mut seen = state.take_pending().unwrap_or_default();
        seen.extend_from_slice(&state.parser.screen().contents_formatted());
        state.attach(window);
        seen
    }

    /// A tela que esses bytes desenham num terminal virgem — é o que o webview
    /// mostra, e a única unidade em que "desenhou duas vezes" é afirmável.
    fn rendered(bytes: &[u8]) -> String {
        let mut parser = vt100::Parser::new(24, 80, SCROLLBACK_LINES);
        parser.process(bytes);
        parser.screen().contents()
    }

    /// O portão nasce fechado: sem isso a espera passaria direto e a linha
    /// voltaria a ser escrita num shell que ainda não lê.
    #[test]
    fn the_line_editor_gate_starts_closed() {
        let gate = LineEditorGate::default();
        assert!(!gate.wait_open(Duration::from_millis(10)));
    }

    /// O `633;P` é o sinal, e ele vale em QUALQUER modo — é o editor de linha
    /// que abre o portão, não o modo prompt. Um shell em modo clássico também
    /// tem `bindkey '\e='` e também aceita a injeção; exigir `tyba-prompt=1`
    /// aqui deixaria a submissão pendurada até o teto justamente nele.
    #[test]
    fn any_prompt_report_opens_the_line_editor_gate() {
        for report in [
            b"\x1b]633;P;tyba-prompt=1\x07".as_slice(),
            b"\x1b]633;P;tyba-prompt=0\x07",
        ] {
            let mut state = ScreenState::new(24, 80);
            let gate = Arc::clone(&state.line_editor);
            let mut machine = CaptureMachine::new("s1".into());
            assert!(!gate.wait_open(Duration::from_millis(0)));
            ingest_chunk(&mut state, &mut machine, report, 1_000);
            assert!(
                gate.wait_open(Duration::from_millis(0)),
                "o `633;P` não abriu o portão: {report:?}"
            );
        }
    }

    /// Saída comum não abre o portão. É o teste que dá sentido aos outros: o rc
    /// do usuário IMPRIME enquanto carrega — `Last login`, banner de nvm, OSC 7
    /// —, e um portão que abrisse com byte qualquer abriria no primeiro deles,
    /// deixando a espera valendo zero justamente na janela que ela cobre.
    #[test]
    fn output_during_startup_does_not_open_the_line_editor_gate() {
        let mut state = ScreenState::new(24, 80);
        let gate = Arc::clone(&state.line_editor);
        let mut machine = CaptureMachine::new("s1".into());
        for chunk in [
            b"Last login: Fri Aug 22\r\n".as_slice(),
            b"\x1b]7;file:///Users/tester\x07",
            b"nvm carregando...\r\n",
        ] {
            ingest_chunk(&mut state, &mut machine, chunk, 1_000);
        }
        assert!(!gate.wait_open(Duration::from_millis(0)));
    }

    /// Quem já esperava acorda — a espera não pode depender de o portão já
    /// estar aberto na hora de perguntar.
    #[test]
    fn a_waiter_wakes_when_the_shell_reaches_its_line_editor() {
        let state = Arc::new(Mutex::new(ScreenState::new(24, 80)));
        let gate = Arc::clone(&state.lock().line_editor);
        let opener = Arc::clone(&state);
        let waiter = std::thread::spawn(move || gate.wait_open(Duration::from_secs(5)));
        let mut machine = CaptureMachine::new("s1".into());
        std::thread::sleep(Duration::from_millis(30));
        ingest_chunk(
            &mut opener.lock(),
            &mut machine,
            b"\x1b]633;P;tyba-prompt=1\x07",
            1_000,
        );
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn ingesting_a_chunk_queues_it_for_an_attached_screen() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        let mut machine = CaptureMachine::new("s1".into());
        ingest_chunk(&mut state, &mut machine, b"tyba", 1_000);
        assert_eq!(state.take_pending().as_deref(), Some(&b"tyba"[..]));
        assert!(state.parser.screen().contents().contains("tyba"));
    }

    #[test]
    fn ingesting_a_chunk_queues_nothing_for_a_detached_screen() {
        let mut state = ScreenState::new(24, 80);
        let mut machine = CaptureMachine::new("s1".into());
        ingest_chunk(&mut state, &mut machine, b"tyba", 1_000);
        assert!(state.pending.is_empty());
        assert!(state.parser.screen().contents().contains("tyba"));
    }

    /// O achado do review no nível do encanamento. Num comando incompleto o
    /// `133;C` nunca chega, então nada repunha o eco engolido: o core, que vê o
    /// chunk inteiro, ficava com a linha submetida, e a janela anexada só com o
    /// `PS2`. Trocar de aba e voltar consertava — a foto vem do core —, ficar
    /// olhando não.
    #[test]
    fn an_incomplete_command_leaves_the_window_with_the_screen_the_core_has() {
        let mut state = ScreenState::new(24, 80);
        state.attach("main");
        let mut machine = CaptureMachine::new("s1".into());
        let mut seen = Vec::new();
        for chunk in [
            b"\x1b]633;P;tyba-prompt=1\x07".as_slice(),
            b"\x1b]133;B\x07",
            b"for i in 1 2 3; do\r\nfor> ",
        ] {
            ingest_chunk(&mut state, &mut machine, chunk, 1_000);
            seen.extend(state.take_pending().unwrap_or_default());
        }
        assert!(
            rendered(&seen).contains("for i in 1 2 3; do"),
            "a janela ficou sem a linha que o `PS2` espera terminar: {:?}",
            rendered(&seen)
        );
        assert_eq!(rendered(&seen), state.parser.screen().contents());
    }

    /// A invariante que o lock único sustenta: só existem duas ordens em que um
    /// `attach` pode cair em relação a um chunk — antes dele e depois dele —, e
    /// as duas têm de deixar a janela com a mesma tela do core. É por não haver
    /// uma terceira ordem que a duplicação some.
    #[test]
    fn an_attacher_sees_the_same_screen_on_either_side_of_a_chunk() {
        let chunk = b"tyba\r\n";

        let mut before = ScreenState::new(24, 80);
        let mut machine = CaptureMachine::new("s1".into());
        let mut seen_before = attach_bytes(&mut before, "main");
        ingest_chunk(&mut before, &mut machine, chunk, 1_000);
        seen_before.extend(before.take_pending().unwrap_or_default());

        let mut after = ScreenState::new(24, 80);
        let mut machine = CaptureMachine::new("s1".into());
        ingest_chunk(&mut after, &mut machine, chunk, 1_000);
        let mut seen_after = attach_bytes(&mut after, "main");
        seen_after.extend(after.take_pending().unwrap_or_default());

        assert_eq!(rendered(&seen_before), rendered(&seen_after));
        assert_eq!(rendered(&seen_before), before.parser.screen().contents());
    }

    /// A terceira ordem, executada à mão: parse num lock, `attach` na fresta,
    /// `apply_screen` noutro. A foto já traz o chunk e o `attached()` do segundo
    /// lock manda o mesmo chunk para a fila — a janela desenha a saída duas
    /// vezes.
    ///
    /// O teste existe porque a corrida em si não é testável: ela mora no
    /// entrelaçamento de duas threads e some ao olhar. Isto é o mais perto que
    /// dá de deixá-la executável, e é o que torna verificável o motivo de
    /// `ingest_chunk` ser um bloco só.
    #[test]
    fn splitting_the_step_draws_the_chunk_twice() {
        let chunk = b"tyba\r\n";
        let mut state = ScreenState::new(24, 80);
        let mut machine = CaptureMachine::new("s1".into());

        state.parser.process(chunk);
        let alt_screen = state.parser.screen().alternate_screen();
        let actions = machine.on_chunk(chunk, 1_000, alt_screen);

        let mut seen = attach_bytes(&mut state, "main");

        apply_screen(&mut state, chunk, &actions);
        seen.extend(state.take_pending().unwrap_or_default());

        assert_eq!(rendered(&seen).matches("tyba").count(), 2);
        assert_eq!(state.parser.screen().contents().matches("tyba").count(), 1);
    }
}

/// A prova de que o produtor está LIGADO — um PTY de verdade, um app mock, e o
/// palpite chegando ao sink.
///
/// Existe porque o resto da fiação é invisível ao teste unitário: com o `spawn`
/// resolvendo o observador por dentro, trocar a resolução por `None` deixaria
/// toda a suíte verde e o produtor desligado em campo. É a diferença entre
/// "compila" e "funciona".
#[cfg(all(test, unix))]
mod spawn_observed_tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use parking_lot::Mutex;
    use portable_pty::CommandBuilder;

    use super::{PtyId, PtyPool};
    use crate::session::{AgentRunnerKind, ObservedAgent, ObservedState, SessionKind};
    use crate::status::observer::ScreenObserver;
    use crate::status::registry::ManifestRegistry;

    const CODEX: &str = r#"
id = "codex"
match = { title = ["Codex"] }

[[rules]]
id = "working"
state = "running"
region = { bottom_lines = 3 }
contains = ["esc to interrupt"]
"#;

    /// Manifesto que só reconhece pelo BINÁRIO. É o caso que a cutucada existe
    /// para servir: nada na tela identifica, e a identidade chega pelo poll.
    const CODEX_POR_PROCESSO: &str = r#"
id = "codex"
match = { process = ["codex"] }
"#;

    type Visto = Arc<Mutex<Vec<Option<ObservedAgent>>>>;

    /// O que a sonda de processo devolve, trocável no meio do teste — é assim
    /// que o poll de 2 s se comporta em campo.
    type Sonda = Arc<Mutex<Option<String>>>;

    fn pool_com_manifesto() -> (PtyPool, Visto) {
        let pool = PtyPool::new();
        let visto: Visto = Arc::new(Mutex::new(Vec::new()));
        let da_fabrica = Arc::clone(&visto);
        pool.set_screen_observers(Arc::new(move |_id, kind| {
            let do_sink = Arc::clone(&da_fabrica);
            ScreenObserver::for_session(
                kind,
                Arc::new(ManifestRegistry::from_sources(&[CODEX])),
                Box::new(|| None),
                Box::new(move |observed| do_sink.lock().push(observed)),
                crate::status::observed_notify::ObservedNotifier::silent(),
            )
        }));
        (pool, visto)
    }

    fn pool_com_sonda() -> (PtyPool, Visto, Sonda) {
        let pool = PtyPool::new();
        let visto: Visto = Arc::new(Mutex::new(Vec::new()));
        let sonda: Sonda = Arc::new(Mutex::new(None));
        let da_fabrica = Arc::clone(&visto);
        let sonda_da_fabrica = Arc::clone(&sonda);
        pool.set_screen_observers(Arc::new(move |_id, kind| {
            let do_sink = Arc::clone(&da_fabrica);
            let do_probe = Arc::clone(&sonda_da_fabrica);
            ScreenObserver::for_session(
                kind,
                Arc::new(ManifestRegistry::from_sources(&[CODEX_POR_PROCESSO])),
                Box::new(move || do_probe.lock().clone()),
                Box::new(move |observed| do_sink.lock().push(observed)),
                crate::status::observed_notify::ObservedNotifier::silent(),
            )
        }));
        (pool, visto, sonda)
    }

    /// Uma rajada com a tela do agente no FIM: o primeiro `printf` chega bem
    /// depois do último flush (vira quadro na hora), e o segundo chega 5 ms
    /// atrás dele — dentro da janela, portanto sem quadro próprio. Só o
    /// assentamento vê essa segunda tela.
    fn tela_de_codex_no_fim_da_rajada() -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(
            "sleep 0.1; printf 'inicio\\n'; sleep 0.005; \
             printf '\\033]0;Codex\\007. Working (2s . esc to interrupt)\\n'; sleep 0.2",
        );
        cmd
    }

    fn espera(visto: &Visto, o_que: impl Fn(&[Option<ObservedAgent>]) -> bool) -> bool {
        espera_ate(visto, Duration::from_secs(10), o_que)
    }

    /// Para afirmar que algo NÃO acontece, o teto é curto de propósito: o
    /// processo do teste vive ~0,3 s, e esperar dez segundos por um palpite que
    /// não vem é só suíte lenta.
    fn espera_ate(
        visto: &Visto,
        teto: Duration,
        o_que: impl Fn(&[Option<ObservedAgent>]) -> bool,
    ) -> bool {
        let prazo = Instant::now() + teto;
        while Instant::now() < prazo {
            if o_que(&visto.lock()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn a_sessao_de_shell_recebe_o_palpite_do_pty_de_verdade() {
        let app = tauri::test::mock_app();
        let (pool, visto) = pool_com_manifesto();

        pool.spawn(
            app.handle().clone(),
            PtyId::new_v4(),
            tela_de_codex_no_fim_da_rajada(),
            None,
            None,
            80,
            24,
            &SessionKind::Shell,
            Box::new(|| {}),
        )
        .unwrap();

        assert!(
            espera(&visto, |publicados| publicados.iter().flatten().any(|o| {
                o.agent == "codex" && o.state == Some(ObservedState::Running)
            })),
            "o palpite nunca chegou ao sink: publicados={:?}",
            visto.lock()
        );

        // E o fim do processo leva o palpite junto — senão o quadro fica
        // apontando um agente que morreu com o terminal.
        assert!(
            espera(&visto, |publicados| publicados.last() == Some(&None)),
            "o palpite sobreviveu à morte do PTY: publicados={:?}",
            visto.lock()
        );
    }

    /// A cutucada do poll de processo, ponta a ponta.
    ///
    /// O caso real: o dono digita `claude` num shell, a tela assenta, e dois
    /// segundos depois o poll descobre o binário. Nesse instante a thread
    /// emissora está parada no `recv` e não há mais saída nenhuma vindo do
    /// PTY — sem `nudge_screen`, ninguém reavalia e o agente jamais entra na
    /// lista, por mais que a faixa âmbar (que nasce do mesmo poll) já esteja
    /// na tela dizendo que ele existe.
    ///
    /// Sem o teste a fiação seria invisível: trocar o corpo de `nudge_screen`
    /// por `false` deixa a suíte inteira verde.
    #[test]
    fn o_poll_de_processo_acorda_a_reavaliacao_com_a_tela_parada() {
        let app = tauri::test::mock_app();
        let (pool, visto, sonda) = pool_com_sonda();
        let id = PtyId::new_v4();

        // Fala uma vez e cala a boca: depois do `printf` não há mais flush
        // nenhum, que é a condição em que o bug aparecia.
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("printf 'prompt\n'; sleep 3");

        pool.spawn(
            app.handle().clone(),
            id,
            cmd,
            None,
            None,
            80,
            24,
            &SessionKind::Shell,
            Box::new(|| {}),
        )
        .unwrap();

        // Deixa a rajada inicial passar: é ela que fecha o portão de sequência.
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            visto.lock().iter().flatten().next().is_none(),
            "identificou um agente antes de a sonda saber de algum: {:?}",
            visto.lock()
        );

        *sonda.lock() = Some("codex".to_string());
        assert!(pool.nudge_screen(id), "a cutucada não achou a sessão");

        assert!(
            espera_ate(&visto, Duration::from_secs(2), |publicados| {
                publicados.iter().flatten().any(|o| o.agent == "codex")
            }),
            "o agente descoberto pelo poll nunca chegou ao sink: {:?}",
            visto.lock()
        );
    }

    /// A mesma fiação, do outro lado: onde há hook, a tela não opina — e a
    /// recusa acontece antes de qualquer recorte.
    #[test]
    fn a_sessao_de_agente_gerenciada_nao_recebe_palpite_nenhum() {
        let app = tauri::test::mock_app();
        let (pool, visto) = pool_com_manifesto();

        pool.spawn(
            app.handle().clone(),
            PtyId::new_v4(),
            tela_de_codex_no_fim_da_rajada(),
            None,
            None,
            80,
            24,
            &SessionKind::Agent {
                runner: AgentRunnerKind::Codex,
            },
            Box::new(|| {}),
        )
        .unwrap();

        assert!(
            !espera_ate(&visto, Duration::from_secs(2), |publicados| {
                !publicados.is_empty()
            }),
            "sessão com hook recebeu palpite de tela: publicados={:?}",
            visto.lock()
        );
    }
}

#[cfg(all(test, unix))]
mod cano_tests {
    use super::*;
    use crate::session::cano::{CanoOutcome, CanoWatch};
    use crate::ssh::tmux::login_marker;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn ssh_kind() -> SessionKind {
        SessionKind::Ssh {
            host_id: "h".into(),
        }
    }

    fn shell(script: &str) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(script);
        cmd
    }

    type Events = Arc<Mutex<Vec<String>>>;

    fn spawn_cano(pool: &PtyPool, id: PtyId, script: &str, events: &Events) {
        let app = tauri::test::mock_app();
        let (on_login, on_finish, on_exit) =
            (Arc::clone(events), Arc::clone(events), Arc::clone(events));
        pool.spawn_cano(
            app.handle().clone(),
            id,
            shell(script),
            100,
            30,
            &ssh_kind(),
            LoginPipe {
                watch: CanoWatch::new(NONCE),
                on_login: Box::new(move || on_login.lock().push("login".into())),
                on_finish: Box::new(move |outcome| {
                    let label = match outcome {
                        CanoOutcome::LoggedIn => "finish:logged_in".to_string(),
                        CanoOutcome::NotLoggedIn { tail } => {
                            format!("finish:{}", String::from_utf8_lossy(&tail).trim())
                        }
                    };
                    on_finish.lock().push(label);
                }),
            },
            Box::new(move || on_exit.lock().push("exit".into())),
        )
        .unwrap();
    }

    fn wait_for(events: &Events, what: &str) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if events.lock().iter().any(|e| e == what) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        events.lock().clone()
    }

    #[test]
    fn marco_do_cano_avisa_o_login_e_a_saida_chega_depois_do_desfecho() {
        let pool = PtyPool::new();
        let events: Events = Arc::default();
        let marker = String::from_utf8(login_marker(NONCE)).unwrap();
        let script = format!("printf 'Last login\\r\\n'; printf '%s' '{marker}'; sleep 0.1");
        spawn_cano(&pool, PtyId::new_v4(), &script, &events);
        let seen = wait_for(&events, "exit");
        assert_eq!(seen, ["login", "finish:logged_in", "exit"]);
    }

    #[test]
    fn saida_sem_marco_entrega_o_que_o_ssh_escreveu() {
        let pool = PtyPool::new();
        let events: Events = Arc::default();
        spawn_cano(
            &pool,
            PtyId::new_v4(),
            "echo 'root@vps.example.test: Permission denied (publickey).'; exit 255",
            &events,
        );
        let seen = wait_for(&events, "exit");
        assert_eq!(
            seen,
            [
                "finish:root@vps.example.test: Permission denied (publickey).",
                "exit"
            ]
        );
    }

    #[test]
    fn religar_o_cano_no_mesmo_id_herda_janelas_e_tamanho() {
        let pool = PtyPool::new();
        let events: Events = Arc::default();
        let id = PtyId::new_v4();
        spawn_cano(&pool, id, "sleep 0.3", &events);
        pool.screen_of(id).unwrap().lock().attach("main");
        pool.resize(id, 132, 41).unwrap();
        wait_for(&events, "exit");

        let again: Events = Arc::default();
        spawn_cano(&pool, id, "stty size; sleep 0.3", &again);
        let screen = pool.screen_of(id).unwrap();
        assert!(
            screen.lock().attached(),
            "o pane que mostrava o Cano antigo continua recebendo a saída"
        );
        assert_eq!(pool.ptys.lock().get(&id).unwrap().size, (132, 41));
        wait_for(&again, "exit");
        let contents = screen.lock().parser.screen().contents();
        assert!(
            contents.contains("41 132"),
            "o PTY novo nasce no tamanho do pane: {contents:?}"
        );
    }
}

#[cfg(all(test, unix))]
mod transport_tests {
    use tauri::Listener;

    use super::tmux_control::{capture_pane, refresh_client, send_keys, CAPTURE_LINES};
    use super::*;

    const MARKER: &str = "\x1b]633;P;tyba-ctl=0123456789abcdef0123456789abcdef\x07";
    const TARGET: &str = "tyba-teste";

    fn kind() -> SessionKind {
        SessionKind::Ssh {
            host_id: "h".into(),
        }
    }

    /// Um tmux de mentira: escreve o prelúdio cru, troca para o protocolo e
    /// devolve cada comando recebido como saída do pane — é assim que o teste
    /// vê o que o transporte escreveu.
    fn fake_tmux() -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(format!(
            "printf 'Last login\\r\\n'; printf '{MARKER}'; \
             printf '%%output %%0 pronto\\\\015\\\\012\\n'; \
             while IFS= read -r linha; do \
               printf '%%output %%0 [%s]\\\\015\\\\012\\n' \"$linha\"; \
             done"
        ));
        cmd
    }

    fn control() -> Transport {
        Transport::TmuxControl {
            pane: TARGET.into(),
            session: TARGET.into(),
            control_marker: Some(MARKER.into()),
        }
    }

    fn spawn(pool: &PtyPool, id: PtyId, transport: Transport, cmd: CommandBuilder) {
        let app = tauri::test::mock_app();
        spawn_em(app.handle(), pool, id, transport, cmd);
    }

    /// O mesmo spawn com o app na mão de quem chama — é o que permite escutar
    /// os eventos da sessão.
    fn spawn_em(
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        pool: &PtyPool,
        id: PtyId,
        transport: Transport,
        cmd: CommandBuilder,
    ) {
        pool.spawn_with_transport(
            app.clone(),
            id,
            cmd,
            None,
            None,
            100,
            30,
            &kind(),
            None,
            transport,
            Box::new(|| {}),
        )
        .unwrap();
    }

    type Anuncios = Arc<Mutex<Vec<String>>>;

    /// Tudo que a sessão anunciar sobre o transporte, na ordem — assinado
    /// ANTES do spawn, que é a única forma de ouvir o estado inicial.
    fn escutar_transporte(app: &tauri::AppHandle<tauri::test::MockRuntime>, id: PtyId) -> Anuncios {
        let anuncios: Anuncios = Arc::default();
        let sink = Arc::clone(&anuncios);
        app.listen(format!("session://transport/{id}"), move |event| {
            let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            sink.lock()
                .push(payload["transport"].as_str().unwrap().to_string());
        });
        anuncios
    }

    fn esperar_anuncios(anuncios: &Anuncios, quantos: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while anuncios.lock().len() < quantos && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        anuncios.lock().clone()
    }

    fn wait_for_screen(pool: &PtyPool, id: PtyId, what: &str) -> String {
        let screen = pool.screen_of(id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let contents = screen.lock().parser.screen().contents();
            if contents.contains(what) || Instant::now() >= deadline {
                return contents;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn o_transporte_cru_entrega_o_byte_como_sempre() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("printf 'cru\\r\\n'; while IFS= read -r l; do printf '[%s]\\r\\n' \"$l\"; done");
        spawn(&pool, id, Transport::Raw, cmd);
        wait_for_screen(&pool, id, "cru");
        pool.write(id, b"oi\r").unwrap();
        let contents = wait_for_screen(&pool, id, "[oi]");
        assert!(contents.contains("cru"), "{contents:?}");
        assert!(
            contents.contains("[oi]"),
            "o byte escrito chega inteiro ao processo: {contents:?}"
        );
        pool.kill(id).unwrap();
    }

    #[test]
    fn o_preludio_cru_aparece_e_o_protocolo_nao() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        let contents = wait_for_screen(&pool, id, "pronto");
        assert!(
            contents.contains("Last login"),
            "o banner do ssh é byte cru: {contents:?}"
        );
        assert!(
            contents.contains("pronto"),
            "depois do marco, o %output vira tela: {contents:?}"
        );
        assert!(
            !contents.contains("%output"),
            "o protocolo nunca chega à tela: {contents:?}"
        );
        pool.kill(id).unwrap();
    }

    #[test]
    fn ao_trocar_de_protocolo_o_transporte_declara_o_tamanho() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        let contents = wait_for_screen(&pool, id, &refresh_client(100, 30));
        assert!(
            contents.contains(&refresh_client(100, 30)),
            "o cliente de controle não herda tamanho de tty: {contents:?}"
        );
        pool.kill(id).unwrap();
    }

    #[test]
    fn escrita_em_modo_de_controle_vira_send_keys() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        wait_for_screen(&pool, id, "pronto");
        pool.write(id, "é\r".as_bytes()).unwrap();
        let esperado = send_keys(TARGET, "é\r".as_bytes());
        let contents = wait_for_screen(&pool, id, &esperado);
        assert!(contents.contains(&esperado), "{contents:?}");
        pool.kill(id).unwrap();
    }

    #[test]
    fn redimensionar_em_modo_de_controle_vira_refresh_client() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        wait_for_screen(&pool, id, "pronto");
        pool.resize(id, 90, 25).unwrap();
        let contents = wait_for_screen(&pool, id, &refresh_client(90, 25));
        assert!(contents.contains(&refresh_client(90, 25)), "{contents:?}");
        assert_eq!(
            pool.ptys.lock().get(&id).unwrap().size,
            (90, 25),
            "o tamanho que o core guarda acompanha o pane"
        );
        pool.kill(id).unwrap();
    }

    #[test]
    fn redesenhar_pede_a_captura_ao_tmux() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        wait_for_screen(&pool, id, "pronto");
        pool.redraw_from_capture(id).unwrap();
        let esperado = capture_pane(TARGET, CAPTURE_LINES);
        let contents = wait_for_screen(&pool, id, &esperado);
        assert!(contents.contains(&esperado), "{contents:?}");
        pool.kill(id).unwrap();
    }

    /// Reatar chama o redesenho antes de o tmux existir: o pedido espera a
    /// troca de protocolo em vez de virar lixo no meio do prelúdio do ssh.
    #[test]
    fn redesenho_pedido_antes_do_marco_espera_o_protocolo_comecar() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(format!(
            "printf 'Last login\\r\\n'; sleep 0.5; printf '{MARKER}'; \
             while IFS= read -r linha; do \
               printf '%%output %%0 [%s]\\\\015\\\\012\\n' \"$linha\"; \
             done"
        ));
        spawn(&pool, id, control(), cmd);
        wait_for_screen(&pool, id, "Last login");
        pool.redraw_from_capture(id).unwrap();
        let esperado = capture_pane(TARGET, CAPTURE_LINES);
        let contents = wait_for_screen(&pool, id, &esperado);
        assert!(
            contents.contains(&esperado),
            "a captura sai depois do marco, nunca antes: {contents:?}"
        );
        pool.kill(id).unwrap();
    }

    /// O transporte contra um tmux de verdade, na máquina local e numa sessão
    /// descartável com socket próprio — nunca contra host remoto.
    ///
    /// `cargo test --lib pty::transport_tests::contra_o_tmux_de_verdade -- --ignored`
    #[test]
    #[ignore = "precisa do binário do tmux; não roda na suíte padrão"]
    fn contra_o_tmux_de_verdade_a_tela_vem_do_output_e_o_redesenho_da_captura() {
        let tmux = "/opt/homebrew/bin/tmux";
        assert!(
            std::path::Path::new(tmux).exists(),
            "sem tmux em {tmux} este teste não tem o que exercitar"
        );
        /// Servidor de tmux descartável: morre mesmo se o teste estourar no
        /// meio, para não deixar um `sh` pendurado na máquina do dono.
        struct ServidorDescartavel {
            tmux: &'static str,
            socket: String,
        }
        impl Drop for ServidorDescartavel {
            fn drop(&mut self) {
                let _ = std::process::Command::new(self.tmux)
                    .args(["-L", &self.socket, "kill-server"])
                    .status();
            }
        }

        let socket = format!("tyba-blk1-{}", Uuid::new_v4().simple());
        let _servidor = ServidorDescartavel {
            tmux,
            socket: socket.clone(),
        };
        let name = format!("tyba-blk1-{}", Uuid::new_v4().simple());

        let mut cmd = CommandBuilder::new(tmux);
        for arg in ["-L", &socket, "-C", "new-session", "-A", "-s", &name] {
            cmd.arg(arg);
        }
        cmd.arg("/bin/sh");
        cmd.env("TERM", "xterm-256color");

        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(
            &pool,
            id,
            Transport::TmuxControl {
                pane: name.clone(),
                session: name.clone(),
                // Sem prelúdio de ssh: o primeiro byte já é protocolo.
                control_marker: None,
            },
            cmd,
        );

        let prompt = wait_for_screen(&pool, id, "$");
        assert!(
            !prompt.contains("%output") && !prompt.contains("%begin"),
            "o protocolo não chega à tela: {prompt:?}"
        );
        pool.write(id, b"printf 'OLA-DO-TMUX\\n'\r").unwrap();
        let depois = wait_for_screen(&pool, id, "OLA-DO-TMUX");
        assert!(
            depois.matches("OLA-DO-TMUX").count() >= 2,
            "o eco do comando e a saída dele: {depois:?}"
        );

        // Reatar: o `capture-pane` é o único jeito de a tela voltar.
        pool.kill(id).unwrap();
        let id = PtyId::new_v4();
        let mut cmd = CommandBuilder::new(tmux);
        for arg in ["-L", &socket, "-C", "new-session", "-A", "-s", &name] {
            cmd.arg(arg);
        }
        cmd.arg("/bin/sh");
        spawn(
            &pool,
            id,
            Transport::TmuxControl {
                pane: name.clone(),
                session: name.clone(),
                control_marker: None,
            },
            cmd,
        );
        pool.redraw_from_capture(id).unwrap();
        let reatado = wait_for_screen(&pool, id, "OLA-DO-TMUX");
        assert!(
            reatado.contains("OLA-DO-TMUX"),
            "o redesenho traz o que a sessão já tinha: {reatado:?}"
        );
        assert!(
            !reatado.contains("%begin"),
            "o embrulho do bloco fica fora da tela: {reatado:?}"
        );

        pool.kill(id).unwrap();
    }

    #[test]
    fn a_senha_digitada_antes_do_marco_vai_crua_ao_ssh() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(format!(
            "printf 'Password: '; IFS= read -r senha; printf '<%s>\\r\\n' \"$senha\"; \
             printf '{MARKER}'; sleep 0.3"
        ));
        spawn(&pool, id, control(), cmd);
        wait_for_screen(&pool, id, "Password:");
        pool.write(id, b"segredo\r").unwrap();
        let contents = wait_for_screen(&pool, id, "<segredo>");
        assert!(
            contents.contains("<segredo>"),
            "antes do marco o transporte é cru — send-keys aqui seria lixo: {contents:?}"
        );
        pool.kill(id).unwrap();
    }

    /// O par de escrita de `prompt_mode`. Quem reata uma sessão integrada sabe
    /// que ela era integrada, mas o `capture-pane` devolve só o texto do pane:
    /// nenhum `633;P` volta com ele, e sem esta marca a aparência integrada só
    /// reapareceria depois do próximo prompt de verdade.
    #[test]
    fn marcar_o_modo_prompt_mexe_so_na_sessao_pedida() {
        let pool = PtyPool::new();
        let (um, outro) = (PtyId::new_v4(), PtyId::new_v4());
        spawn(&pool, um, control(), fake_tmux());
        spawn(&pool, outro, control(), fake_tmux());

        pool.set_prompt_mode(um, true);

        assert_eq!(pool.prompt_mode(um), Some(true));
        assert_eq!(
            pool.prompt_mode(outro),
            Some(false),
            "a marca é de uma sessão, não do pool"
        );
        pool.kill(um).unwrap();
        pool.kill(outro).unwrap();
    }

    /// Ligar o modo prompt é dizer que o `633;P` aconteceu — e é ele que abre o
    /// portão do editor de linha (ver [`LineEditorGate`]). Sem isso a primeira
    /// submissão numa sessão reatada esperaria o teto inteiro de
    /// `LINE_EDITOR_WAIT` antes de sair, num shell que já está no prompt há
    /// minutos.
    #[test]
    fn ligar_o_modo_prompt_abre_o_portao_do_editor_de_linha() {
        let pool = PtyPool::new();
        let (aberto, fechado) = (PtyId::new_v4(), PtyId::new_v4());
        spawn(&pool, aberto, control(), fake_tmux());
        spawn(&pool, fechado, control(), fake_tmux());

        pool.set_prompt_mode(aberto, true);
        pool.set_prompt_mode(fechado, false);

        assert!(pool
            .line_editor_gate(aberto)
            .unwrap()
            .wait_open(Duration::from_millis(10)));
        assert!(
            !pool
                .line_editor_gate(fechado)
                .unwrap()
                .wait_open(Duration::from_millis(10)),
            "desligar o modo prompt não afirma nada sobre o editor de linha"
        );
        pool.kill(aberto).unwrap();
        pool.kill(fechado).unwrap();
    }

    /// Critério novo: o religar NÃO repõe mais o modo prompt — quem repõe é o
    /// `SessionManager`, no marco de LOGIN, via [`PtyPool::set_prompt_mode`].
    ///
    /// Armadilha que mudou o critério: todo religar nasce cru, e a fase crua
    /// vem ANTES do login — banner do ssh, chave do host, pedido de senha. Com
    /// o modo prompt reposto já no spawn, o teclado pertence à linha de comando
    /// do TYBA, e num Host com senha o dono digitaria a senha numa caixa que a
    /// segura em vez de no prompt do `ssh`.
    #[test]
    fn religar_em_modo_de_controle_nao_repoe_o_modo_prompt() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        wait_for_screen(&pool, id, "pronto");
        pool.set_prompt_mode(id, true);
        pool.terminate(id).unwrap();

        spawn(&pool, id, control(), fake_tmux());

        assert_eq!(
            pool.prompt_mode(id),
            Some(false),
            "o modo prompt do religar volta no login, não no spawn"
        );
        assert!(
            !pool
                .line_editor_gate(id)
                .unwrap()
                .wait_open(Duration::from_millis(10)),
            "sem modo prompt reposto, o portão do editor de linha nasce fechado"
        );
        pool.kill(id).unwrap();
    }

    /// O mesmo critério pelo transporte cru: a sessão que volta COMUM também
    /// não herda a aparência integrada da anterior. Ali nasce shell novo, sem
    /// tmux nem hook, e um modo prompt herdado pintaria blocos de uma sessão
    /// que já não existe.
    #[test]
    fn religar_cru_nao_herda_o_modo_prompt_da_sessao_anterior() {
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        spawn(&pool, id, control(), fake_tmux());
        wait_for_screen(&pool, id, "pronto");
        pool.set_prompt_mode(id, true);
        pool.terminate(id).unwrap();

        let mut cru = CommandBuilder::new("/bin/sh");
        cru.arg("-c");
        cru.arg("printf 'comum\\r\\n'; sleep 0.3");
        spawn(&pool, id, Transport::Raw, cru);

        assert_eq!(pool.prompt_mode(id), Some(false));
        pool.kill(id).unwrap();
    }

    /// Quem responde consulta do terminal (DA, DSR, DECRQM) precisa saber por
    /// onde a sessão fala: em modo de controle o tmux remoto já responde
    /// sozinho E ainda encaminha a consulta crua ao cliente, então uma segunda
    /// resposta só pode voltar como `send-keys` — ou seja, vira digitação no
    /// pane. O front não tem como descobrir isso; o core anuncia.
    #[test]
    fn a_sessao_crua_anuncia_o_transporte_no_spawn() {
        let app = tauri::test::mock_app();
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        let anuncios = escutar_transporte(app.handle(), id);

        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("printf 'cru\\r\\n'; sleep 0.3");
        spawn_em(app.handle(), &pool, id, Transport::Raw, cmd);

        assert_eq!(esperar_anuncios(&anuncios, 1), ["raw"]);
        pool.kill(id).unwrap();
    }

    /// A sessão integrada nasce crua — banner do ssh, pedido de senha — e só
    /// vira modo de controle no marco. O anúncio acompanha as duas fases.
    #[test]
    fn a_troca_de_protocolo_anuncia_o_modo_de_controle() {
        let app = tauri::test::mock_app();
        let pool = PtyPool::new();
        let id = PtyId::new_v4();
        let anuncios = escutar_transporte(app.handle(), id);

        spawn_em(app.handle(), &pool, id, control(), fake_tmux());

        assert_eq!(esperar_anuncios(&anuncios, 2), ["raw", "tmux_control"]);
        pool.kill(id).unwrap();
    }
}

#[cfg(test)]
mod screen_tests {
    #[test]
    fn parser_rastreia_o_estado_de_bracketed_paste_do_programa() {
        let mut parser = vt100::Parser::new(24, 80, super::SCROLLBACK_LINES);
        assert!(!parser.screen().bracketed_paste());
        parser.process(b"\x1b[?2004h");
        assert!(parser.screen().bracketed_paste());
        parser.process(b"\x1b[?2004l");
        assert!(!parser.screen().bracketed_paste());
    }

    #[test]
    fn snapshot_preserves_visible_text() {
        let mut parser = vt100::Parser::new(24, 80, super::SCROLLBACK_LINES);
        parser.process(b"hello \x1b[31mred\x1b[0m world");
        let dump = parser.screen().contents_formatted();
        let text = String::from_utf8_lossy(&dump);
        assert!(text.contains("hello"));
        assert!(text.contains("red"));
        assert!(text.contains("world"));
    }
}

/// A tela alternativa atravessando o transporte de controle inteiro: protocolo
/// do tmux → retenção de OSC → máquina de captura → fila do webview.
///
/// O fluxo é o do `tmux -C` de verdade, gravado contra o VPS do dono em
/// 2026-09-18 com um cliente de controle puro (sem o app no meio) enquanto um
/// tmux ANINHADO subia e saía dentro da sessão integrada. Os marcadores `133`
/// são da integração local — o cliente da gravação rodava sem o rc do TYBA.
#[cfg(test)]
mod alt_screen_tests {
    use super::*;

    const COLS: u16 = 100;
    const ROWS: u16 = 38;

    /// O tmux aninhado entrando em tela alternativa. Repare no `\033[1;24r`:
    /// a região de rolagem que ele declara é a do tamanho que ELE conhece.
    const ENTRA_ALT: &str = "%output %0 \\033[?1049h\\033[?1h\\033=\\033[H\\033[J\\033[34h\\033[?25h\\033[?1000l\\033[?1002l\\033[?1003l\\033[?1006l\\033[?1005l\\033[?2004h\\033[m\\017\\033[34h\\033[?25h\\033[?1006l\\033[?1000l\\033[?1002l\\033[?1003l\\033[1;1H\\033[1;24r\\033[c\\033[>c\\033[>q\\033]10;?\\033\\134\\033]11;?\\033\\134\\033[1;1H\\033[?25l\\033[K\\015\\012\\033[K\\015\\012\\033[K\\015\\012\\033[K\\015\\012\\033[K\\015\\012\\033[K\\015\\012\\033[K\\015\\012\\033[K\\033[30m\\033[42m\\015\\012[1] 0:bash*                                         \"srv1084118\" 14:06 18-Sep-26\\033[m\\017\\033[34h\\033[?25h\\033[1;1H\n";

    /// A saída dele — `\033[1;24r` de novo e o `\033[?1049l` no fim.
    const SAI_ALT: &str = "%output %0 \\033[1;24r\\033[m\\017\\033[?1l\\033>\\033[H\\033[J\\033[34h\\033[?25h\\033[?1000l\\033[?1002l\\033[?1003l\\033[?1006l\\033[?1005l\\033[?2004l\\033[?7727l\\033[?1004l\\033[?1049l\n";

    /// O que o tmux imprime ao fechar, e o prompt do servidor logo atrás.
    const VOLTA_O_PROMPT: &str =
        "%output %0 [exited]\\015\\012\\033[?2004hroot@srv1084118:~# \\033]133;D;0\\007\\033]133;A\\007\n";

    /// O caminho de leitura de uma sessão integrada, do byte do socket até o
    /// que o webview desenha — o xterm.js do front modelado por um `vt100` do
    /// mesmo tamanho do pane.
    struct Pane {
        decoder: tmux_control::ControlDecoder,
        hold_back: holdback::HoldBack,
        state: ScreenState,
        machine: capture::CaptureMachine,
        webview: vt100::Parser,
    }

    impl Pane {
        fn new() -> Self {
            let mut state = ScreenState::new(ROWS, COLS);
            state.attach("janela");
            Self {
                decoder: tmux_control::ControlDecoder::new(),
                hold_back: holdback::HoldBack::new(),
                state,
                machine: capture::CaptureMachine::new("sessao".into()),
                webview: vt100::Parser::new(ROWS, COLS, 0),
            }
        }

        /// Uma leitura do PTY, exatamente como a thread leitora a trata.
        fn read(&mut self, bytes: &[u8]) {
            let mut decoded = Vec::new();
            for event in self.decoder.feed(bytes) {
                if let tmux_control::ControlEvent::Output(chunk) = event {
                    decoded.extend_from_slice(&chunk);
                }
            }
            if decoded.is_empty() {
                return;
            }
            let ready = self.hold_back.feed(&decoded);
            if ready.is_empty() {
                return;
            }
            ingest_chunk(&mut self.state, &mut self.machine, &ready, 0);
            if let Some(pending) = self.state.take_pending() {
                self.webview.process(&pending);
            }
        }

        /// O que o webview vê, linha a linha.
        fn linhas(&self) -> Vec<String> {
            self.webview
                .screen()
                .contents()
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    /// Sessão integrada com o modo prompt ligado, o dono no prompt do servidor
    /// e o `tmux` aninhado submetido — o estado de onde o defeito parte.
    fn no_tmux_aninhado() -> Pane {
        let mut pane = Pane::new();
        pane.read(b"%output %0 \\033]633;P;tyba-prompt=1\\007\\033]133;A\\007root@srv1084118:~# \\033]133;B\\007\n");
        pane.read(b"%output %0 tmux\\015\\012\n");
        pane.read(b"%output %0 \\033]633;E;dG11eA==\\007\\033]133;C\\007\n");
        pane.read(ENTRA_ALT.as_bytes());
        assert!(
            pane.webview.screen().alternate_screen(),
            "o aninhado subiu: o webview está em tela alternativa"
        );
        pane
    }

    /// O critério em jogo: o tmux do dono aninhado dentro da sessão integrada
    /// continua funcionando — sair dele devolve a tela normal ao webview.
    ///
    /// O `?1049l` e o `133;D` chegam na MESMA leitura de propósito: é o que o
    /// transporte de controle produz (uma leitura do socket carrega vários
    /// `%output`), e é onde o defeito mora.
    #[test]
    fn sair_do_tmux_aninhado_tira_o_webview_da_tela_alternativa() {
        let mut pane = no_tmux_aninhado();
        pane.read(format!("{SAI_ALT}{VOLTA_O_PROMPT}").as_bytes());
        assert!(
            !pane.webview.screen().alternate_screen(),
            "o webview ficou preso na tela alternativa: teclado do app, tela em branco"
        );
    }

    /// O mesmo, com a leitura partida NO MEIO da sequência de saída — o corte
    /// que uma leitura de socket produz naturalmente e que some em teste.
    #[test]
    fn a_saida_partida_no_meio_ainda_devolve_a_tela_normal() {
        let saida = SAI_ALT.as_bytes();
        let corte = saida.len() / 2;
        let mut pane = no_tmux_aninhado();
        pane.read(&saida[..corte]);
        pane.read(format!("{}{VOLTA_O_PROMPT}", &SAI_ALT[corte..]).as_bytes());
        assert!(
            !pane.webview.screen().alternate_screen(),
            "o payload partido não pode perder a volta da tela alternativa"
        );
    }

    /// Não basta o booleano virar: a região de rolagem que o aninhado deixou
    /// (`1;24` num pane de 38 linhas) tem de voltar a cobrir o pane inteiro,
    /// senão a saída seguinte rola dentro de uma janela de 24 linhas e as de
    /// baixo ficam congeladas.
    #[test]
    fn depois_da_saida_a_rolagem_cobre_o_pane_inteiro() {
        let mut pane = no_tmux_aninhado();
        pane.read(format!("{SAI_ALT}{VOLTA_O_PROMPT}").as_bytes());

        // Sem quebra de linha no fim: a última escrita tem de ficar na última
        // LINHA do pane, e uma quebra sobrando deixaria o cursor numa linha
        // vazia que o `contents()` apara.
        let ultima = ROWS + 1;
        let mut leitura = b"%output %0 ".to_vec();
        for i in 0..=ultima {
            leitura.extend_from_slice(format!("L{i}").as_bytes());
            if i < ultima {
                leitura.extend_from_slice(b"\\015\\012");
            }
        }
        leitura.push(b'\n');
        pane.read(&leitura);

        let linhas = pane.linhas();
        assert_eq!(
            linhas.last().map(String::as_str),
            Some(format!("L{ultima}").as_str()),
            "a última linha escrita fica na última linha do pane: {linhas:?}"
        );
        assert_eq!(
            linhas.len(),
            usize::from(ROWS),
            "o pane rolou inteiro, sem linhas congeladas fora da região: {linhas:?}"
        );
    }
}

#[cfg(all(test, unix))]
mod echo_tests {
    use portable_pty::{native_pty_system, PtySize};

    /// Lê o `ECHO` como `PtyPool::line_echo` lê, mas de um fd solto — o pool
    /// exige uma sessão inteira, e o que está sob teste é a leitura da flag.
    fn echo_of(fd: std::os::unix::io::RawFd) -> Option<bool> {
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } != 0 {
            return None;
        }
        let termios = unsafe { termios.assume_init() };
        Some(termios.c_lflag & libc::ECHO != 0)
    }

    fn set_echo(fd: std::os::unix::io::RawFd, on: bool) {
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        assert_eq!(unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) }, 0);
        let mut termios = unsafe { termios.assume_init() };
        if on {
            termios.c_lflag |= libc::ECHO;
        } else {
            termios.c_lflag &= !libc::ECHO;
        }
        assert_eq!(unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) }, 0);
    }

    /// O sinal que decide para onde vai a seta precisa acompanhar a troca de
    /// modo em tempo real: o `npm create` começa canônico (`Ok to proceed?`) e
    /// vira raw quando abre o menu, dentro do MESMO comando. Um valor lido uma
    /// vez no início mandaria a seta para o lado errado da metade em diante.
    #[test]
    fn line_echo_acompanha_o_modo_do_tty() {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let fd = pair.master.as_raw_fd().expect("master fd");

        // Um pty nasce canônico com eco — é o estado em que o shell espera
        // comando, e aquele em que a seta vira `^[[A` na saída.
        assert_eq!(echo_of(fd), Some(true), "pty novo nasce com eco");

        // Raw: quem lê tecla a tecla desliga o eco justamente para tratar as
        // setas por conta própria.
        set_echo(fd, false);
        assert_eq!(echo_of(fd), Some(false));

        set_echo(fd, true);
        assert_eq!(echo_of(fd), Some(true), "volta ao canônico ao fim");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::kill_process_group;
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn is_alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_dead(pid: i32, within: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < within {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !is_alive(pid)
    }

    #[test]
    fn kills_entire_process_group() {
        let mut leader = unsafe {
            Command::new("sh")
                .arg("-c")
                .arg("sleep 60 & echo $$ $!; wait")
                .stdout(Stdio::piped())
                .pre_exec(|| {
                    libc::setsid();
                    Ok(())
                })
                .spawn()
                .expect("spawn leader")
        };

        let stdout = leader.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read pids");

        let mut ids = line.split_whitespace();
        let leader_pid: i32 = ids.next().unwrap().parse().unwrap();
        let child_pid: i32 = ids.next().unwrap().parse().unwrap();

        assert!(is_alive(leader_pid));
        assert!(is_alive(child_pid));

        kill_process_group(leader_pid as u32).expect("kill group");

        assert!(
            wait_dead(child_pid, Duration::from_secs(2)),
            "child survived group kill"
        );

        let status = leader.wait().expect("reap leader");
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "leader not killed by SIGKILL"
        );
    }

    fn pooled_session(pool: &super::PtyPool) -> (i32, i32) {
        use portable_pty::native_pty_system;
        use std::sync::Arc;

        let pair = native_pty_system()
            .openpty(super::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut cmd = super::CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg(r#"(trap "" HUP; sleep 60) & echo $$ $!; wait"#);
        let child = pair.slave.spawn_command(cmd).expect("spawn in pty");
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().expect("reader");
        let mut line = String::new();
        BufReader::new(reader).read_line(&mut line).expect("pids");
        let mut ids = line.split_whitespace();
        let leader_pid: i32 = ids.next().unwrap().parse().unwrap();
        let child_pid: i32 = ids.next().unwrap().parse().unwrap();

        let writer = pair.master.take_writer().expect("writer");
        let handle = super::PtyHandle {
            master: pair.master,
            writer: Arc::new(parking_lot::Mutex::new(writer)),
            leader_pid: child.process_id(),
            child,
            leader_start: None,
            screen: Arc::new(parking_lot::Mutex::new(super::ScreenState::new(24, 80))),
            size: (80, 24),
            // Sem thread emissora nesta fixture: ponta fraca sem dono, que
            // nunca sobe — é o que se quer aqui, porque o teste é de derrubar
            // árvore de processo, não de tela.
            nudge: std::sync::Weak::new(),
            transport: super::Transport::Raw,
            control: None,
        };
        pool.ptys.lock().insert(uuid::Uuid::new_v4(), handle);

        (leader_pid, child_pid)
    }

    #[test]
    fn hook_expected_distingue_sem_pty_de_sem_hook() {
        // `false` sozinho é ambíguo, e é por isso que este campo existe: sem
        // ele a interface não separa um `zsh` que ainda está carregando o `rc`
        // de um `fish` que jamais vai reportar `633;P`.
        let pool = super::PtyPool::new();
        let (_leader, _child) = pooled_session(&pool);
        let id = *pool.ptys.lock().keys().next().expect("sessão no pool");

        // Nasce falso: quem sabe qual shell subiu é o SessionManager.
        assert_eq!(pool.hook_expected(id), Some(false));

        pool.set_hook_expected(id, true);
        assert_eq!(pool.hook_expected(id), Some(true));

        pool.set_hook_expected(id, false);
        assert_eq!(pool.hook_expected(id), Some(false));

        // Sessão que não existe é `None`, e não `Some(false)`: o comando de IPC
        // achata os dois em `false`, mas aqui a diferença tem de sobreviver.
        let ausente = super::PtyId::new_v4();
        assert_eq!(pool.hook_expected(ausente), None);

        pool.kill_all();
    }

    #[test]
    fn kill_all_takes_every_session_tree_down() {
        let pool = super::PtyPool::new();
        let (first_leader, first_child) = pooled_session(&pool);
        let (second_leader, second_child) = pooled_session(&pool);

        for pid in [first_leader, first_child, second_leader, second_child] {
            assert!(is_alive(pid), "session {pid} not alive before kill_all");
        }

        pool.kill_all();

        for pid in [first_child, second_child] {
            assert!(
                wait_dead(pid, Duration::from_secs(2)),
                "agent tree survived kill_all: {pid}"
            );
        }
        assert!(
            pool.ptys.lock().is_empty(),
            "pool kept handles after kill_all"
        );
    }
}
