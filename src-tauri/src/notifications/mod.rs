//! Histórico de notificações (painel Notificações).
//!
//! O core grava cada evento que merece atenção — pedido de aprovação, falha de
//! agente, agente que terminou, limite de uso — no SQLite, com segredos
//! mascarados ANTES de gravar (princípio #10), e guarda o lido/não lido. O
//! webview só lê a lista (`notifications_list`) e relê quando o core avisa em
//! [`EVENT_NOTIFICATIONS_CHANGED`]. O texto é montado na UI a partir de `kind`
//! e dos campos; aqui não sai frase pronta, só o `detail` que o próprio core
//! já escreve em pt-BR.
//!
//! Retenção: 7 dias e no máximo 500 linhas, podadas na inserção.

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::agent::usage::{AgentUsage, LimitWindow, USAGE_WARN_PCT};
use crate::approvals::{ApprovalRequest, Decision, RiskLevel};
use crate::session::store::{Store, StoreError};
use crate::session::{
    AgentRunnerKind, Session, SessionId, SessionKind, SessionStatus, SharedSessionManager,
};

/// Core→webview: mudou algo no histórico (entrou, resolveu, leu). Payload `null`.
pub const EVENT_NOTIFICATIONS_CHANGED: &str = "notifications://changed";

/// Quanto tempo uma entrada vive.
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Teto de linhas; as mais antigas saem primeiro.
pub const MAX_ENTRIES: i64 = 500;
/// Teto do comando gravado num pedido de aprovação.
const MAX_COMMAND_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Approval,
    AgentFinished,
    AgentFailed,
    UsageLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationCategory {
    Request,
    Error,
    Done,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct UsageInfo {
    pub window: LimitWindow,
    pub pct: f64,
    pub resets_at: Option<u64>,
}

/// Espelha `NotificationEntry` de `src/lib/ipc.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NotificationEntry {
    pub id: u64,
    pub at_ms: i64,
    pub session_id: SessionId,
    pub session_title: String,
    pub agent: Option<String>,
    pub kind: NotificationKind,
    pub category: NotificationCategory,
    pub approval_id: Option<u64>,
    pub pending: bool,
    pub decision: Option<Decision>,
    pub risk: Option<RiskLevel>,
    pub command: Option<String>,
    pub detail: Option<String>,
    pub usage: Option<UsageInfo>,
    pub read: bool,
}

impl NotificationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NotificationKind::Approval => "approval",
            NotificationKind::AgentFinished => "agent_finished",
            NotificationKind::AgentFailed => "agent_failed",
            NotificationKind::UsageLimit => "usage_limit",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "approval" => NotificationKind::Approval,
            "agent_finished" => NotificationKind::AgentFinished,
            "agent_failed" => NotificationKind::AgentFailed,
            "usage_limit" => NotificationKind::UsageLimit,
            _ => return None,
        })
    }
}

impl NotificationCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NotificationCategory::Request => "request",
            NotificationCategory::Error => "error",
            NotificationCategory::Done => "done",
            NotificationCategory::Info => "info",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "request" => NotificationCategory::Request,
            "error" => NotificationCategory::Error,
            "done" => NotificationCategory::Done,
            "info" => NotificationCategory::Info,
            _ => return None,
        })
    }
}

pub(crate) fn window_str(w: LimitWindow) -> &'static str {
    match w {
        LimitWindow::FiveHour => "five_hour",
        LimitWindow::SevenDay => "seven_day",
    }
}

pub(crate) fn parse_window(raw: &str) -> Option<LimitWindow> {
    match raw {
        "five_hour" => Some(LimitWindow::FiveHour),
        "seven_day" => Some(LimitWindow::SevenDay),
        _ => None,
    }
}

/// Mascara e corta. A ordem importa: cortar antes de mascarar poderia partir
/// um segredo ao meio, e a metade que sobra já não casa com o padrão.
fn redact_cut(text: &str, max_bytes: usize) -> String {
    let redacted = crate::session::redact::redact(text);
    let mut end = redacted.len().min(max_bytes);
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    redacted[..end].to_string()
}

