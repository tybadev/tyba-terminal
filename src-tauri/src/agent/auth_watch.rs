//! Entrega C, mecanismo 2 — o lado stateful do scanner de runtime: liga o
//! [`crate::status::auth_scan::AuthScanner`] (puro) à sessão, confirma
//! estagnação antes de acusar (evita faixa falsa por conteúdo incidental do
//! agente ou retry transitório) e deduplica por `(session, kind)` até haver
//! progresso.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Runtime};

use crate::session::{AgentRunnerKind, SessionId, SessionStatus, SharedSessionManager};
use crate::status::auth_scan::{patterns_for, AuthScanner};

use super::auth_alert::{AuthAlertKind, AuthAlertPayload, AuthPhase, EVENT_AGENT_AUTH_ALERT};

/// Mesmo valor de `TURN_END_SETTLE_MS` (`agent/session.rs`) e de
/// `OBSERVED_SETTLE_MS` (`status/observed_notify.rs`) — não um número
/// escolhido à parte: é o terceiro lugar do repo que espera a poeira baixar
/// antes de agir, com o mesmo teto.
const AUTH_SETTLE_MS: u64 = 2500;

/// Onde o alerta confirmado vai. Fechado sobre o `AppHandle` concreto e não
/// guardado por valor: mesmo raciocínio do `ObservedSink`
/// (`status/observer.rs`) — `AuthWatch` vive dentro de `PtyPool`, que não é
/// genérica sobre `Runtime`, então o tipo concreto do Tauri não pode
/// atravessar essa fronteira. `Send + Sync` porque a thread de settle
/// chama por fora da thread leitora que criou o `AuthWatch`.
type AuthSink = Arc<dyn Fn(AuthAlertPayload) + Send + Sync>;

/// Liga o scanner puro à sessão. Vive na thread LEITORA do PTY (molde do
/// `ScreenObserver`, que vive na thread emissora) — `feed` é chamado depois
/// de cada `read()` bem-sucedido.
pub struct AuthWatch {
    emit: AuthSink,
    sessions: SharedSessionManager,
    session_id: SessionId,
    scanner: AuthScanner,
    /// Kind "em voo" (settle pendente) OU já confirmado — as duas travam
    /// re-arme (R7, dedup até progresso). `Arc<Mutex<..>>`, não um
    /// `HashSet` liso: a thread de settle roda separada da que chama
    /// `feed`, e precisa mexer neste mesmo conjunto pra decidir "descarta"
    /// (R5) vs "confirma e marca" (R6) sem inventar um canal só pra isso.
    emitted: Arc<Mutex<HashSet<AuthAlertKind>>>,
    settle: Duration,
}

impl AuthWatch {
    /// `None` quando este runner não tem tabela (R10) — Codex e Custom
    /// hoje. `pty::mod` só instala um pipe quando isto devolve `Some`.
    ///
    /// Genérica sobre `R: Runtime` só AQUI, na borda: o `AppHandle` real
    /// (produção) ou o `MockRuntime` (teste) vira uma `AuthSink` concreta
    /// antes de entrar na struct — a mesma borda que `status::observer`
    /// atravessa pra montar um `ObservedSink`.
    pub fn new<R: Runtime>(
        app: AppHandle<R>,
        sessions: SharedSessionManager,
        session_id: SessionId,
        runner: &AgentRunnerKind,
    ) -> Option<Self> {
        let patterns = patterns_for(runner);
        if patterns.is_empty() {
            return None;
        }
        let emit: AuthSink = Arc::new(move |payload| {
            let _ = app.emit(EVENT_AGENT_AUTH_ALERT, payload);
        });
        Some(Self::with_settle(
            emit,
            sessions,
            session_id,
            patterns,
            Duration::from_millis(AUTH_SETTLE_MS),
        ))
    }

