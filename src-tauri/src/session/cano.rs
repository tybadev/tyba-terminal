//! O ciclo de vida do Cano (o `ssh` local que alcança a SSH Session).

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::session::{ConnectionState, SessionId, SessionKind, StartupMode};
use crate::ssh::classify::{classify, CanoFailure};
use crate::ssh::tmux::{login_marker, Probe};

pub const PRELOGIN_CAP: usize = 16 * 1024;

pub struct CanoWatch {
    marker: Vec<u8>,
    tail: Vec<u8>,
    logged_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanoOutcome {
    LoggedIn,
    NotLoggedIn { tail: Vec<u8> },
}

impl CanoWatch {
    pub fn new(nonce: &str) -> Self {
        Self {
            marker: login_marker(nonce),
            tail: Vec::new(),
            logged_in: false,
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> bool {
        if self.logged_in {
            return false;
        }
        let from = self.tail.len().saturating_sub(self.marker.len() - 1);
        self.tail.extend_from_slice(data);
        if contains(&self.tail[from..], &self.marker) {
            self.logged_in = true;
            self.tail = Vec::new();
            return true;
        }
        if self.tail.len() > PRELOGIN_CAP {
            let excess = self.tail.len() - PRELOGIN_CAP;
            self.tail.drain(..excess);
        }
        false
    }

    pub fn finish(self) -> CanoOutcome {
        if self.logged_in {
            CanoOutcome::LoggedIn
        } else {
            CanoOutcome::NotLoggedIn { tail: self.tail }
        }
    }
}

pub const RECONNECT_DEADLINE: Duration = Duration::from_secs(300);

pub const STABLE_LIVE: Duration = Duration::from_secs(30);

fn to_chrono(d: Duration) -> chrono::Duration {
    chrono::Duration::from_std(d).unwrap_or(chrono::Duration::MAX)
}

const BACKOFF_SECS: [u64; 6] = [1, 2, 4, 8, 16, 30];

/// O que o driver (`lib.rs`) tem de executar depois de cada evento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanoDecision {
    Nothing,
    /// Falha de conexão: o motivo está em [`CanoLifecycle::failure`].
    Failed,
    /// Rodar o árbitro `has-session` e devolver o veredito com este token.
    Probe {
        token: u64,
    },
    /// A SSH Session acabou no Host: descartar a sessão local.
    Dispose,
    /// Esperar e chamar [`CanoLifecycle::timer_fired`] com este token.
    Wait {
        token: u64,
        delay: Duration,
    },
    Respawn,
    GaveUp,
}

/// Fase do Cano de uma sessão. Pura: o relógio entra por parâmetro, e quem
/// executa probe, espera e respawn é o driver.
#[derive(Debug, Clone)]
pub struct CanoLifecycle {
    state: ConnectionState,
    failure: Option<CanoFailure>,
    /// Muda a cada decisão que espera resposta: probe ou timer com token velho
    /// chegou atrasado e não vale mais.
    token: u64,
    /// Início da queda em curso; `None` fora de queda.
    drop_started: Option<DateTime<Utc>>,
    /// Esperas já agendadas nesta queda: índice do [`BACKOFF_SECS`].
    attempt: usize,
    live_since: Option<DateTime<Utc>>,
    /// A SSH Session acabou no Host e a sessão local foi descartada.
    disposed: bool,
}

/// Regra 13. `None` é a sessão descartada.
const TRANSITIONS: &[(ConnectionState, Option<ConnectionState>)] = {
    use ConnectionState::*;
    &[
        (Connecting, Some(Live)),
        (Connecting, Some(Failed)),
        (Connecting, Some(Reconnecting)),
        (Connecting, Some(Dropped)),
        (Live, Some(Reconnecting)),
        (Live, None),
        (Reconnecting, Some(Connecting)),
        (Reconnecting, Some(Dropped)),
        (Reconnecting, None),
        (Failed, Some(Connecting)),
        (Dropped, Some(Connecting)),
    ]
};

impl CanoLifecycle {
    /// Primeira conexão, fora de queda.
    pub fn connecting() -> Self {
        Self {
            state: ConnectionState::Connecting,
            failure: None,
            token: 0,
            drop_started: None,
            attempt: 0,
            live_since: None,
            disposed: false,
        }
    }

