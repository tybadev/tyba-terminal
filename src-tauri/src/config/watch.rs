//! Edição na mão do `config.toml`, aplicada ao vivo.
//!
//! Editor salva de muitos jeitos — escrita direta, temp + rename, várias
//! escritas seguidas —, e cada um vira uma rajada de eventos. O [`Coalescer`]
//! junta a rajada e só dispara depois de um silêncio: um reload por salvamento,
//! nunca um por evento. É lógica pura, com o relógio injetado, para ser testada
//! sem disco.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

pub const QUIET: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub struct Coalescer {
    quiet: Duration,
    deadline: Option<Instant>,
}

impl Coalescer {
    pub fn new(quiet: Duration) -> Self {
        Self {
            quiet,
            deadline: None,
        }
    }

    /// Chegou evento: o disparo vai para `quiet` depois deste instante.
    pub fn touch(&mut self, now: Instant) {
        self.deadline = Some(now + self.quiet);
    }

    /// Quanto esperar até o próximo disparo; `None` quando não há nada pendente.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(now))
    }

    /// Dispara (uma vez) se o silêncio já durou o bastante.
    pub fn fire(&mut self, now: Instant) -> bool {
        match self.deadline {
            Some(d) if d <= now => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }
}

/// O evento mexeu no arquivo vigiado? A pasta tem outros arquivos (temas, o
/// temp da própria gravação), e eles não pedem reload.
pub fn touches(paths: &[PathBuf], file_name: &str) -> bool {
    paths
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some(file_name))
}

/// Vigia `root` e chama `on_change` uma vez por rajada que tocou `file_name`.
/// A thread é dona do watcher; ela e ele vivem enquanto o app vive.
pub fn spawn(root: &Path, file_name: &'static str, on_change: impl Fn() + Send + 'static) {
    let (tx, rx) = mpsc::channel::<()>();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if touches(&event.paths, file_name) {
                let _ = tx.send(());
            }
        }
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[tyba] config: watcher indisponível: {e}");
            return;
        }
    };
    // Vigia a pasta, não o arquivo: salvar por rename troca o inode, e um
    // watch no arquivo morreria no primeiro salvamento.
    if let Err(e) = watcher.watch(root, RecursiveMode::NonRecursive) {
        eprintln!("[tyba] config: não deu para vigiar {}: {e}", root.display());
        return;
    }
    let _ = std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            let _watcher = watcher;
            let mut c = Coalescer::new(QUIET);
            loop {
                let got = match c.wait(Instant::now()) {
                    None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                    Some(d) => rx.recv_timeout(d),
                };
                match got {
                    Ok(()) => c.touch(Instant::now()),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if c.fire(Instant::now()) {
                    on_change();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: Duration = Duration::from_millis(100);

    #[test]
    fn idle_coalescer_waits_forever_and_never_fires() {
        let mut c = Coalescer::new(Q);
        let t0 = Instant::now();
        assert_eq!(c.wait(t0), None);
        assert!(!c.fire(t0 + Duration::from_secs(10)));
    }

    #[test]
    fn a_burst_fires_once_after_the_last_event() {
        let mut c = Coalescer::new(Q);
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        c.touch(t0);
        c.touch(t0 + ms(30));
        c.touch(t0 + ms(60));
        assert!(
            !c.fire(t0 + ms(100)),
            "conta do último evento, não do primeiro"
        );
        assert_eq!(c.wait(t0 + ms(100)), Some(ms(60)));
        assert!(c.fire(t0 + ms(160)));
        assert!(!c.fire(t0 + ms(500)), "um disparo por rajada");
        assert_eq!(c.wait(t0 + ms(500)), None);
    }

    #[test]
    fn overdue_wait_is_zero() {
        let mut c = Coalescer::new(Q);
        let t0 = Instant::now();
        c.touch(t0);
        assert_eq!(c.wait(t0 + Duration::from_secs(1)), Some(Duration::ZERO));
    }

    #[test]
    fn two_separate_saves_fire_twice() {
        let mut c = Coalescer::new(Q);
        let t0 = Instant::now();
        c.touch(t0);
        assert!(c.fire(t0 + Q));
        c.touch(t0 + Duration::from_secs(1));
        assert!(c.fire(t0 + Duration::from_secs(1) + Q));
    }

    #[test]
    fn only_the_config_file_counts() {
        let root = PathBuf::from("/h/.tyba");
        assert!(touches(&[root.join("config.toml")], "config.toml"));
        assert!(touches(
            &[root.join(".config.toml.x.tmp"), root.join("config.toml")],
            "config.toml"
        ));
        assert!(!touches(&[root.join(".config.toml.x.tmp")], "config.toml"));
        assert!(!touches(&[root.join("themes/nord.json")], "config.toml"));
        assert!(!touches(&[], "config.toml"));
    }
}