fn redacted(text: &str) -> String {
    crate::session::redact::redact(text).into_owned()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `"claude" | "codex" | nome do custom`; `None` fora de sessão de agente.
fn agent_name(kind: &SessionKind) -> Option<String> {
    match kind {
        SessionKind::Agent { runner } => Some(match runner {
            AgentRunnerKind::ClaudeCode => "claude".to_string(),
            AgentRunnerKind::Codex => "codex".to_string(),
            AgentRunnerKind::Custom(name) => name.clone(),
        }),
        _ => None,
    }
}

/// O que uma mudança de status de sessão de agente vira, se virar algo. Só a
/// TRANSIÇÃO conta: o mesmo estado emitido de novo não produz nada.
///
/// `Exited { code: -1 }` é a sentinela de "o core não sabe o código" (ver
/// `session_exited` em `lib.rs`, que nunca recebe o código real) — toda saída
/// de agente, inclusive o `/exit` do dono, chega assim. Tratá-la como falha
/// encheria o histórico de "saiu com -1" a cada aba fechada.
fn status_transition(
    prev: Option<&SessionStatus>,
    next: &SessionStatus,
) -> Option<(NotificationKind, NotificationCategory, Option<String>)> {
    match next {
        SessionStatus::Idle { summary } if matches!(prev, Some(SessionStatus::Running)) => Some((
            NotificationKind::AgentFinished,
            NotificationCategory::Done,
            summary.as_deref().map(redacted),
        )),
        SessionStatus::Failed { reason } if !matches!(prev, Some(SessionStatus::Failed { .. })) => {
            Some((
                NotificationKind::AgentFailed,
                NotificationCategory::Error,
                Some(redacted(reason)),
            ))
        }
        SessionStatus::Exited { code }
            if *code != 0
                && *code != -1
                && !matches!(
                    prev,
                    Some(SessionStatus::Exited { .. } | SessionStatus::Failed { .. })
                ) =>
        {
            Some((
                NotificationKind::AgentFailed,
                NotificationCategory::Error,
                Some(format!("saiu com {code}")),
            ))
        }
        _ => None,
    }
}

/// Faixa de aviso de limite: 85, 95 ou 100. `None` abaixo de 85%.
fn usage_band(pct: f64) -> Option<u8> {
    if pct >= 100.0 {
        Some(100)
    } else if pct >= 95.0 {
        Some(95)
    } else if pct >= USAGE_WARN_PCT {
        Some(85)
    } else {
        None
    }
}

/// Chave de dedupe do limite: sessão, janela (o tipo e o `resets_at` que a
/// identifica — janela nova rearma) e faixa.
type UsageKey = (SessionId, &'static str, Option<u64>, u8);

pub struct Notifications {
    store: Arc<Store>,
    sessions: SharedSessionManager,
    usage_seen: Mutex<HashSet<UsageKey>>,
    /// Sessões cujo turno acabou de falhar (`StopFailure`). O core leva a
    /// sessão a `Idle` logo depois, e esse `running → idle` não é "terminou":
    /// a falha já está no histórico. Sai daqui no próximo `Running`.
    stop_failed: Mutex<HashSet<SessionId>>,
}

impl Notifications {
    /// Pedidos de aprovação vivem só em memória: ao subir, todo pedido que o
    /// banco ainda tem como pendente é de um processo que morreu, e ninguém vai
    /// decidi-lo. Vira `pending=false` com `decision=null`.
    pub fn new(store: Arc<Store>, sessions: SharedSessionManager) -> Self {
        if let Err(e) = store.abandon_pending_notifications() {
            eprintln!("[tyba] notificações: pedidos pendentes não encerrados: {e}");
        }
        Self {
            store,
            sessions,
            usage_seen: Mutex::new(HashSet::new()),
            stop_failed: Mutex::new(HashSet::new()),
        }
    }

    fn entry(&self, session_id: SessionId, kind: NotificationKind) -> NotificationEntry {
        let session = self.sessions.get(session_id);
        Self::entry_for(session_id, session.as_ref(), kind)
    }

    fn entry_for(
        session_id: SessionId,
        session: Option<&Session>,
        kind: NotificationKind,
    ) -> NotificationEntry {
        NotificationEntry {
            id: 0,
            at_ms: now_ms(),
            session_id,
            session_title: session.map(|s| s.title.clone()).unwrap_or_default(),
            agent: session.and_then(|s| agent_name(&s.kind)),
            kind,
            category: NotificationCategory::Info,
            approval_id: None,
            pending: false,
            decision: None,
            risk: None,
            command: None,
            detail: None,
            usage: None,
            read: false,
        }
    }

    fn insert(&self, entry: &NotificationEntry) -> bool {
        match self.store.insert_notification(entry, entry.at_ms) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("[tyba] notificações: falha ao gravar: {e}");
                false
            }
        }
    }

    /// `approvals://requested`.
    pub fn approval_requested(&self, request: &ApprovalRequest) -> bool {
        let mut e = self.entry(request.session_id, NotificationKind::Approval);
        e.category = NotificationCategory::Request;
        e.pending = true;
        e.approval_id = Some(request.id);
        e.risk = Some(request.risk);
        e.command = Some(redact_cut(&request.command, MAX_COMMAND_BYTES));
        self.insert(&e)
    }

    /// `approvals://resolved`: atualiza a entrada do pedido, não cria outra.
    pub fn approval_resolved(&self, approval_id: u64, decision: Decision) -> bool {
        self.store
            .resolve_notification_approval(approval_id, decision)
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// `agent://alert` `stop_failure`. `reason` é o motivo fixo em pt-BR do
    /// core ([`crate::agent::usage::stop_failure_reason`]), nunca texto da API.
    pub fn stop_failure(&self, session_id: SessionId, reason: &str) -> bool {
        self.stop_failed.lock().insert(session_id);
        let mut e = self.entry(session_id, NotificationKind::AgentFailed);
        e.category = NotificationCategory::Error;
        e.detail = Some(redacted(reason));
        self.insert(&e)
    }

    /// Uso que o `statusLine` reportou. Uma entrada por (sessão, janela,
    /// faixa); faixas 85, 95 e 100.
    pub fn usage(&self, session_id: SessionId, usage: &AgentUsage) -> bool {
        let mut changed = false;
        for (window, rate) in [
            (LimitWindow::FiveHour, usage.rate_5h),
            (LimitWindow::SevenDay, usage.rate_7d),
        ] {
            let Some(rate) = rate else { continue };
            let Some(band) = usage_band(rate.pct) else {
                continue;
            };
            let key = (session_id, window_str(window), rate.resets_at, band);
            if !self.usage_seen.lock().insert(key) {
                continue;
            }
            let mut e = self.entry(session_id, NotificationKind::UsageLimit);
            e.category = if rate.pct >= 100.0 {
                NotificationCategory::Error
            } else {
                NotificationCategory::Info
            };
            e.usage = Some(UsageInfo {
                window,
                pct: rate.pct,
                resets_at: rate.resets_at,
            });
            changed |= self.insert(&e);
        }
        changed
    }

    /// Mudança de status emitida em `session://status/<id>`. Só sessão de
    /// agente; só na transição.
    pub fn status_changed(&self, prev: Option<&SessionStatus>, session: &Session) -> bool {
        if !matches!(session.kind, SessionKind::Agent { .. }) {
            return false;
        }
        if matches!(session.status, SessionStatus::Running) {
            self.stop_failed.lock().remove(&session.id);
            return false;
        }
        let Some((kind, category, detail)) = status_transition(prev, &session.status) else {
            return false;
        };
        if kind == NotificationKind::AgentFinished && self.stop_failed.lock().remove(&session.id) {
            return false;
        }
        let mut e = Self::entry_for(session.id, Some(session), kind);
        e.category = category;
        e.detail = detail;
        self.insert(&e)
    }

    /// Mais nova primeiro; até 500, dos últimos 7 dias.
    pub fn list(&self) -> Result<Vec<NotificationEntry>, StoreError> {
        self.store.list_notifications(now_ms())
    }

    /// `ids` = esses; `None` = todas.
    pub fn mark_read(&self, ids: Option<&[u64]>) -> Result<bool, StoreError> {
        self.store.mark_notifications_read(ids).map(|n| n > 0)
    }
}

/// Avisa o webview que o histórico mudou.
pub fn emit_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_NOTIFICATIONS_CHANGED, ());
}