    /// Sessão que tinha login concluído e voltou no boot: é uma queda que
    /// começa agora, com o Cano já sendo religado.
    pub fn resumed_drop(now: DateTime<Utc>) -> Self {
        Self {
            drop_started: Some(now),
            ..Self::connecting()
        }
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn failure(&self) -> Option<&CanoFailure> {
        self.failure.as_ref()
    }

    pub fn can_transition(from: ConnectionState, to: Option<ConnectionState>) -> bool {
        TRANSITIONS.contains(&(from, to))
    }

    /// Única escrita de `state`: o que não está na tabela não acontece.
    fn go(&mut self, to: ConnectionState) -> bool {
        if self.disposed {
            return false;
        }
        if self.state == to {
            return true;
        }
        if !Self::can_transition(self.state, Some(to)) {
            return false;
        }
        self.state = to;
        true
    }

    fn next_token(&mut self) -> u64 {
        self.token += 1;
        self.token
    }

    pub fn logged_in(&mut self, now: DateTime<Utc>) -> CanoDecision {
        if self.go(ConnectionState::Live) {
            self.live_since = Some(now);
            self.failure = None;
        }
        CanoDecision::Nothing
    }

    pub fn exited(&mut self, outcome: &CanoOutcome, now: DateTime<Utc>) -> CanoDecision {
        let live_since = self.live_since.take();
        match (outcome, self.state) {
            (CanoOutcome::NotLoggedIn { tail }, ConnectionState::Connecting) => {
                let failure = classify(&String::from_utf8_lossy(tail));
                if self.drop_started.is_some() && failure.reason.retryable_in_drop() {
                    return self.schedule(now);
                }
                self.go(ConnectionState::Failed);
                self.failure = Some(failure);
                CanoDecision::Failed
            }
            (CanoOutcome::LoggedIn, ConnectionState::Live) => {
                // Um Cano que autentica e cai logo depois é a mesma queda: só
                // 30 s de pé provam que a conexão voltou de verdade.
                let stable = live_since.is_some_and(|since| now - since >= to_chrono(STABLE_LIVE));
                if self.drop_started.is_none() || stable {
                    self.drop_started = Some(now);
                    self.attempt = 0;
                }
                CanoDecision::Probe {
                    token: self.next_token(),
                }
            }
            _ => CanoDecision::Nothing,
        }
    }

    pub fn probed(&mut self, token: u64, verdict: Probe, now: DateTime<Utc>) -> CanoDecision {
        if token != self.token || self.state != ConnectionState::Live || self.disposed {
            return CanoDecision::Nothing;
        }
        if !verdict.should_reattach() {
            self.disposed = true;
            return CanoDecision::Dispose;
        }
        self.schedule(now)
    }

    pub fn timer_fired(&mut self, token: u64, now: DateTime<Utc>) -> CanoDecision {
        if token != self.token || self.state != ConnectionState::Reconnecting {
            return CanoDecision::Nothing;
        }
        if self.deadline_passed(now) {
            self.go(ConnectionState::Dropped);
            return CanoDecision::GaveUp;
        }
        if self.go(ConnectionState::Connecting) {
            CanoDecision::Respawn
        } else {
            CanoDecision::Nothing
        }
    }

    /// Tentativa pedida pelo dono. Vale como primeira conexão: fora de queda.
    pub fn retry(&mut self, now: DateTime<Utc>) -> CanoDecision {
        let _ = now;
        if !matches!(
            self.state,
            ConnectionState::Failed | ConnectionState::Dropped
        ) || !self.go(ConnectionState::Connecting)
        {
            return CanoDecision::Nothing;
        }
        *self = Self {
            token: self.token + 1,
            ..Self::connecting()
        };
        CanoDecision::Respawn
    }

    fn deadline_passed(&self, now: DateTime<Utc>) -> bool {
        self.drop_started
            .is_some_and(|started| now - started >= to_chrono(RECONNECT_DEADLINE))
    }

    /// Próxima espera da queda, ou a desistência. De `live` não há desistência
    /// direta (a tabela não tem `live → dropped`): a espera sai assim mesmo, e
    /// quem desiste é o fim dela.
    fn schedule(&mut self, now: DateTime<Utc>) -> CanoDecision {
        if self.state == ConnectionState::Connecting && self.deadline_passed(now) {
            self.go(ConnectionState::Dropped);
            return CanoDecision::GaveUp;
        }
        let secs = BACKOFF_SECS[self.attempt.min(BACKOFF_SECS.len() - 1)];
        self.attempt += 1;
        self.go(ConnectionState::Reconnecting);
        CanoDecision::Wait {
            token: self.next_token(),
            delay: Duration::from_secs(secs),
        }
    }
}

/// Regra 19: o que o boot faz com uma SSH Session que o app anterior deixou.
#[derive(Debug, Clone)]
pub enum SshBoot {
    /// Nunca autenticou: não há tmux no Host para reatar.
    Forget,
    /// Tinha login, mas a pref de startup não religa nada.
    Keep,
    /// Tinha login: volta como uma queda que começa no boot.
    Reattach(CanoLifecycle),
}

/// `None` para o que não é SSH: esta decisão não toca outras sessões.
pub fn ssh_boot(
    kind: &SessionKind,
    logged_in: bool,
    mode: StartupMode,
    now: DateTime<Utc>,
) -> Option<SshBoot> {
    match kind {
        SessionKind::Ssh { .. } if !logged_in => Some(SshBoot::Forget),
        SessionKind::Ssh { .. } if mode != StartupMode::Resume => Some(SshBoot::Keep),
        SessionKind::Ssh { .. } => Some(SshBoot::Reattach(CanoLifecycle::resumed_drop(now))),
        _ => None,
    }
}

pub type Job = Box<dyn FnOnce() + Send>;

/// Os efeitos que o condutor pede. Em produção é o `AppState`; nos testes, um
/// dublê com timer e sonda controlados à mão.
pub trait CanoPorts: Clone + Send + 'static {
    /// O que é preciso para sondar e religar. `None` é a sessão que não existe
    /// mais — e sessão que não existe não é sondada nem religada.
    type Target: Send;

