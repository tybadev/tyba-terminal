//! Janela de splash do arranque.
//!
//! O `tauri.conf.json` declara duas janelas: `splash` (pequena, sem borda,
//! centrada, visível) e `main` (escondida). O splash aparece no instante em que
//! o processo sobe; a `main` só é mostrada quando o front avisa que o app está
//! pronto — o mesmo sinal que antes tirava o splash de dentro do webview (o
//! `SPLASH_DONE_EVENT`, que o `App` dispara quando o core termina de carregar).
//!
//! O aviso vem pelo comando [`splash_done`]. A rede é o [`CEILING`], armado no
//! `.setup()`: o core pode estar parado num diálogo de permissão do macOS, ou o
//! webview pode nem ter carregado, e uma janela principal que nunca aparece é
//! pior que uma UI vazia. Quem chegar primeiro fecha; o segundo é no-op.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};

pub const SPLASH_LABEL: &str = "splash";
const MAIN_LABEL: &str = "main";

/// Teto de espera do splash.
///
/// "Pronto" depende do core, e o core pode ficar parado num diálogo de
/// permissão do macOS que segura a thread até alguém clicar. Sem teto, um boot
/// que trava deixa o usuário olhando um logo — e, com a `main` escondida, sem
/// janela nenhuma para interagir.
pub const CEILING: Duration = Duration::from_secs(4);

/// Garante que a troca splash → `main` acontece uma vez só, venha ela do front
/// ou do teto — e de qualquer janela: toda janela nova roda o mesmo `App` e
/// dispara o mesmo aviso.
#[derive(Default)]
pub struct SplashLatch(AtomicBool);

impl SplashLatch {
    /// `true` só para o primeiro chamador.
    pub fn take(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
}

/// Mostra a `main` e fecha o splash. Idempotente.
///
/// A `main` aparece ANTES de o splash fechar: no Windows e no Linux, fechar a
/// última janela visível com a outra ainda escondida pode encerrar o app.
pub fn finish<R: Runtime>(app: &AppHandle<R>) {
    let Some(latch) = app.try_state::<SplashLatch>() else {
        return;
    };
    if !latch.take() {
        return;
    }
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.show();
        let _ = main.set_focus();
    }
    if let Some(splash) = app.get_webview_window(SPLASH_LABEL) {
        let _ = splash.close();
    }
}

/// Arma o teto: passado [`CEILING`], a `main` aparece de todo jeito.
pub fn arm_ceiling<R: Runtime>(app: AppHandle<R>) {
    std::thread::Builder::new()
        .name("splash-ceiling".into())
        .spawn(move || {
            std::thread::sleep(CEILING);
            finish(&app);
        })
        .expect("failed to spawn splash ceiling thread");
}

/// O front terminou de carregar: troca o splash pela janela principal.
#[tauri::command]
pub fn splash_done(app: AppHandle) {
    finish(&app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn only_the_first_caller_takes_the_latch() {
        let latch = SplashLatch::default();
        assert!(latch.take());
        assert!(!latch.take());
        assert!(!latch.take());
    }

    /// Front e teto podem chegar juntos (boot que termina perto dos 4 s): só um
    /// dos dois pode fazer a troca.
    #[test]
    fn racing_callers_take_the_latch_exactly_once() {
        let latch = Arc::new(SplashLatch::default());
        let winners: usize = (0..8)
            .map(|_| {
                let latch = Arc::clone(&latch);
                std::thread::spawn(move || latch.take())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum();
        assert_eq!(winners, 1);
    }

    #[test]
    fn finish_without_managed_latch_is_a_noop() {
        let app = tauri::test::mock_app();
        finish(app.handle());
    }

    #[test]
    fn finish_is_idempotent() {
        let app = tauri::test::mock_app();
        app.manage(SplashLatch::default());
        finish(app.handle());
        assert!(!app.state::<SplashLatch>().take());
        finish(app.handle());
    }
}
