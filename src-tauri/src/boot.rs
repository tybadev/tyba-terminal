//! Arranque do core: o sinal de "estado carregado" e a instrumentação do boot.
//!
//! O closure `.setup()` do Tauri roda na **main thread**, dentro do callback
//! `Ready`, com o event loop parado até ele retornar — e as janelas declaradas
//! no `tauri.conf.json` já estão na tela nesse instante, porque o Tauri as cria
//! antes de chamar o setup. Enquanto o closure não devolve, o webview não
//! consegue nem servir o `index.html` (o protocolo `tauri://` usa o mesmo
//! handler de main thread): a janela aparece congelada, com beachball.
//!
//! Por isso o setup ficou com o mínimo que **precisa** da main thread (menu,
//! janela) e o resto do arranque — SQLite, `ssh -G`, reabertura de sessão,
//! layout, checkpoints — corre na thread de boot. Quem observa o fim dela é o
//! [`BootGate`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

/// Emitido para o webview quando a thread de boot termina. Payload: nenhum — o
/// front reconsulta `boot_snapshot` ao receber.
pub const EVENT_READY: &str = "app://ready";

/// Emitido quando a thread de boot morreu de pânico ANTES de abrir o portão.
/// Payload: `{ message: string }`, a mensagem do pânico.
///
/// Vem sempre seguido de [`EVENT_READY`]: o portão abre de todo jeito (senão
/// todo comando de escrita paga o timeout de espera e a falha vira lentidão), e
/// o snapshot que o front reconsultar estará incompleto — sem sessões, sem
/// layout, ou pela metade. É este evento que diz que o vazio é falha, não
/// ausência de dado.
///
/// **O evento é o caminho rápido, não a garantia.** Ele sai de dentro da thread
/// de boot, que começa no `.setup()` — antes de o webview carregar —, e o
/// `listen()` do Tauri é assíncrono: um pânico logo no começo dispara antes de o
/// listener existir, e o core não reemite. A rede é [`BootGate::failure`], que o
/// `boot_snapshot` devolve a cada consulta; o mesmo desenho do `app://ready`,
/// que tem o poll do front como segunda via.
pub const EVENT_FAILED: &str = "app://boot-failed";

/// "O estado do core terminou de carregar?"
///
/// `AtomicBool` + `Condvar` em vez de `tokio::sync::Notify` porque a maioria dos
/// consumidores é síncrona (comandos que só querem *perguntar*, sem `.await`), e
/// em vez de `Once` porque `Once` não separa "perguntar" de "executar": não há
/// como consultar o estado sem se comprometer a rodar a inicialização.
///
/// As duas operações têm papéis distintos e não se substituem: [`is_ready`] é
/// para leitura — o comando responde `ready: false` e o front espera o evento —,
/// e [`wait_ready`] é para escrita, onde agir antes da hora corromperia o que a
/// thread de boot ainda vai carregar por cima.
///
/// O portão guarda também a mensagem da falha, quando houve — ver
/// [`mark_failed`] e [`failure`]. É o mesmo `Mutex` do `Condvar`: quem escreve
/// grava a mensagem e abre o portão sob um lock só, e assim ninguém observa
/// `ready: true` com a falha ainda em branco.
///
/// [`is_ready`]: BootGate::is_ready
/// [`wait_ready`]: BootGate::wait_ready
/// [`mark_failed`]: BootGate::mark_failed
/// [`failure`]: BootGate::failure
pub struct BootGate {
    ready: AtomicBool,
    /// `Mutex` do `Condvar` e cofre da mensagem de falha, no mesmo lugar de
    /// propósito: é o que torna "gravar a falha" e "abrir o portão" indivisíveis
    /// para quem observa.
    failure: Mutex<Option<String>>,
    changed: Condvar,
}

impl Default for BootGate {
    fn default() -> Self {
        Self::new()
    }
}

impl BootGate {
    pub fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            failure: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn mark_ready(&self) {
        let _guard = self.failure.lock();
        self.ready.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    /// A thread de boot morreu antes de terminar: grava a mensagem e abre o
    /// portão, nessa ordem e sob o mesmo lock.
    ///
    /// A ordem é a decisão. O `store(Release)` publica a escrita anterior, então
    /// quem lê `is_ready() == true` — e o `boot_snapshot` lê o `ready` primeiro,
    /// de propósito — já enxerga a mensagem. Na ordem inversa haveria um
    /// instante em que o snapshot diria "carregou, e sem falha" sobre um app que
    /// já tinha morrido, e o front trataria como final o vazio que era falha.
    ///
    /// O portão abre porque falha com portão fechado é pior que falha: todo
    /// comando de escrita pagaria [`wait_ready`] inteiro antes de agir, e o
    /// usuário veria lentidão em vez de erro.
    ///
    /// A primeira mensagem manda: quem morreu primeiro explica o resto.
    ///
    /// [`wait_ready`]: BootGate::wait_ready
    pub fn mark_failed(&self, message: impl Into<String>) {
        let mut failure = self.failure.lock();
        if failure.is_none() {
            *failure = Some(message.into());
        }
        self.ready.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    /// A mensagem da falha, se o boot morreu antes de terminar.
    ///
    /// Segunda via do [`EVENT_FAILED`]. O evento é emitido de dentro da thread
    /// de boot, que começa antes de o webview carregar, e o `listen()` do Tauri
    /// é assíncrono: um pânico cedo dispara antes de o listener existir e o core
    /// não reemite. Quem reconsulta o estado encontra a falha aqui.
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().clone()
    }

    /// Segura o chamador até o boot terminar. Devolve `false` no timeout — que é
    /// um teto de sanidade, não um caminho esperado: preferimos agir com estado
    /// pela metade a travar a UI para sempre se a thread de boot morreu.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        if self.is_ready() {
            return true;
        }
        let deadline = Instant::now() + timeout;
        let mut guard = self.failure.lock();
        while !self.is_ready() {
            if self.changed.wait_until(&mut guard, deadline).timed_out() {
                return self.is_ready();
            }
        }
        true
    }
}