    fn now(&self) -> DateTime<Utc>;
    fn target(&self, id: SessionId) -> Option<Self::Target>;
    /// Roda um evento na máquina da sessão; sessão sem ciclo decide `Nothing`.
    fn apply(
        &self,
        id: SessionId,
        event: impl FnOnce(&mut CanoLifecycle) -> CanoDecision,
    ) -> CanoDecision;
    /// O árbitro `has-session`. Bloqueia: só é chamado de dentro de `background`.
    fn probe(&self, target: &Self::Target) -> Probe;
    fn background(&self, job: Job);
    fn after(&self, delay: Duration, job: Job);
    /// `Err` leva o texto do erro de spawn.
    fn respawn(&self, id: SessionId, target: Self::Target) -> Result<(), String>;
    fn dispose(&self, id: SessionId);
}

/// Executa o que a máquina decidiu. Nada aqui decide fase: sonda e espera
/// devolvem o resultado à máquina com o token que ela deu.
///
/// A aba fechada durante uma espera ou sonda não vaza porque duas guardas se
/// somam: `SessionManager::dispose` tira a sessão **antes** do ciclo, e o
/// respawn só sai depois de `target` confirmar que a sessão existe. Inverter
/// aquela ordem abre uma janela em que o respawn recria o ciclo de uma sessão
/// já descartada (ver `dispose_esconde_a_sessao_antes_de_soltar_o_ciclo`).
pub fn conduct<P: CanoPorts>(ports: &P, id: SessionId, decision: CanoDecision) {
    match decision {
        CanoDecision::Nothing | CanoDecision::Failed | CanoDecision::GaveUp => {}
        CanoDecision::Probe { token } => {
            let p = ports.clone();
            ports.background(Box::new(move || {
                let Some(target) = p.target(id) else {
                    return;
                };
                let verdict = p.probe(&target);
                let next = p.apply(id, |c| c.probed(token, verdict, p.now()));
                conduct(&p, id, next);
            }));
        }
        CanoDecision::Wait { token, delay } => {
            let p = ports.clone();
            ports.after(
                delay,
                Box::new(move || {
                    let next = p.apply(id, |c| c.timer_fired(token, p.now()));
                    conduct(&p, id, next);
                }),
            );
        }
        CanoDecision::Dispose => ports.dispose(id),
        CanoDecision::Respawn => {
            let Some(target) = ports.target(id) else {
                return;
            };
            if let Err(detail) = ports.respawn(id, target) {
                // Sem Cano não há marco: para a máquina é uma saída antes do login.
                let outcome = CanoOutcome::NotLoggedIn {
                    tail: detail.into_bytes(),
                };
                let next = ports.apply(id, |c| c.exited(&outcome, ports.now()));
                conduct(ports, id, next);
            }
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn marco_inteiro_numa_leitura_conclui_o_login() {
        let mut watch = CanoWatch::new(NONCE);
        let mut chunk = b"Last login: never\r\n".to_vec();
        chunk.extend(login_marker(NONCE));
        assert!(watch.feed(&chunk));
        assert_eq!(watch.finish(), CanoOutcome::LoggedIn);
    }

    #[test]
    fn marco_partido_entre_leituras_ainda_conclui_o_login() {
        let marker = login_marker(NONCE);
        for cut in 1..marker.len() {
            let mut watch = CanoWatch::new(NONCE);
            let mut first = b"banner\r\n".to_vec();
            first.extend_from_slice(&marker[..cut]);
            assert!(!watch.feed(&first), "corte {cut}");
            assert!(watch.feed(&marker[cut..]), "corte {cut}");
        }
    }

    #[test]
    fn marco_de_outro_spawn_e_ignorado() {
        let mut watch = CanoWatch::new(NONCE);
        assert!(!watch.feed(&login_marker("ffffffffffffffffffffffffffffffff")));
        assert!(matches!(watch.finish(), CanoOutcome::NotLoggedIn { .. }));
    }

    #[test]
    fn saida_pre_login_guarda_so_os_ultimos_16_kib() {
        let mut watch = CanoWatch::new(NONCE);
        watch.feed(&vec![b'a'; PRELOGIN_CAP]);
        watch.feed(b"Permission denied (publickey).");
        let CanoOutcome::NotLoggedIn { tail } = watch.finish() else {
            panic!("sem marco não há login");
        };
        assert_eq!(tail.len(), PRELOGIN_CAP);
        assert!(tail.ends_with(b"Permission denied (publickey)."));
    }

    #[test]
    fn depois_do_marco_nada_mais_e_guardado_e_o_aviso_sai_uma_vez() {
        let mut watch = CanoWatch::new(NONCE);
        assert!(watch.feed(&login_marker(NONCE)));
        assert!(!watch.feed(&login_marker(NONCE)));
        watch.feed(&vec![b'x'; 4096]);
        assert_eq!(
            watch.tail.capacity(),
            0,
            "o shell remoto não pode encher memória aqui"
        );
        assert_eq!(watch.finish(), CanoOutcome::LoggedIn);
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap()
    }

    fn refused() -> CanoOutcome {
        CanoOutcome::NotLoggedIn {
            tail: b"Root@vps.example.test: Permission denied (publickey).\r\n".to_vec(),
        }
    }

    #[test]
    fn primeira_conexao_que_sai_antes_do_login_falha_sem_probe_nem_respawn() {
        let mut cano = CanoLifecycle::connecting();
        assert_eq!(cano.exited(&refused(), at(0)), CanoDecision::Failed);
        assert_eq!(cano.state(), ConnectionState::Failed);
        let failure = cano.failure().expect("falha carrega o motivo");
        assert_eq!(
            failure.reason,
            crate::ssh::classify::FailureReason::AuthRefused
        );
        assert_eq!(
            failure.detail,
            "Root@vps.example.test: Permission denied (publickey)."
        );
    }

    #[test]
    fn marco_de_login_poe_a_conexao_no_ar() {
        let mut cano = CanoLifecycle::connecting();
        assert_eq!(cano.logged_in(at(0)), CanoDecision::Nothing);
        assert_eq!(cano.state(), ConnectionState::Live);
    }

    #[test]
    fn queda_depois_do_login_pergunta_ao_arbitro_antes_de_qualquer_coisa() {
        let mut cano = CanoLifecycle::connecting();
        cano.logged_in(at(0));
        assert!(matches!(
            cano.exited(&CanoOutcome::LoggedIn, at(60)),
            CanoDecision::Probe { .. }
        ));
        assert_eq!(
            cano.state(),
            ConnectionState::Live,
            "enquanto o árbitro não responde, um `exit` do dono não pisca reconectando"
        );
    }

    fn dropped_at(secs: i64) -> (CanoLifecycle, u64) {
        let mut cano = CanoLifecycle::connecting();
        cano.logged_in(at(secs - 3600));
        let CanoDecision::Probe { token } = cano.exited(&CanoOutcome::LoggedIn, at(secs)) else {
            panic!("queda sem probe");
        };
        (cano, token)
    }

    #[test]
    fn sessao_que_acabou_no_host_e_descartada_como_hoje() {
        for verdict in [Probe::Gone, Probe::NoTmux] {
            let (mut cano, token) = dropped_at(0);
            assert_eq!(cano.probed(token, verdict, at(1)), CanoDecision::Dispose);
        }
    }

    #[test]
    fn sessao_viva_ou_incerta_no_host_espera_um_segundo_reconectando() {
        for verdict in [Probe::Alive, Probe::Unknown] {
            let (mut cano, token) = dropped_at(0);
            let CanoDecision::Wait { delay, .. } = cano.probed(token, verdict, at(1)) else {
                panic!("{verdict:?} tem de reatar");
            };
            assert_eq!(delay, Duration::from_secs(1));
            assert_eq!(cano.state(), ConnectionState::Reconnecting);
        }
    }

    fn no_route() -> CanoOutcome {
        CanoOutcome::NotLoggedIn {
            tail: b"ssh: connect to host vps.example.test port 22: Operation timed out\r\n"
                .to_vec(),
        }
    }

    #[test]
    fn fim_da_espera_religa_o_cano() {
        let (mut cano, token) = dropped_at(0);
        let CanoDecision::Wait { token, .. } = cano.probed(token, Probe::Alive, at(0)) else {
            panic!("tinha de esperar");
        };
        assert_eq!(cano.timer_fired(token, at(1)), CanoDecision::Respawn);
        assert_eq!(cano.state(), ConnectionState::Connecting);
    }

    #[test]
    fn backoff_segue_1_2_4_8_16_30_30_enquanto_a_rede_nao_volta() {
        let (mut cano, token) = dropped_at(0);
        let mut now = 0;
        let mut decision = cano.probed(token, Probe::Unknown, at(now));
        let mut seen = Vec::new();
        while let CanoDecision::Wait { token, delay } = decision {
            seen.push(delay.as_secs());
            now += delay.as_secs() as i64;
            decision = match cano.timer_fired(token, at(now)) {
                CanoDecision::Respawn => cano.exited(&no_route(), at(now)),
                other => other,
            };
            assert!(seen.len() < 100, "a queda tem prazo");
        }
        assert_eq!(&seen[..8], &[1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(decision, CanoDecision::GaveUp);
        assert_eq!(cano.state(), ConnectionState::Dropped);
        assert!(
            (300..330).contains(&now),
            "desiste quando passam 300 s da queda; got {now}"
        );
    }

    /// Reata, autentica, fica `live` por `live_for` segundos e cai de novo.
    /// Devolve o delay da espera que a nova queda agendou.
    fn relogin_and_drop(cano: &mut CanoLifecycle, now: &mut i64, live_for: i64) -> CanoDecision {
        cano.logged_in(at(*now));
        *now += live_for;
        let CanoDecision::Probe { token } = cano.exited(&CanoOutcome::LoggedIn, at(*now)) else {
            panic!("queda sem probe");
        };
        cano.probed(token, Probe::Alive, at(*now))
    }

    fn wait_then_respawn(cano: &mut CanoLifecycle, now: &mut i64, decision: CanoDecision) {
        let CanoDecision::Wait { token, delay } = decision else {
            panic!("esperava espera, veio {decision:?}");
        };
        *now += delay.as_secs() as i64;
        assert_eq!(cano.timer_fired(token, at(*now)), CanoDecision::Respawn);
    }

    #[test]
    fn login_que_dura_menos_de_30s_nao_zera_a_queda() {
        let (mut cano, token) = dropped_at(0);
        let mut now = 0;
        let mut decision = cano.probed(token, Probe::Alive, at(now));
        let mut delays = Vec::new();
        while let CanoDecision::Wait { token, delay } = decision {
            delays.push(delay.as_secs());
            now += delay.as_secs() as i64;
            decision = match cano.timer_fired(token, at(now)) {
                CanoDecision::Respawn => relogin_and_drop(&mut cano, &mut now, 29),
                other => other,
            };
            assert!(delays.len() < 100);
        }
        assert_eq!(
            &delays[..4],
            &[1, 2, 4, 8],
            "o backoff continua de onde estava"
        );
        assert_eq!(decision, CanoDecision::GaveUp);
        assert!(
            now < 400,
            "a queda que só pisca ainda desiste perto de 5 min; got {now}"
        );
    }

    #[test]
    fn login_que_dura_30s_zera_a_queda() {
        let (mut cano, token) = dropped_at(0);
        let mut now = 0;
        let decision = cano.probed(token, Probe::Alive, at(now));
        wait_then_respawn(&mut cano, &mut now, decision);
        let decision = relogin_and_drop(&mut cano, &mut now, 29);
        wait_then_respawn(&mut cano, &mut now, decision);
        let decision = relogin_and_drop(&mut cano, &mut now, 30);
        let CanoDecision::Wait { token, delay } = decision else {
            panic!("esperava espera, veio {decision:?}");
        };
        assert_eq!(
            delay,
            Duration::from_secs(1),
            "queda nova depois de 30 s de pé recomeça do primeiro degrau"
        );
        now += 290;
        assert_eq!(cano.timer_fired(token, at(now)), CanoDecision::Respawn);
        assert!(
            matches!(cano.exited(&no_route(), at(now)), CanoDecision::Wait { .. }),
            "o relógio conta da queda nova, não da antiga"
        );
    }

    #[test]
    fn dentro_da_queda_so_rede_ausente_continua_tentando() {
        let unresolved = CanoOutcome::NotLoggedIn {
            tail: b"ssh: Could not resolve hostname vps.example.test: nodename nor servname provided\r\n"
                .to_vec(),
        };
        for (outcome, retries) in [(no_route(), true), (unresolved, true), (refused(), false)] {
            let (mut cano, token) = dropped_at(0);
            let mut now = 0;
            let decision = cano.probed(token, Probe::Alive, at(now));
            wait_then_respawn(&mut cano, &mut now, decision);
            let decision = cano.exited(&outcome, at(now));
            if retries {
                assert!(matches!(decision, CanoDecision::Wait { .. }), "{outcome:?}");
                assert_eq!(cano.state(), ConnectionState::Reconnecting);
            } else {
                assert_eq!(decision, CanoDecision::Failed);
                assert_eq!(cano.state(), ConnectionState::Failed);
                assert_eq!(
                    cano.failure().map(|f| f.reason),
                    Some(crate::ssh::classify::FailureReason::AuthRefused)
                );
            }
        }
    }

    #[test]
    fn resposta_atrasada_de_probe_ou_timer_nao_vale_mais() {
        let (mut cano, probe_token) = dropped_at(0);
        let CanoDecision::Wait { token, .. } = cano.probed(probe_token, Probe::Alive, at(0)) else {
            panic!("tinha de esperar");
        };
        assert_eq!(
            cano.probed(probe_token, Probe::Gone, at(0)),
            CanoDecision::Nothing
        );
        assert_eq!(cano.timer_fired(token + 1, at(1)), CanoDecision::Nothing);
        assert_eq!(cano.state(), ConnectionState::Reconnecting);
    }

    #[test]
    fn tentar_de_novo_so_vale_a_partir_de_falha_ou_desistencia() {
        let mut failed = CanoLifecycle::connecting();
        failed.exited(&refused(), at(0));
        assert_eq!(failed.retry(at(10)), CanoDecision::Respawn);
        assert_eq!(failed.state(), ConnectionState::Connecting);
        assert!(
            failed.failure().is_none(),
            "a faixa de falha some ao tentar de novo"
        );
        assert_eq!(
            failed.exited(&no_route(), at(11)),
            CanoDecision::Failed,
            "a tentativa do dono é conexão nova: falhou, para"
        );

        let mut live = CanoLifecycle::connecting();
        live.logged_in(at(0));
        assert_eq!(live.retry(at(1)), CanoDecision::Nothing);
        assert_eq!(
            CanoLifecycle::connecting().retry(at(1)),
            CanoDecision::Nothing
        );
    }

    #[test]
    fn desistencia_aceita_nova_tentativa_do_dono() {
        let (mut cano, token) = dropped_at(0);
        let decision = cano.probed(token, Probe::Unknown, at(0));
        let CanoDecision::Wait { token, .. } = decision else {
            panic!()
        };
        cano.timer_fired(token, at(1));
        assert_eq!(cano.exited(&no_route(), at(400)), CanoDecision::GaveUp);
        assert_eq!(cano.retry(at(500)), CanoDecision::Respawn);
        assert_eq!(cano.state(), ConnectionState::Connecting);
    }

    #[test]
    fn sessao_retomada_no_boot_e_uma_queda_que_comecou_agora() {
        let mut cano = CanoLifecycle::resumed_drop(at(0));
        assert_eq!(cano.state(), ConnectionState::Connecting);
        assert!(
            matches!(cano.exited(&no_route(), at(5)), CanoDecision::Wait { .. }),
            "boot sem rede ainda tenta por 5 min"
        );
        let mut cano = CanoLifecycle::resumed_drop(at(0));
        assert_eq!(cano.exited(&refused(), at(5)), CanoDecision::Failed);
    }

    fn ssh() -> SessionKind {
        SessionKind::Ssh {
            host_id: "h1".into(),
        }
    }

    #[test]
    fn boot_religa_sessao_com_login_como_queda_que_comecou_no_boot() {
        let boot = at(0);
        let Some(SshBoot::Reattach(cano)) = ssh_boot(&ssh(), true, StartupMode::Resume, boot)
        else {
            panic!("sessão com login tem de religar");
        };
        assert_eq!(cano.state(), ConnectionState::Connecting);

        let mut sem_rede = cano.clone();
        assert!(
            matches!(
                sem_rede.exited(&no_route(), at(5)),
                CanoDecision::Wait { .. }
            ),
            "dentro da queda, rede ausente continua tentando"
        );
        assert_eq!(sem_rede.state(), ConnectionState::Reconnecting);

        let mut antes = cano.clone();
        assert!(matches!(
            antes.exited(&no_route(), at(299)),
            CanoDecision::Wait { .. }
        ));
        let mut no_prazo = cano;
        assert_eq!(
            no_prazo.exited(&no_route(), at(300)),
            CanoDecision::GaveUp,
            "os 300 s contam do boot"
        );
        assert_eq!(no_prazo.state(), ConnectionState::Dropped);
    }

    #[test]
    fn boot_esquece_sessao_sem_login_em_qualquer_modo() {
        for mode in [
            StartupMode::Resume,
            StartupMode::KeepLayout,
            StartupMode::Fresh,
        ] {
            assert!(
                matches!(ssh_boot(&ssh(), false, mode, at(0)), Some(SshBoot::Forget)),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn boot_so_religa_ssh_quando_a_pref_religa() {
        for mode in [StartupMode::KeepLayout, StartupMode::Fresh] {
            assert!(
                matches!(ssh_boot(&ssh(), true, mode, at(0)), Some(SshBoot::Keep)),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn decisao_ssh_do_boot_nao_toca_sessao_de_shell() {
        for logged_in in [true, false] {
            assert!(ssh_boot(&SessionKind::Shell, logged_in, StartupMode::Resume, at(0)).is_none());
        }
    }

    const ALL: [ConnectionState; 5] = [
        ConnectionState::Live,
        ConnectionState::Connecting,
        ConnectionState::Reconnecting,
        ConnectionState::Dropped,
        ConnectionState::Failed,
    ];

    /// A regra 13, escrita de novo à mão: se a tabela do código mudar, este
    /// teste tem de mudar junto, de propósito.
    #[test]
    fn so_as_transicoes_da_regra_13_existem() {
        use ConnectionState::*;
        let expected: &[(ConnectionState, Option<ConnectionState>)] = &[
            (Connecting, Some(Live)),
            (Connecting, Some(Failed)),
            (Connecting, Some(Reconnecting)),
            (Connecting, Some(Dropped)),
            (Live, Some(Reconnecting)),
            (Live, None),
            (Reconnecting, Some(Connecting)),
            (Reconnecting, Some(Dropped)),
            (Reconnecting, None),
            (Failed, Some(Connecting)),
            (Dropped, Some(Connecting)),
        ];
        for from in ALL {
            for to in ALL.iter().copied().map(Some).chain([None]) {
                if Some(from) == to {
                    continue;
                }
                assert_eq!(
                    CanoLifecycle::can_transition(from, to),
                    expected.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Event {
        Login,
        ExitLoggedIn,
        ExitNoRoute,
        ExitRefused,
        Probe(Probe),
        Timer,
        Retry,
    }

    fn apply(cano: &mut CanoLifecycle, event: Event, now: DateTime<Utc>) -> CanoDecision {
        let token = cano.token;
        match event {
            Event::Login => cano.logged_in(now),
            Event::ExitLoggedIn => cano.exited(&CanoOutcome::LoggedIn, now),
            Event::ExitNoRoute => cano.exited(&no_route(), now),
            Event::ExitRefused => cano.exited(&refused(), now),
            Event::Probe(verdict) => cano.probed(token, verdict, now),
            Event::Timer => cano.timer_fired(token, now),
            Event::Retry => cano.retry(now),
        }
    }

    fn explore(
        cano: &CanoLifecycle,
        now: i64,
        depth: usize,
        seen: &mut std::collections::HashSet<(ConnectionState, Option<ConnectionState>)>,
    ) {
        if depth == 0 || cano.disposed {
            return;
        }
        let events = [
            Event::Login,
            Event::ExitLoggedIn,
            Event::ExitNoRoute,
            Event::ExitRefused,
            Event::Probe(Probe::Alive),
            Event::Probe(Probe::Gone),
            Event::Timer,
            Event::Retry,
        ];
        for event in events {
            for step in [1, 31, 301] {
                let mut next = cano.clone();
                let from = next.state();
                apply(&mut next, event, at(now + step));
                let to = (!next.disposed).then(|| next.state());
                if Some(from) != to {
                    assert!(
                        CanoLifecycle::can_transition(from, to),
                        "{event:?} levou {from:?} -> {to:?}, fora da regra 13"
                    );
                    seen.insert((from, to));
                }
                explore(&next, now + step, depth - 1, seen);
            }
        }
    }

    #[test]
    fn nenhuma_sequencia_de_eventos_sai_da_tabela() {
        let mut seen = std::collections::HashSet::new();
        explore(&CanoLifecycle::connecting(), 0, 4, &mut seen);
        explore(&CanoLifecycle::resumed_drop(at(0)), 0, 4, &mut seen);
        use ConnectionState::*;
        for required in [
            (Connecting, Some(Live)),
            (Connecting, Some(Failed)),
            (Connecting, Some(Reconnecting)),
            (Live, Some(Reconnecting)),
            (Live, None),
            (Reconnecting, Some(Connecting)),
            (Failed, Some(Connecting)),
        ] {
            assert!(seen.contains(&required), "a máquina nunca fez {required:?}");
        }
    }

    /// O condutor pela fiação real do `SessionManager`: só sonda, timer e
    /// respawn são dublês.
    mod conductor {
        use super::*;
        use crate::pty::{PtyPool, SharedPtyPool};
        use crate::session::store::Store;
        use crate::session::{Session, SessionKind, SessionManager, SessionStatus};
        use parking_lot::Mutex;
        use std::sync::Arc;

        #[derive(Clone)]
        struct Harness {
            manager: Arc<SessionManager>,
            pool: SharedPtyPool,
            clock: Arc<Mutex<i64>>,
            verdict: Probe,
            respawn_fails: bool,
            in_flight: Arc<Mutex<Vec<Job>>>,
            timers: Arc<Mutex<Vec<(Duration, Job)>>>,
            probes: Arc<Mutex<Vec<SessionId>>>,
            respawns: Arc<Mutex<Vec<SessionId>>>,
        }

        impl CanoPorts for Harness {
            type Target = SessionId;

            fn now(&self) -> DateTime<Utc> {
                at(*self.clock.lock())
            }

            fn target(&self, id: SessionId) -> Option<SessionId> {
                match self.manager.get(id)?.kind {
                    SessionKind::Ssh { .. } => Some(id),
                    _ => None,
                }
            }

            fn apply(
                &self,
                id: SessionId,
                event: impl FnOnce(&mut CanoLifecycle) -> CanoDecision,
            ) -> CanoDecision {
                self.manager.apply_cano(id, event).0
            }

            fn probe(&self, target: &SessionId) -> Probe {
                self.probes.lock().push(*target);
                self.verdict
            }

            fn background(&self, job: Job) {
                self.in_flight.lock().push(job);
            }

            fn after(&self, delay: Duration, job: Job) {
                self.timers.lock().push((delay, job));
            }

            fn respawn(&self, id: SessionId, _target: SessionId) -> Result<(), String> {
                self.respawns.lock().push(id);
                if self.respawn_fails {
                    Err("ssh: No such file or directory".into())
                } else {
                    Ok(())
                }
            }

            fn dispose(&self, id: SessionId) {
                self.manager.dispose(&self.pool, id);
            }
        }

        impl Harness {
            fn new(verdict: Probe) -> (Self, SessionId) {
                let store = Arc::new(Store::open_in_memory().unwrap());
                let manager = Arc::new(SessionManager::new(store));
                let id = SessionId::new_v4();
                manager.sessions.write().insert(
                    id,
                    Session {
                        id,
                        kind: SessionKind::Ssh {
                            host_id: "h1".into(),
                        },
                        title: "ssh vps".into(),
                        repo_root: None,
                        worktree: None,
                        status: SessionStatus::Running,
                        attention: false,
                        created_at: Utc::now(),
                        cwd: None,
                        connection: ConnectionState::Connecting,
                        connection_failure: None,
                        agent_conversation_id: None,
                        observed: None,
                        opened_by_gate: false,
                        did_work: false,
                    },
                );
                manager.track_cano(id, CanoLifecycle::connecting());
                let harness = Self {
                    manager,
                    pool: Arc::new(PtyPool::new()),
                    clock: Arc::new(Mutex::new(0)),
                    verdict,
                    respawn_fails: false,
                    in_flight: Arc::default(),
                    timers: Arc::default(),
                    probes: Arc::default(),
                    respawns: Arc::default(),
                };
                (harness, id)
            }

            /// Login às 0 s e queda às 60 s: a sonda fica em voo.
            fn drop_with_probe_in_flight(&self, id: SessionId) {
                self.manager.apply_cano_login(id, at(0));
                *self.clock.lock() = 60;
                let decision = self
                    .manager
                    .apply_cano(id, |c| c.exited(&CanoOutcome::LoggedIn, at(60)))
                    .0;
                conduct(self, id, decision);
                assert_eq!(self.in_flight.lock().len(), 1, "a queda tem de sondar");
            }

            fn answer_probes(&self) {
                let jobs = std::mem::take(&mut *self.in_flight.lock());
                for job in jobs {
                    job();
                }
            }

            /// Sessão em `reconnecting` com a espera agendada.
            fn reconnecting(&self, id: SessionId) {
                self.drop_with_probe_in_flight(id);
                self.answer_probes();
                assert_eq!(self.connection(id), Some(ConnectionState::Reconnecting));
                assert_eq!(self.timers.lock().len(), 1, "a queda tem de esperar");
            }

            fn fire_timers(&self) {
                let timers = std::mem::take(&mut *self.timers.lock());
                for (delay, job) in timers {
                    *self.clock.lock() += delay.as_secs() as i64;
                    job();
                }
            }

            fn connection(&self, id: SessionId) -> Option<ConnectionState> {
                self.manager.get(id).map(|s| s.connection)
            }

            fn respawn_count(&self) -> usize {
                self.respawns.lock().len()
            }
        }

        #[test]
        fn fim_da_espera_religa_o_cano_pela_fiacao_real() {
            let (h, id) = Harness::new(Probe::Alive);
            h.reconnecting(id);
            h.fire_timers();
            assert_eq!(h.respawn_count(), 1);
            assert_eq!(h.connection(id), Some(ConnectionState::Connecting));
        }

        #[test]
        fn respawn_que_nem_sobe_conta_como_saida_antes_do_login() {
            let (mut h, id) = Harness::new(Probe::Alive);
            h.respawn_fails = true;
            h.reconnecting(id);
            h.fire_timers();
            assert_eq!(h.respawn_count(), 1);
            assert_eq!(h.connection(id), Some(ConnectionState::Failed));
            let failure = h.manager.get(id).unwrap().connection_failure.unwrap();
            assert_eq!(failure.detail, "ssh: No such file or directory");
            assert!(h.timers.lock().is_empty(), "falha fora da rede não insiste");
        }

        fn assert_gone(h: &Harness, id: SessionId) {
            assert!(h.manager.get(id).is_none(), "a sessão voltou");
            assert!(
                !h.manager.canos.lock().contains_key(&id),
                "o ciclo da sessão descartada vazou"
            );
        }

        #[test]
        fn aba_fechada_durante_a_espera_nao_religa() {
            let (h, id) = Harness::new(Probe::Alive);
            h.reconnecting(id);
            h.manager.dispose(&h.pool, id);
            h.fire_timers();
            assert_eq!(h.respawn_count(), 0);
            assert_gone(&h, id);
        }

        #[test]
        fn aba_fechada_com_a_sonda_em_voo_nao_sonda_nem_religa() {
            let (h, id) = Harness::new(Probe::Alive);
            h.drop_with_probe_in_flight(id);
            h.manager.dispose(&h.pool, id);
            h.answer_probes();
            h.fire_timers();
            assert!(h.probes.lock().is_empty(), "sondou sessão descartada");
            assert!(h.timers.lock().is_empty());
            assert_eq!(h.respawn_count(), 0);
            assert_gone(&h, id);
        }

        /// A espera venceu (a máquina já disse `Respawn`) e o dono fecha a aba
        /// ao mesmo tempo. O lock do ciclo fica preso para parar o `dispose` no
        /// meio: se a sessão ainda estiver visível nesse ponto, o condutor religa
        /// uma aba fechada.
        #[test]
        fn dispose_esconde_a_sessao_antes_de_soltar_o_ciclo() {
            let (h, id) = Harness::new(Probe::Alive);
            h.reconnecting(id);
            let token = h.manager.canos.lock()[&id].token;
            let respawn = h.manager.apply_cano(id, |c| c.timer_fired(token, at(61))).0;
            assert_eq!(respawn, CanoDecision::Respawn);

            let held = h.manager.canos.lock();
            let closing = {
                let h = h.clone();
                std::thread::spawn(move || h.manager.dispose(&h.pool, id))
            };
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while h.manager.get(id).is_some() && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            conduct(&h, id, respawn);
            drop(held);
            closing.join().unwrap();

            assert_eq!(h.respawn_count(), 0, "religou uma aba fechada");
            assert_gone(&h, id);
        }
    }
}