/// Aplica `f` ao histórico (se já existe — antes do `manage` do boot não há
/// nada a gravar) e emite [`EVENT_NOTIFICATIONS_CHANGED`] quando algo mudou.
pub fn record(app: &AppHandle, f: impl FnOnce(&Notifications) -> bool) {
    if let Some(n) = app.try_state::<Notifications>() {
        if f(&n) {
            emit_changed(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::usage::RateWindow;
    use crate::session::SessionManager;

    fn agent_session(status: SessionStatus) -> Session {
        Session {
            id: SessionId::new_v4(),
            kind: SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode,
            },
            title: "corrige o parser".into(),
            repo_root: None,
            worktree: None,
            status,
            attention: false,
            created_at: chrono::Utc::now(),
            cwd: None,
            connection: crate::session::ConnectionState::default(),
            connection_failure: None,
            agent_conversation_id: None,
            observed: None,
            agent_model: None,
            status_note: None,
            opened_by_gate: false,
            did_work: false,
            agent_usage: None,
        }
    }

    /// Histórico com uma sessão de agente conhecida pelo `SessionManager`.
    fn center() -> (Notifications, Arc<Store>, Session) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let session = agent_session(SessionStatus::Exited { code: 0 });
        store.upsert_session(&session).unwrap();
        let sessions: SharedSessionManager = Arc::new(SessionManager::new(Arc::clone(&store)));
        sessions.restore().unwrap();
        (
            Notifications::new(Arc::clone(&store), sessions),
            store,
            session,
        )
    }

    fn entry(session_id: SessionId, at_ms: i64) -> NotificationEntry {
        let mut e = Notifications::entry_for(session_id, None, NotificationKind::AgentFinished);
        e.at_ms = at_ms;
        e.category = NotificationCategory::Done;
        e
    }

    fn request(id: u64, session_id: SessionId, command: &str) -> ApprovalRequest {
        ApprovalRequest {
            id,
            session_id,
            risk: RiskLevel::Red,
            risk_reason: None,
            allowlist_key: command.into(),
            command: command.into(),
            cwd: None,
            context: None,
            requested_at_ms: 0,
            agent_id: None,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn store_lista_da_mais_nova_para_a_mais_velha() {
        let store = Store::open_in_memory().unwrap();
        let sid = SessionId::new_v4();
        for at in [NOW - 3_000, NOW - 1_000, NOW - 2_000] {
            store.insert_notification(&entry(sid, at), NOW).unwrap();
        }
        let list = store.list_notifications(NOW).unwrap();
        let ats: Vec<i64> = list.iter().map(|e| e.at_ms).collect();
        assert_eq!(ats, vec![NOW - 1_000, NOW - 2_000, NOW - 3_000]);
        assert!(list.iter().all(|e| e.session_id == sid && !e.read));
        assert_eq!(list[0].kind, NotificationKind::AgentFinished);
    }

    #[test]
    fn store_poda_por_idade() {
        let store = Store::open_in_memory().unwrap();
        let sid = SessionId::new_v4();
        store
            .insert_notification(&entry(sid, NOW - RETENTION_MS - 1), NOW)
            .unwrap();
        store
            .insert_notification(&entry(sid, NOW - 5), NOW)
            .unwrap();
        let list = store.list_notifications(NOW).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].at_ms, NOW - 5);
        // E a linha velha saiu do banco, não só da lista.
        assert_eq!(store.count_notifications().unwrap(), 1);
    }

    #[test]
    fn store_poda_por_contagem_mantendo_as_mais_novas() {
        let store = Store::open_in_memory().unwrap();
        let sid = SessionId::new_v4();
        for i in 0..(MAX_ENTRIES + 10) {
            store
                .insert_notification(&entry(sid, NOW - 100_000 + i), NOW)
                .unwrap();
        }
        assert_eq!(store.count_notifications().unwrap(), MAX_ENTRIES);
        let list = store.list_notifications(NOW).unwrap();
        assert_eq!(list.len() as i64, MAX_ENTRIES);
        assert_eq!(list[0].at_ms, NOW - 100_000 + MAX_ENTRIES + 9);
        assert_eq!(list.last().unwrap().at_ms, NOW - 100_000 + 10);
    }

    #[test]
    fn store_marca_lido_por_ids_e_todas() {
        let store = Store::open_in_memory().unwrap();
        let sid = SessionId::new_v4();
        let a = store
            .insert_notification(&entry(sid, NOW - 3), NOW)
            .unwrap();
        let _b = store
            .insert_notification(&entry(sid, NOW - 2), NOW)
            .unwrap();
        let _c = store
            .insert_notification(&entry(sid, NOW - 1), NOW)
            .unwrap();
        store.mark_notifications_read(Some(&[a])).unwrap();
        let read: Vec<(u64, bool)> = store
            .list_notifications(NOW)
            .unwrap()
            .iter()
            .map(|e| (e.id, e.read))
            .collect();
        assert_eq!(read.iter().filter(|(_, r)| *r).count(), 1);
        assert!(read.contains(&(a, true)));
        store.mark_notifications_read(None).unwrap();
        assert!(store
            .list_notifications(NOW)
            .unwrap()
            .iter()
            .all(|e| e.read));
    }

    #[test]
    fn pedido_resolvido_atualiza_a_mesma_entrada() {
        let (n, store, session) = center();
        assert!(n.approval_requested(&request(7, session.id, "git push origin feat")));
        let list = store.list_notifications(now_ms()).unwrap();
        assert_eq!(list.len(), 1);
        let e = &list[0];
        assert_eq!(e.kind, NotificationKind::Approval);
        assert_eq!(e.category, NotificationCategory::Request);
        assert!(e.pending);
        assert_eq!(e.approval_id, Some(7));
        assert_eq!(e.risk, Some(RiskLevel::Red));
        assert_eq!(e.session_title, "corrige o parser");
        assert_eq!(e.agent.as_deref(), Some("claude"));

        assert!(n.approval_resolved(7, Decision::Denied));
        let list = store.list_notifications(now_ms()).unwrap();
        assert_eq!(list.len(), 1, "resolver não cria outra entrada");
        let e = &list[0];
        assert!(!e.pending);
        assert_eq!(e.decision, Some(Decision::Denied));
        assert_eq!(e.category, NotificationCategory::Done);
        assert!(e.read);
        // Resolver de novo não acha pedido pendente.
        assert!(!n.approval_resolved(7, Decision::Approved));
    }

    #[test]
    fn pendente_de_processo_anterior_vira_abandonado_ao_subir() {
        let (n, store, session) = center();
        n.approval_requested(&request(1, session.id, "rm -rf build"));
        let sessions: SharedSessionManager = Arc::new(SessionManager::new(Arc::clone(&store)));
        let _reboot = Notifications::new(Arc::clone(&store), sessions);
        let e = &store.list_notifications(now_ms()).unwrap()[0];
        assert!(!e.pending);
        assert_eq!(e.decision, None);
        assert_ne!(e.category, NotificationCategory::Request);
    }

    #[test]
    fn comando_com_segredo_sai_mascarado_e_cortado() {
        let (n, store, session) = center();
        let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let command = format!("curl -H 'x-api-key: {secret}' https://exemplo.invalid");
        n.approval_requested(&request(1, session.id, &command));
        let long = format!("echo {}", "x".repeat(5_000));
        n.approval_requested(&request(2, session.id, &long));
        let list = store.list_notifications(now_ms()).unwrap();
        let masked = list.iter().find(|e| e.approval_id == Some(1)).unwrap();
        let cmd = masked.command.as_deref().unwrap();
        assert!(!cmd.contains(secret), "segredo vazou: {cmd}");
        assert!(!cmd.contains("sk-ant-api03"), "segredo vazou: {cmd}");
        let cut = list.iter().find(|e| e.approval_id == Some(2)).unwrap();
        assert!(cut.command.as_deref().unwrap().len() <= MAX_COMMAND_BYTES);
    }

    #[test]
    fn running_para_idle_cria_uma_vez() {
        let (n, store, _) = center();
        let mut s = agent_session(SessionStatus::Idle {
            summary: Some("pronto".into()),
        });
        assert!(n.status_changed(Some(&SessionStatus::Running), &s));
        // A mesma emissão de novo (idle → idle) não cria.
        assert!(!n.status_changed(Some(&s.status.clone()), &s));
        s.status = SessionStatus::Idle { summary: None };
        assert!(!n.status_changed(
            Some(&SessionStatus::Idle {
                summary: Some("pronto".into())
            }),
            &s
        ));
        let list = store.list_notifications(now_ms()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, NotificationKind::AgentFinished);
        assert_eq!(list[0].category, NotificationCategory::Done);
        assert_eq!(list[0].detail.as_deref(), Some("pronto"));
    }

    #[test]
    fn failed_cria_agent_failed_e_shell_nao_cria() {
        let (n, store, _) = center();
        let s = agent_session(SessionStatus::Failed {
            reason: "spawn falhou".into(),
        });
        assert!(n.status_changed(Some(&SessionStatus::Running), &s));
        assert!(!n.status_changed(Some(&s.status.clone()), &s));
        let mut exited = agent_session(SessionStatus::Exited { code: 2 });
        assert!(n.status_changed(Some(&SessionStatus::Running), &exited));
        // A sentinela -1 (código desconhecido) não é falha.
        exited.status = SessionStatus::Exited { code: -1 };
        assert!(!n.status_changed(Some(&SessionStatus::Running), &exited));
        let mut shell = agent_session(SessionStatus::Failed { reason: "x".into() });
        shell.kind = SessionKind::Shell;
        assert!(!n.status_changed(Some(&SessionStatus::Running), &shell));

        let list = store.list_notifications(now_ms()).unwrap();
        let details: Vec<_> = list.iter().map(|e| e.detail.as_deref()).collect();
        assert_eq!(details, vec![Some("saiu com 2"), Some("spawn falhou")]);
        assert!(list.iter().all(|e| e.kind == NotificationKind::AgentFailed
            && e.category == NotificationCategory::Error));
    }

    #[test]
    fn stop_failure_nao_vira_tambem_agent_finished() {
        let (n, store, session) = center();
        assert!(n.stop_failure(session.id, "limite de uso atingido"));
        let mut s = session.clone();
        s.status = SessionStatus::Idle {
            summary: Some("Falhou: limite de uso atingido".into()),
        };
        assert!(!n.status_changed(Some(&SessionStatus::Running), &s));
        // O próximo turno normal volta a avisar.
        s.status = SessionStatus::Running;
        n.status_changed(Some(&SessionStatus::Idle { summary: None }), &s);
        s.status = SessionStatus::Idle { summary: None };
        assert!(n.status_changed(Some(&SessionStatus::Running), &s));

        let list = store.list_notifications(now_ms()).unwrap();
        assert_eq!(list.len(), 2);
        let failed = list
            .iter()
            .find(|e| e.kind == NotificationKind::AgentFailed)
            .unwrap();
        assert_eq!(failed.detail.as_deref(), Some("limite de uso atingido"));
        assert_eq!(failed.category, NotificationCategory::Error);
    }

    #[test]
    fn usage_limit_uma_entrada_por_faixa() {
        let (n, store, session) = center();
        let at = |pct: f64| AgentUsage {
            rate_5h: Some(RateWindow {
                pct,
                resets_at: Some(1_900_000_000),
            }),
            ..AgentUsage::default()
        };
        assert!(!n.usage(session.id, &at(80.0)));
        assert!(n.usage(session.id, &at(86.0)));
        assert!(!n.usage(session.id, &at(90.0)));
        assert!(n.usage(session.id, &at(96.0)));
        assert!(!n.usage(session.id, &at(97.0)));
        assert!(n.usage(session.id, &at(100.0)));
        assert!(!n.usage(session.id, &at(100.0)));
        // Outra sessão tem as próprias faixas.
        assert!(n.usage(SessionId::new_v4(), &at(88.0)));

        let list = store.list_notifications(now_ms()).unwrap();
        assert_eq!(list.len(), 4);
        let mine: Vec<_> = list
            .iter()
            .filter(|e| e.session_id == session.id)
            .map(|e| (e.usage.unwrap().pct, e.category))
            .collect();
        assert_eq!(
            mine,
            vec![
                (100.0, NotificationCategory::Error),
                (96.0, NotificationCategory::Info),
                (86.0, NotificationCategory::Info),
            ]
        );
        assert!(list
            .iter()
            .all(|e| e.usage.unwrap().window == LimitWindow::FiveHour));
    }

    #[test]
    fn serializa_com_os_nomes_do_contrato() {
        let mut e = entry(SessionId::nil(), NOW);
        e.kind = NotificationKind::UsageLimit;
        e.category = NotificationCategory::Info;
        e.usage = Some(UsageInfo {
            window: LimitWindow::SevenDay,
            pct: 90.0,
            resets_at: None,
        });
        e.decision = Some(Decision::ApprovedAlways);
        e.risk = Some(RiskLevel::Yellow);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "usage_limit");
        assert_eq!(v["category"], "info");
        assert_eq!(v["usage"]["window"], "seven_day");
        assert_eq!(v["decision"], "approved_always");
        assert_eq!(v["risk"], "yellow");
        for field in [
            "id",
            "at_ms",
            "session_id",
            "session_title",
            "agent",
            "approval_id",
            "pending",
            "command",
            "detail",
            "read",
        ] {
            assert!(v.get(field).is_some(), "falta {field}");
        }
    }
}
