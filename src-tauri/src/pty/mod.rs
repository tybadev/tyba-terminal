use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

mod capture;
mod holdback;

#[cfg(target_os = "windows")]
pub mod conpty_jailed;

pub const EVENT_CWD_CHANGED: &str = "session://cwd-changed";

const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const READ_BUF_SIZE: usize = 8 * 1024;
const SCROLLBACK_LINES: usize = 1000;
const CHANNEL_CAPACITY: usize = 128;

pub type PtyId = Uuid;

struct ScreenState {
    parser: vt100::Parser,
    pending: Vec<u8>,
    attachers: HashMap<String, usize>,
    /// Última resposta do shell sobre o modo prompt (`633;P`). Guardada para
    /// poder ser CONSULTADA: evento só é entregue a quem já estava ouvindo.
    prompt_mode: bool,
}

impl ScreenState {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
            pending: Vec::with_capacity(READ_BUF_SIZE),
            attachers: HashMap::new(),
            prompt_mode: false,
        }
    }

    fn attached(&self) -> bool {
        !self.attachers.is_empty()
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

fn emit_pending(state: &mut ScreenState, app: &AppHandle, event: &str) {
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

#[derive(Clone, Serialize)]
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

struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    leader_pid: Option<u32>,
    leader_start: Option<u64>,
    screen: SharedScreen,
    size: (u16, u16),
}

#[derive(Default)]
pub struct PtyPool {
    ptys: Mutex<HashMap<PtyId, PtyHandle>>,
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
                state.parser.process(CLEAR_SCREEN);
                state.pending.clear();
                if state.attached() {
                    state.pending.extend_from_slice(CLEAR_SCREEN);
                }
            }
            capture::Action::PromptMode(on) => state.prompt_mode = *on,
            _ => {}
        }
    }
}

/// O lado não-visual das ações: eventos para o webview, histórico e blocos.
/// Fora do lock de tela — `emit` atravessa IPC e não pode segurar o terminal.
struct ActionSink {
    app: AppHandle,
    session_id: PtyId,
    command_event: String,
    cwd_event: String,
    prompt_mode_event: String,
}

impl ActionSink {
    /// Devolve `true` quando um comando começou — é o sinal para reiniciar o
    /// relógio do checkpoint.
    fn run(&self, actions: Vec<capture::Action>, cols: u16, rows: u16) -> bool {
        let mut started = false;
        for action in actions {
            match action {
                capture::Action::Running {
                    command,
                    agent_match,
                } => {
                    started = true;
                    let _ = self.app.emit(
                        &self.command_event,
                        SessionCommandPayload {
                            command,
                            running: true,
                            agent_match,
                            continuation: false,
                        },
                    );
                }
                capture::Action::Idle => {
                    let _ = self.app.emit(
                        &self.command_event,
                        SessionCommandPayload {
                            command: None,
                            running: false,
                            agent_match: false,
                            continuation: false,
                        },
                    );
                }
                capture::Action::Continuation(on) => {
                    let _ = self.app.emit(
                        &self.command_event,
                        SessionCommandPayload {
                            command: None,
                            running: false,
                            agent_match: false,
                            continuation: on,
                        },
                    );
                }
                capture::Action::Record(record) => crate::history::record(record),
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

    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        app: AppHandle,
        session_id: PtyId,
        mut cmd: CommandBuilder,
        env: Option<&HashMap<String, String>>,
        jail: Option<Box<dyn JailedSpawner>>,
        cols: u16,
        rows: u16,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<(), PtyError> {
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
        let writer = master
            .take_writer()
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let screen: SharedScreen = Arc::new(Mutex::new(ScreenState::new(rows, cols)));
        let reader_screen = Arc::clone(&screen);

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
            },
        );

        let output_event = format!("pty://output/{session_id}");
        let exit_event = format!("pty://exit/{session_id}");
        let command_event = format!("session://command/{session_id}");
        let cwd_event = format!("session://cwd/{session_id}");
        let bracketed_event = format!("session://bracketed/{session_id}");
        let prompt_mode_event = format!("session://prompt-mode/{session_id}");
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);

        std::thread::Builder::new()
            .name(format!("pty-reader-{session_id}"))
            .spawn(move || {
                let mut buf = [0u8; READ_BUF_SIZE];
                let mut hold_back = holdback::HoldBack::new();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let ready = hold_back.feed(&buf[..n]);
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
                let sink = ActionSink {
                    app: app.clone(),
                    session_id,
                    command_event,
                    cwd_event,
                    prompt_mode_event,
                };

                loop {
                    let chunk = if !queued {
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
                            // O vt100 do core vê o chunk INTEIRO: o recorte da
                            // máquina decide o que vira bloco, não o que o
                            // terminal desenha.
                            let alt_screen = {
                                let mut screen = reader_screen.lock();
                                screen.parser.process(&chunk);
                                screen.parser.screen().alternate_screen()
                            };
                            let actions = machine.on_chunk(&chunk, now_ms(), alt_screen);
                            let (bracketed, cols, rows) = {
                                let mut screen = reader_screen.lock();
                                apply_screen(&mut screen, &chunk, &actions);
                                if due {
                                    emit_pending(&mut screen, &app, &output_event);
                                }
                                queued = !screen.pending.is_empty();
                                let (rows, cols) = screen.parser.screen().size();
                                (screen.parser.screen().bracketed_paste(), cols, rows)
                            };
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
                            {
                                let mut screen = reader_screen.lock();
                                emit_pending(&mut screen, &app, &output_event);
                                queued = false;
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
                let _ = app.emit(&exit_event, PtyExitPayload { code: None });
                on_exit();
            })
            .map_err(|e| {
                let _ = self.kill(session_id);
                PtyError::Spawn(format!("pty emitter thread: {e}"))
            })?;

        Ok(())
    }

    pub fn write(&self, id: PtyId, data: &[u8]) -> Result<(), PtyError> {
        let mut ptys = self.ptys.lock();
        let handle = ptys.get_mut(&id).ok_or(PtyError::NotFound(id))?;
        handle.writer.write_all(data)?;
        handle.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: PtyId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let mut ptys = self.ptys.lock();
        let handle = ptys.get_mut(&id).ok_or(PtyError::NotFound(id))?;
        if handle.size == (cols, rows) {
            return Ok(());
        }
        handle
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Open(e.to_string()))?;
        handle.size = (cols, rows);
        handle.screen.lock().parser.set_size(rows, cols);
        Ok(())
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
            writer,
            leader_pid: child.process_id(),
            child,
            leader_start: None,
            screen: Arc::new(parking_lot::Mutex::new(super::ScreenState::new(24, 80))),
            size: (80, 24),
        };
        pool.ptys.lock().insert(uuid::Uuid::new_v4(), handle);

        (leader_pid, child_pid)
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