    /// Seam de teste (`pub(crate)`): sink e teto de settle injetados
    /// direto, sem depender de `tauri::test::mock_app` nem de decodificar
    /// evento por IPC — os testes de R5/R6/R7 também não podem pagar
    /// 2500ms reais por caso. `AUTH_SETTLE_MS` continua sendo o valor real
    /// de produção; aqui só a fiação de saída e o teto mudam, nunca a
    /// lógica de `feed`/`arm_settle`.
    pub(crate) fn with_settle(
        emit: AuthSink,
        sessions: SharedSessionManager,
        session_id: SessionId,
        patterns: &'static [(&'static str, AuthAlertKind)],
        settle: Duration,
    ) -> Self {
        Self {
            emit,
            sessions,
            session_id,
            scanner: AuthScanner::new(patterns),
            emitted: Arc::new(Mutex::new(HashSet::new())),
            settle,
        }
    }

    fn is_running(&self) -> bool {
        self.sessions
            .get(self.session_id)
            .map(|s| matches!(s.status, SessionStatus::Running))
            .unwrap_or(false)
    }

    /// Alimenta um chunk cru do PTY.
    pub fn feed(&mut self, data: &[u8]) {
        // Progresso: a sessão voltou a rodar — alertas já confirmados
        // deixam de bloquear um alerta NOVO da mesma espécie ("Progresso
        // posterior limpa o emitted daquele kind", § do design). Só paga o
        // `sessions.get` (clona a `Session` inteira) quando há algo pra de
        // fato limpar: a maioria dos reads de uma sessão saudável nunca tem
        // `emitted` não-vazio.
        if !self.emitted.lock().is_empty() && self.is_running() {
            self.emitted.lock().clear();
        }
        let Some(kind) = self.scanner.feed(data) else {
            return;
        };
        {
            let mut emitted = self.emitted.lock();
            if emitted.contains(&kind) {
                return; // já em voo ou já confirmado — dedupe (R7)
            }
            emitted.insert(kind);
        }
        self.arm_settle(kind);
    }

    /// Confirmação de estagnação: espera `settle`, depois olha o status REAL
    /// da sessão — não o que estava na tela no instante do match, que pode
    /// ter sido conteúdo do próprio agente ou um retry que já se recuperou.
    fn arm_settle(&self, kind: AuthAlertKind) {
        let emit = Arc::clone(&self.emit);
        let sessions = self.sessions.clone();
        let session_id = self.session_id;
        let emitted = Arc::clone(&self.emitted);
        let settle = self.settle;
        std::thread::spawn(move || {
            std::thread::sleep(settle);
            let running = sessions
                .get(session_id)
                .map(|s| matches!(s.status, SessionStatus::Running))
                .unwrap_or(false);
            if running {
                // R5: o agente seguiu — string incidental, ou um retry que já
                // se recuperou sozinho. Solta o "em voo" pra permitir
                // reavaliar se a mesma string aparecer de novo depois.
                emitted.lock().remove(&kind);
                return;
            }
            // R6: `Idle`, `AwaitingInput`, `Exited`/`Failed`, ou a sessão nem
            // existe mais (`sessions.get` devolveu `None`, "ausente") — todos
            // contam como "parou", nenhum é "seguiu trabalhando".
            emit(AuthAlertPayload {
                session_id,
                phase: AuthPhase::Runtime,
                kind,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc as StdArc;

    use super::*;
    use crate::session::{Session, SessionKind, SessionManager};
    use crate::status::auth_scan::patterns_for as scan_patterns_for;

    /// Bem mais curto que `AUTH_SETTLE_MS` de produção — os testes daqui
    /// esperam o settle de verdade (`std::thread::sleep`), então o valor
    /// precisa ser pequeno pra suíte não ficar penosa; a LÓGICA é a mesma,
    /// só o teto muda (ver `with_settle`). 150ms (não 60ms): a suíte inteira
    /// roda em paralelo sob `cargo test`, e sob contenção pesada de CPU um
    /// teto curto demais falseia como "não emitiu" por atraso de
    /// agendamento, não por bug de lógica.
    const TEST_SETTLE: Duration = Duration::from_millis(150);

    /// Espelha o `make()` privado de `session::mod::tests` -- não dá pra
    /// reusar dali (módulo diferente), então uma cópia mínima local.
    fn make_agent_session(status: SessionStatus) -> Session {
        Session {
            id: SessionId::new_v4(),
            kind: SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode,
            },
            title: "s".into(),
            repo_root: None,
            worktree: None,
            status,
            attention: false,
            created_at: chrono::Utc::now(),
            cwd: Some(std::path::PathBuf::from("/tmp")),
            connection: crate::session::ConnectionState::default(),
            agent_conversation_id: None,
            observed: None,
            opened_by_gate: false,
            did_work: false,
        }
    }

    /// `restore()` (`session/mod.rs`) SEMPRE reconcilia qualquer status que
    /// não seja `Exited`/`Failed` para `Exited{-1}` -- é assim que ele
    /// funciona de verdade (nenhum PTY sobrevive ao processo do TYBA), então
    /// não dá pra construir uma sessão `Running`/`Idle`/`AwaitingInput`
    /// direto via `store + restore`. O status pedido é aplicado DEPOIS,
    /// com `apply_status` (mesma mutação sem IPC que `set_status` usa por
    /// baixo).
    fn manager_with(status: SessionStatus) -> (SharedSessionManager, SessionId) {
        let store = StdArc::new(crate::session::store::Store::open_in_memory().unwrap());
        let session = make_agent_session(SessionStatus::Exited { code: 0 });
        let id = session.id;
        store.upsert_session(&session).unwrap();
        let manager: SharedSessionManager = StdArc::new(SessionManager::new(StdArc::clone(&store)));
        manager.restore().unwrap();
        manager.apply_status(id, status);
        (manager, id)
    }

    /// Sink de teste: acumula os payloads emitidos, sem IPC nenhum -- nem
    /// `mock_app`, nem `listen`, nem round-trip por JSON.
    fn spy_sink() -> (AuthSink, StdArc<Mutex<Vec<AuthAlertPayload>>>) {
        let seen = StdArc::new(Mutex::new(Vec::new()));
        let sink = StdArc::clone(&seen);
        let emit: AuthSink = StdArc::new(move |payload| sink.lock().push(payload));
        (emit, seen)
    }

    fn watch(emit: AuthSink, sessions: SharedSessionManager, session_id: SessionId) -> AuthWatch {
        AuthWatch::with_settle(
            emit,
            sessions,
            session_id,
            scan_patterns_for(&AgentRunnerKind::ClaudeCode),
            TEST_SETTLE,
        )
    }

    fn wait_for<F: Fn() -> bool>(cond: F) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// R5: o padrão casou, mas quando o settle acorda a sessão já voltou a
    /// `Running` — string incidental ou retry recuperado, NÃO emite.
    #[test]
    fn running_at_settle_time_discards_without_emitting() {
        let (sessions, id) = manager_with(SessionStatus::Running);
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"not logged in");
        // Espera bem além do settle pra dar tempo do evento chegar, SE fosse
        // chegar -- o que a asserção diz que não pode acontecer.
        std::thread::sleep(TEST_SETTLE * 4);
        assert!(
            seen.lock().is_empty(),
            "emitiu mesmo com a sessão Running no settle: {:?}",
            seen.lock()
        );
    }

    /// R6: `Idle` no settle -- confirma e emite `{Runtime, kind}`.
    #[test]
    fn idle_at_settle_time_emits_runtime_alert() {
        let (sessions, id) = manager_with(SessionStatus::Idle { summary: None });
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"not logged in");
        assert!(wait_for(|| !seen.lock().is_empty()), "nenhum evento chegou");
        let payloads = seen.lock().clone();
        assert_eq!(payloads.len(), 1);
        // R8: a fase runtime carrega o session_id da sessão certa.
        assert_eq!(payloads[0].session_id, id);
        assert!(matches!(payloads[0].phase, AuthPhase::Runtime));
        assert!(matches!(payloads[0].kind, AuthAlertKind::NotLoggedIn));
    }

    /// R6: `AwaitingInput` também conta como "parou" -- mesma emissão.
    #[test]
    fn awaiting_input_at_settle_time_emits_runtime_alert() {
        let (sessions, id) = manager_with(SessionStatus::AwaitingInput {
            hint: None,
            reason: crate::session::AwaitingReason::Reply,
        });
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"not logged in");
        assert!(wait_for(|| !seen.lock().is_empty()));
    }

    /// R6: sessão que já terminou (`Exited`) também conta como "parou".
    #[test]
    fn exited_at_settle_time_emits_runtime_alert() {
        let (sessions, id) = manager_with(SessionStatus::Exited { code: 0 });
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"not logged in");
        assert!(wait_for(|| !seen.lock().is_empty()));
    }

    /// R6: sessão "ausente" -- `sessions.get` devolve `None` porque o id
    /// nunca existiu no manager. Mesmo braço de "parou".
    #[test]
    fn absent_session_at_settle_time_emits_runtime_alert() {
        let store = StdArc::new(crate::session::store::Store::open_in_memory().unwrap());
        let sessions: SharedSessionManager = StdArc::new(SessionManager::new(store));
        let id = SessionId::new_v4(); // nunca inserido
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"not logged in");
        assert!(wait_for(|| !seen.lock().is_empty()));
    }

    /// R7: um segundo casamento do MESMO kind antes do settle original
    /// acordar não arma um segundo settle -- só um evento sai.
    #[test]
    fn a_second_match_of_the_same_kind_before_settle_does_not_double_arm() {
        let (sessions, id) = manager_with(SessionStatus::Idle { summary: None });
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"not logged in");
        w.feed(b"not logged in"); // janela já foi limpa pelo 1º match -- o
                                  // 2º precisa do texto de novo pra casar
        std::thread::sleep(TEST_SETTLE * 4);
        // `assert_eq!` expande para um `match` sobre os dois lados, e um
        // `match` estende a vida de QUALQUER temporário criado ao avaliar o
        // scrutinee até o fim do bloco inteiro -- inclusive o `MutexGuard`
        // intermediário de `seen.lock()` usado só pra ler `.len()`. Chamar
        // `seen.lock()` DE NOVO dentro do `{:?}` do painel de falha, com o
        // primeiro guard ainda vivo, autotrava o `parking_lot::Mutex` (não
        // reentrante) -- só se manifesta quando a asserção FALHA de verdade,
        // e nesse ponto o teste trava para sempre em vez de reportar
        // vermelho. O `let` separado abaixo solta o primeiro guard antes do
        // `assert_eq!` sequer começar.
        let matched = seen.lock().len();
        assert_eq!(matched, 1, "dedupe falhou: {:?}", seen.lock());
    }

    /// R7 continuado: depois que a sessão volta a `Running` (progresso), o
    /// MESMO kind pode disparar de novo -- o dedupe não é permanente.
    #[test]
    fn progress_back_to_running_clears_the_dedupe_for_that_kind() {
        let (sessions, id) = manager_with(SessionStatus::Idle { summary: None });
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions.clone(), id);

        w.feed(b"not logged in");
        assert!(wait_for(|| seen.lock().len() == 1));

        // O dono rodou `/login` e o agente voltou a trabalhar.
        sessions.apply_status(id, SessionStatus::Running);
        // `feed` é o único ponto que consulta o progresso -- chamar com
        // texto neutro só pra disparar a checagem, sem casar nada sozinho.
        w.feed(b"trabalhando normalmente\n");

        // A mesma credencial quebra de novo mais tarde, depois de o agente
        // já ter voltado a rodar -- volta pra `Idle` antes do 2º match, senão
        // o 2º settle acordaria com a sessão AINDA `Running` e descartaria
        // por R5 (que é o comportamento certo, mas não o que este teste
        // quer provar: que o DEDUPE em si foi limpo pelo progresso).
        sessions.apply_status(id, SessionStatus::Idle { summary: None });
        w.feed(b"not logged in");
        assert!(
            wait_for(|| seen.lock().len() == 2),
            "kind emitido de novo depois do progresso: {:?}",
            seen.lock()
        );
    }

    /// R11 (metade testável em unit): o payload que atravessa o IPC só tem
    /// `session_id`, `phase` e `kind` -- nunca o texto cru que casou. A
    /// outra metade (o buffer em memória nunca é persistido) é verdade por
    /// construção: nada neste módulo grava em disco/`Store`.
    #[test]
    fn the_emitted_payload_never_carries_raw_matched_text() {
        let (sessions, id) = manager_with(SessionStatus::Idle { summary: None });
        let (emit, seen) = spy_sink();
        let mut w = watch(emit, sessions, id);

        w.feed(b"invalid api key -- account: someone@example.com");
        assert!(wait_for(|| !seen.lock().is_empty()));
        let raw = serde_json::to_string(&seen.lock()[0]).unwrap();
        assert!(
            !raw.contains("example.com") && !raw.contains("account"),
            "o payload vazou texto cru do stream: {raw}"
        );
    }
}
