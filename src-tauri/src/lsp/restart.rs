//! Política de reinício do language server: quando ele cai, o core decide se
//! sobe de novo (e depois de quanto tempo) ou desiste com um motivo visível.
//! Pura — recebe o relógio de fora — para o teste cobrir a máquina de estados
//! sem processo nem `sleep`.

use std::time::{Duration, Instant};

/// Quedas seguidas (sem um período estável de pé no meio) antes de desistir.
pub const MAX_CONSECUTIVE: u32 = 5;
/// Quedas toleradas dentro de [`CRASH_WINDOW`], estável ou não. Segura o
/// server que sobe, fica de pé um minuto e cai — um ciclo que o contador de
/// seguidas zeraria para sempre.
pub const MAX_IN_WINDOW: usize = 12;
pub const CRASH_WINDOW: Duration = Duration::from_secs(3600);
/// De pé por este tempo, a próxima queda conta como a primeira de novo.
pub const STABLE_AFTER: Duration = Duration::from_secs(60);
const BACKOFF_BASE: Duration = Duration::from_millis(500);
const BACKOFF_CAP: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Retry { attempt: u32, after: Duration },
    GiveUp(String),
}

#[derive(Debug, Default)]
pub struct RestartPolicy {
    consecutive: u32,
    crashes: Vec<Instant>,
    ready_since: Option<Instant>,
}

impl RestartPolicy {
    pub fn on_ready(&mut self, now: Instant) {
        self.ready_since = Some(now);
    }

    pub fn on_crash(&mut self, now: Instant) -> Decision {
        if let Some(since) = self.ready_since.take() {
            if now.saturating_duration_since(since) >= STABLE_AFTER {
                self.consecutive = 0;
            }
        }
        self.crashes
            .retain(|t| now.saturating_duration_since(*t) < CRASH_WINDOW);
        self.crashes.push(now);
        self.consecutive += 1;
        if self.consecutive > MAX_CONSECUTIVE {
            return Decision::GiveUp(format!(
                "caiu {MAX_CONSECUTIVE} vezes seguidas logo depois de subir"
            ));
        }
        if self.crashes.len() > MAX_IN_WINDOW {
            return Decision::GiveUp("caiu vezes demais na última hora".into());
        }
        Decision::Retry {
            attempt: self.consecutive,
            after: backoff(self.consecutive),
        }
    }

    /// "Tentar de novo" manual: o dono pediu, o histórico não segura mais.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub fn backoff(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(16);
    BACKOFF_BASE.saturating_mul(1u32 << shift).min(BACKOFF_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retry_after(d: &Decision) -> Duration {
        match d {
            Decision::Retry { after, .. } => *after,
            Decision::GiveUp(r) => panic!("esperava reinício, veio desistência: {r}"),
        }
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(2));
        assert_eq!(backoff(5), Duration::from_secs(8));
        assert_eq!(backoff(40), BACKOFF_CAP, "nunca passa do teto");
    }

    #[test]
    fn crashes_right_after_boot_give_up_after_the_consecutive_limit() {
        let mut p = RestartPolicy::default();
        let mut now = Instant::now();
        for attempt in 1..=MAX_CONSECUTIVE {
            p.on_ready(now);
            now += Duration::from_secs(3);
            let d = p.on_crash(now);
            assert_eq!(
                d,
                Decision::Retry {
                    attempt,
                    after: backoff(attempt)
                }
            );
        }
        p.on_ready(now);
        now += Duration::from_secs(3);
        assert!(
            matches!(p.on_crash(now), Decision::GiveUp(_)),
            "subir e cair em 3s em loop não pode reiniciar para sempre"
        );
    }

    #[test]
    fn a_handshake_alone_does_not_reset_the_counter() {
        // O bug antigo: zerar as tentativas a cada initialize bem-sucedido fazia
        // o server que cai 3s depois de subir reiniciar indefinidamente.
        let mut p = RestartPolicy::default();
        let t0 = Instant::now();
        p.on_ready(t0);
        let first = p.on_crash(t0 + Duration::from_secs(3));
        p.on_ready(t0 + Duration::from_secs(4));
        let second = p.on_crash(t0 + Duration::from_secs(7));
        assert!(retry_after(&second) > retry_after(&first));
    }

    #[test]
    fn a_stable_period_makes_the_next_crash_count_as_the_first() {
        let mut p = RestartPolicy::default();
        let mut now = Instant::now();
        for _ in 0..3 {
            p.on_ready(now);
            now += Duration::from_secs(1);
            p.on_crash(now);
        }
        p.on_ready(now);
        now += STABLE_AFTER + Duration::from_secs(1);
        assert_eq!(
            p.on_crash(now),
            Decision::Retry {
                attempt: 1,
                after: backoff(1)
            }
        );
    }

    #[test]
    fn crash_before_ever_being_ready_counts_as_consecutive() {
        let mut p = RestartPolicy::default();
        let now = Instant::now();
        for _ in 0..MAX_CONSECUTIVE {
            assert!(matches!(p.on_crash(now), Decision::Retry { .. }));
        }
        assert!(matches!(p.on_crash(now), Decision::GiveUp(_)));
    }

    #[test]
    fn a_slow_crash_loop_is_bounded_by_the_window() {
        let mut p = RestartPolicy::default();
        let mut now = Instant::now();
        let mut gave_up = false;
        for _ in 0..=MAX_IN_WINDOW {
            p.on_ready(now);
            now += STABLE_AFTER + Duration::from_secs(1);
            if matches!(p.on_crash(now), Decision::GiveUp(_)) {
                gave_up = true;
                break;
            }
        }
        assert!(gave_up, "cair a cada ~1min a hora toda também desiste");
    }

    #[test]
    fn old_crashes_leave_the_window() {
        let mut p = RestartPolicy::default();
        let mut now = Instant::now();
        for _ in 0..MAX_IN_WINDOW {
            p.on_ready(now);
            now += STABLE_AFTER + Duration::from_secs(1);
            p.on_crash(now);
        }
        now += CRASH_WINDOW;
        p.on_ready(now);
        now += STABLE_AFTER + Duration::from_secs(1);
        assert!(matches!(
            p.on_crash(now),
            Decision::Retry { attempt: 1, .. }
        ));
    }

    #[test]
    fn manual_reset_clears_the_history() {
        let mut p = RestartPolicy::default();
        let now = Instant::now();
        for _ in 0..=MAX_CONSECUTIVE {
            p.on_crash(now);
        }
        assert!(matches!(p.on_crash(now), Decision::GiveUp(_)));
        p.reset();
        assert!(matches!(
            p.on_crash(now),
            Decision::Retry { attempt: 1, .. }
        ));
    }
}