/// `TYBA_BOOT_TRACE=1` liga o trace. Fora isso o boot não imprime nada: medir o
/// arranque não pode custar linha de log em uso normal.
pub fn trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TYBA_BOOT_TRACE").is_some_and(|value| value == "1"))
}

/// Trecho cronometrado do boot. Só imprime quando fechado — um span vazado é
/// medição perdida, daí o `must_use`.
#[must_use = "o span só imprime quando é fechado com end()/end_with()"]
pub struct Span {
    label: &'static str,
    start: Instant,
}

impl Span {
    pub fn start(label: &'static str) -> Self {
        Self {
            label,
            start: Instant::now(),
        }
    }

    pub fn end(self) {
        self.report(None);
    }

    /// `detail` entra entre parênteses — é onde vai a contagem que explica o
    /// número (quantas sessões foram reabertas, p.ex.).
    pub fn end_with(self, detail: impl std::fmt::Display) {
        self.report(Some(detail.to_string()));
    }

    fn report(&self, detail: Option<String>) {
        if !trace_enabled() {
            return;
        }
        let ms = self.start.elapsed().as_secs_f64() * 1000.0;
        match detail {
            Some(detail) => eprintln!("[tyba boot] {:<24} {ms:>8.1}ms  {detail}", self.label),
            None => eprintln!("[tyba boot] {:<24} {ms:>8.1}ms", self.label),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_starts_closed_and_opens_once() {
        let gate = BootGate::new();
        assert!(!gate.is_ready());
        gate.mark_ready();
        assert!(gate.is_ready());
        assert!(gate.wait_ready(Duration::from_millis(0)));
    }

    #[test]
    fn wait_ready_times_out_while_boot_is_pending() {
        let gate = BootGate::new();
        assert!(!gate.wait_ready(Duration::from_millis(10)));
    }

    #[test]
    fn a_gate_that_opened_clean_reports_no_failure() {
        let gate = BootGate::new();
        assert_eq!(gate.failure(), None);
        gate.mark_ready();
        assert_eq!(gate.failure(), None);
    }

    /// A segunda via do [`EVENT_FAILED`]: a mensagem fica no portão, e quem
    /// perdeu o evento a encontra perguntando.
    #[test]
    fn the_failure_stays_readable_after_the_gate_opens() {
        let gate = BootGate::new();
        gate.mark_failed("sessions.restore explodiu");
        // Abre o portão junto — falha com portão fechado vira lentidão.
        assert!(gate.is_ready());
        assert_eq!(gate.failure().as_deref(), Some("sessions.restore explodiu"));
    }

    /// Quem observa `ready: true` tem de observar a falha junto: se a ordem
    /// fosse a inversa, o front leria "carregou" com a mensagem ainda em branco
    /// e trataria como final um vazio que era falha.
    #[test]
    fn the_failure_lands_before_the_gate_opens() {
        let gate = std::sync::Arc::new(BootGate::new());
        let boot = std::sync::Arc::clone(&gate);
        std::thread::spawn(move || boot.mark_failed("morreu"));
        while !gate.is_ready() {
            std::hint::spin_loop();
        }
        assert_eq!(gate.failure().as_deref(), Some("morreu"));
    }

    /// O `guard_boot` chama `mark_ready` no caminho de unwind. A mensagem já
    /// gravada não pode sumir por causa disso.
    #[test]
    fn mark_ready_after_a_failure_does_not_erase_it() {
        let gate = BootGate::new();
        gate.mark_failed("pânico");
        gate.mark_ready();
        assert_eq!(gate.failure().as_deref(), Some("pânico"));
    }

    /// Quem espera para ESCREVER precisa acordar quando o boot morre — senão a
    /// falha volta a ser o teto de espera inteiro.
    #[test]
    fn wait_ready_wakes_on_mark_failed() {
        let gate = std::sync::Arc::new(BootGate::new());
        let writer = std::sync::Arc::clone(&gate);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            writer.mark_failed("morreu");
        });
        assert!(gate.wait_ready(Duration::from_secs(5)));
    }

    #[test]
    fn wait_ready_wakes_on_mark_ready() {
        let gate = std::sync::Arc::new(BootGate::new());
        let writer = std::sync::Arc::clone(&gate);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            writer.mark_ready();
        });
        assert!(gate.wait_ready(Duration::from_secs(5)));
    }
}
