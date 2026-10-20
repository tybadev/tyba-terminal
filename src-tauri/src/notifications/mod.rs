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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
/// Teto de entradas por sessão numa janela de [`SESSION_CAP_WINDOW_MS`]. Um
/// agente que gira `UserPromptSubmit → Stop` em laço não inunda o histórico
/// (e com ele a poda das 500 apagando o resto). Pedido de aprovação não conta:
/// pedido nunca se descarta.
const SESSION_CAP: u32 = 20;
const SESSION_CAP_WINDOW_MS: i64 = 60_000;
/// Até quanto tempo depois do "terminou" o settle ainda corrige o resumo dele.
const SUMMARY_FIX_MS: i64 = 10_000;
/// Intervalo mínimo entre dois [`EVENT_NOTIFICATIONS_CHANGED`]: a UI relê a
/// lista inteira a cada aviso.
const EMIT_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Approval,
    /// Amarelo do Claude decidido no prompt nativo. Não passa pelo
    /// `ApprovalsManager`: o core só observa (hooks `PermissionRequest` →
    /// `PostToolUse`) e a decisão é DEDUZIDA, como a linha
    /// `claude_native_inferred` do `approval_history`. `approval_id` é o id
    /// dessa linha, não de um pedido do TYBA.
    NativeApproval,
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
            NotificationKind::NativeApproval => "native_approval",
            NotificationKind::AgentFinished => "agent_finished",
            NotificationKind::AgentFailed => "agent_failed",
            NotificationKind::UsageLimit => "usage_limit",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "approval" => NotificationKind::Approval,
            "native_approval" => NotificationKind::NativeApproval,
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

/// Estado do aviso de limite de UMA janela. O limite é da conta, não da
/// sessão (mesmo motivo do [`crate::agent::usage::LimitTracker`]): quatro
/// sessões não dão quatro entradas do mesmo 87%, e um agente que forja
/// `_statusline` com `resets_at` diferente a cada evento não escapa da dedupe —
/// o `resets_at` não é chave.
#[derive(Debug, Clone, Copy, Default)]
struct UsageWindowState {
    /// Maior faixa já gravada nesta subida (0 = nenhuma).
    band: u8,
    /// Fim da janela que motivou a primeira entrada; passado, rearma. Só
    /// guarda um instante FUTURO: um `resets_at` já vencido rearmaria a cada
    /// evento.
    resets_at: Option<u64>,
}

impl UsageWindowState {
    /// A faixa a gravar agora, se houver. Rearma quando o uso cai abaixo de
    /// 85% ou quando o fim da janela guardada passou.
    fn observe(&mut self, pct: f64, resets_at: Option<u64>, now_secs: u64) -> Option<u8> {
        if self.band > 0 {
            let reset_passed = self.resets_at.is_some_and(|r| now_secs >= r);
            if pct < USAGE_WARN_PCT || reset_passed {
                *self = Self::default();
            }
        }
        let band = usage_band(pct)?;
        if band <= self.band {
            return None;
        }
        if self.band == 0 {
            self.resets_at = resets_at.filter(|r| *r > now_secs);
        }
        self.band = band;
        Some(band)
    }
}

/// Janela do teto por sessão.
#[derive(Debug, Clone, Copy)]
struct CapWindow {
    start_ms: i64,
    count: u32,
    dropped: u32,
}

/// Agrupamento de [`EVENT_NOTIFICATIONS_CHANGED`]: no máximo um a cada
/// [`EMIT_INTERVAL`], e sempre um depois da última mudança (trailing).
#[derive(Debug, Default)]
struct EmitThrottle {
    last: Option<Instant>,
    /// Já há uma emissão agendada; ela cobre o que mudar até sair.
    scheduled: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum EmitAction {
    Now,
    After(Duration),
    Skip,
}

impl EmitThrottle {
    fn on_change(&mut self, now: Instant) -> EmitAction {
        if self.scheduled {
            return EmitAction::Skip;
        }
        match self.last {
            Some(last) if now.duration_since(last) < EMIT_INTERVAL => {
                self.scheduled = true;
                EmitAction::After(EMIT_INTERVAL - now.duration_since(last))
            }
            _ => {
                self.last = Some(now);
                EmitAction::Now
            }
        }
    }

    /// A emissão agendada vai sair agora.
    fn on_timer(&mut self, now: Instant) {
        self.scheduled = false;
        self.last = Some(now);
    }
}

pub struct Notifications {
    store: Arc<Store>,
    sessions: SharedSessionManager,
    /// `[five_hour, seven_day]`: o estado não cresce.
    usage_state: Mutex<[UsageWindowState; 2]>,
    /// Sessões cujo próximo `running → idle` não é "terminou": o turno falhou
    /// (`StopFailure`, a falha já está no histórico) ou o próprio usuário o
    /// interrompeu. Sai daqui ao ser consumida ou no próximo `Running`.
    suppress_finished: Mutex<HashSet<SessionId>>,
    /// Teto por sessão; só sessões com janela aberta ficam aqui.
    caps: Mutex<HashMap<SessionId, CapWindow>>,
    emit: Mutex<EmitThrottle>,
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
            usage_state: Mutex::new([UsageWindowState::default(); 2]),
            suppress_finished: Mutex::new(HashSet::new()),
            caps: Mutex::new(HashMap::new()),
            emit: Mutex::new(EmitThrottle::default()),
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

    /// Cabe no teto da sessão? Conta a entrada se couber. As excedentes vão
    /// para um log, uma linha por janela, quando a janela fecha.
    fn admit(&self, session_id: SessionId, now_ms: i64) -> bool {
        let mut caps = self.caps.lock();
        caps.retain(|id, w| {
            let open = now_ms - w.start_ms < SESSION_CAP_WINDOW_MS;
            if !open && w.dropped > 0 {
                eprintln!(
                    "[tyba] notificações: sessão {id} passou de {SESSION_CAP} entradas por \
                     minuto; {} descartadas",
                    w.dropped
                );
            }
            open
        });
        let w = caps.entry(session_id).or_insert(CapWindow {
            start_ms: now_ms,
            count: 0,
            dropped: 0,
        });
        if w.count < SESSION_CAP {
            w.count += 1;
            true
        } else {
            w.dropped += 1;
            false
        }
    }

    /// Grava respeitando o teto da sessão. Tudo menos pedido de aprovação.
    fn insert_capped(&self, entry: &NotificationEntry) -> bool {
        self.admit(entry.session_id, entry.at_ms) && self.insert(entry)
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

    /// O prompt nativo do Claude abriu (`PermissionRequest`). Só registro: a
    /// decisão continua no terminal, e esta entrada nunca vira pedido do TYBA.
    /// Não passa pelo teto, como todo pedido. `inferred_id` é a linha
    /// deduzida do `approval_history` que a resolução vai procurar.
    pub fn native_prompt_opened(
        &self,
        session_id: SessionId,
        inferred_id: i64,
        risk: RiskLevel,
        command: &str,
    ) -> bool {
        let mut e = self.entry(session_id, NotificationKind::NativeApproval);
        e.category = NotificationCategory::Request;
        e.pending = true;
        e.approval_id = Some(inferred_id as u64);
        e.risk = Some(risk);
        e.command = Some(redact_cut(command, MAX_COMMAND_BYTES));
        self.insert(&e)
    }

    /// A chamada rodou: aprovada no prompt nativo (deduzido).
    pub fn native_prompt_approved(&self, inferred_id: i64) -> bool {
        self.store
            .approve_native_notification(inferred_id)
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// O pedido do plugin fechou sem veredito: deixa de estar pendente e não
    /// vira aprovado ("encerrado sem decisão").
    pub fn native_prompt_settled(&self, inferred_id: i64) -> bool {
        self.store
            .settle_native_notification(inferred_id)
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// O turno seguiu sem rodar (ou a sessão acabou): negadas (deduzido).
    pub fn native_prompts_denied(&self, session_id: SessionId) -> bool {
        self.store
            .deny_native_notifications_for_session(session_id)
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// `agent://alert` `stop_failure`. `reason` é o motivo fixo em pt-BR do
    /// core ([`crate::agent::usage::stop_failure_reason`]), nunca texto da API.
    pub fn stop_failure(&self, session_id: SessionId, reason: &str) -> bool {
        self.suppress_finished.lock().insert(session_id);
        let mut e = self.entry(session_id, NotificationKind::AgentFailed);
        e.category = NotificationCategory::Error;
        e.detail = Some(redacted(reason));
        self.insert_capped(&e)
    }

    /// O usuário interrompeu o turno (`AgentSignal::Interrupted`). O
    /// `running → idle` que vem a seguir não vira "terminou". Não grava nada.
    pub fn interrupted(&self, session_id: SessionId) -> bool {
        self.suppress_finished.lock().insert(session_id);
        false
    }

    /// Uso que o `statusLine` reportou. Uma entrada por (janela, faixa) para
    /// a conta inteira; faixas 85, 95 e 100. A entrada leva a sessão que
    /// reportou primeiro.
    pub fn usage(&self, session_id: SessionId, usage: &AgentUsage) -> bool {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.usage_at(session_id, usage, now_secs)
    }

    fn usage_at(&self, session_id: SessionId, usage: &AgentUsage, now_secs: u64) -> bool {
        let mut changed = false;
        for (i, window, rate) in [
            (0, LimitWindow::FiveHour, usage.rate_5h),
            (1, LimitWindow::SevenDay, usage.rate_7d),
        ] {
            let Some(rate) = rate else { continue };
            if self.usage_state.lock()[i]
                .observe(rate.pct, rate.resets_at, now_secs)
                .is_none()
            {
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
            changed |= self.insert_capped(&e);
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
            self.suppress_finished.lock().remove(&session.id);
            return false;
        }
        if let (Some(SessionStatus::Idle { summary: old }), SessionStatus::Idle { summary: new }) =
            (prev, &session.status)
        {
            return match new {
                Some(new) if old.as_ref() != Some(new) => {
                    self.fix_finished_summary(session.id, new)
                }
                _ => false,
            };
        }
        let Some((kind, category, detail)) = status_transition(prev, &session.status) else {
            return false;
        };
        if kind == NotificationKind::AgentFinished
            && self.suppress_finished.lock().remove(&session.id)
        {
            return false;
        }
        let mut e = Self::entry_for(session.id, Some(session), kind);
        e.category = category;
        e.detail = detail;
        self.insert_capped(&e)
    }

    /// `Idle → Idle` com outro resumo: o settle leu a fala final que o `Stop`
    /// ainda não tinha. Corrige a entrada "terminou" recém-gravada em vez de
    /// deixá-la com o texto velho.
    fn fix_finished_summary(&self, session_id: SessionId, summary: &str) -> bool {
        match self.store.update_last_finished_detail(
            session_id,
            &redacted(summary),
            now_ms() - SUMMARY_FIX_MS,
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                eprintln!("[tyba] notificações: falha ao corrigir resumo: {e}");
                false
            }
        }
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

/// Avisa o webview que o histórico mudou, agrupado: no máximo um aviso a cada
/// [`EMIT_INTERVAL`], com um último aviso depois da última mudança.
pub fn emit_changed(app: &AppHandle) {
    let Some(n) = app.try_state::<Notifications>() else {
        return;
    };
    let action = n.emit.lock().on_change(Instant::now());
    match action {
        EmitAction::Now => {
            let _ = app.emit(EVENT_NOTIFICATIONS_CHANGED, ());
        }
        EmitAction::After(wait) => {
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(wait);
                if let Some(n) = app.try_state::<Notifications>() {
                    n.emit.lock().on_timer(Instant::now());
                }
                let _ = app.emit(EVENT_NOTIFICATIONS_CHANGED, ());
            });
        }
        EmitAction::Skip => {}
    }
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
            subagents_working: false,
            agent_usage: None,
            shell_resume_offer: false,
            agent_conversation_cwd: None,
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

    const NOW_SECS: u64 = 1_800_000_000;

    fn rate5(pct: f64, resets_at: Option<u64>) -> AgentUsage {
        AgentUsage {
            rate_5h: Some(RateWindow { pct, resets_at }),
            ..AgentUsage::default()
        }
    }

    fn usage_entries(store: &Store, window: LimitWindow) -> Vec<NotificationEntry> {
        store
            .list_notifications(now_ms())
            .unwrap()
            .into_iter()
            .filter(|e| e.usage.is_some_and(|u| u.window == window))
            .collect()
    }

    #[test]
    fn usage_limit_uma_entrada_por_faixa() {
        let (n, store, session) = center();
        let at = |pct: f64| rate5(pct, Some(NOW_SECS + 3_600));
        assert!(!n.usage_at(session.id, &at(80.0), NOW_SECS));
        assert!(n.usage_at(session.id, &at(86.0), NOW_SECS));
        assert!(!n.usage_at(session.id, &at(90.0), NOW_SECS));
        assert!(n.usage_at(session.id, &at(96.0), NOW_SECS));
        assert!(!n.usage_at(session.id, &at(97.0), NOW_SECS));
        assert!(n.usage_at(session.id, &at(100.0), NOW_SECS));
        assert!(!n.usage_at(session.id, &at(100.0), NOW_SECS));

        let mine: Vec<_> = usage_entries(&store, LimitWindow::FiveHour)
            .iter()
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
    }

    /// `_statusline` forjado pelo socket de hook com `resets_at` diferente a
    /// cada evento (no passado e no futuro) não escapa da dedupe.
    #[test]
    fn usage_forjado_com_resets_at_variando_nao_inunda() {
        let (n, store, session) = center();
        for i in 0..250u64 {
            let usage = AgentUsage {
                rate_5h: Some(RateWindow {
                    pct: 85.0 + (i % 16) as f64,
                    resets_at: Some(NOW_SECS - 500 + i * 7),
                }),
                rate_7d: Some(RateWindow {
                    pct: 99.0 + (i % 3) as f64,
                    resets_at: Some(NOW_SECS - 1 + i),
                }),
                ..AgentUsage::default()
            };
            n.usage_at(session.id, &usage, NOW_SECS);
        }
        assert!(usage_entries(&store, LimitWindow::FiveHour).len() <= 3);
        assert!(usage_entries(&store, LimitWindow::SevenDay).len() <= 3);
    }

    /// O limite é da conta: quatro sessões no mesmo 87% dão uma entrada, com a
    /// sessão que reportou primeiro.
    #[test]
    fn usage_de_quatro_sessoes_da_uma_entrada_por_faixa() {
        let (n, store, session) = center();
        let others: Vec<SessionId> = (0..3).map(|_| SessionId::new_v4()).collect();
        let at = |pct: f64| rate5(pct, Some(NOW_SECS + 3_600));
        assert!(n.usage_at(session.id, &at(87.0), NOW_SECS));
        for id in &others {
            assert!(!n.usage_at(*id, &at(87.0), NOW_SECS));
        }
        assert!(n.usage_at(others[1], &at(95.5), NOW_SECS));
        assert!(!n.usage_at(session.id, &at(95.5), NOW_SECS));

        let list = usage_entries(&store, LimitWindow::FiveHour);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].session_id, session.id);
        assert_eq!(list[1].session_title, "corrige o parser");
        assert_eq!(list[0].session_id, others[1]);
    }

    #[test]
    fn usage_rearma_abaixo_de_85_e_quando_a_janela_reseta() {
        let (n, store, session) = center();
        let resets = NOW_SECS + 60;
        assert!(n.usage_at(session.id, &rate5(90.0, Some(resets)), NOW_SECS));
        assert!(!n.usage_at(session.id, &rate5(90.0, Some(resets)), NOW_SECS + 1));
        // Caiu abaixo de 85: rearma.
        assert!(!n.usage_at(session.id, &rate5(50.0, Some(resets)), NOW_SECS + 2));
        assert!(n.usage_at(session.id, &rate5(88.0, Some(resets)), NOW_SECS + 3));
        // A janela guardada passou: rearma mesmo sem cair.
        assert!(!n.usage_at(session.id, &rate5(88.0, Some(resets)), NOW_SECS + 59));
        assert!(n.usage_at(session.id, &rate5(88.0, Some(resets + 18_000)), resets));
        assert_eq!(usage_entries(&store, LimitWindow::FiveHour).len(), 3);
    }

    /// Um agente em laço `UserPromptSubmit → Stop` não inunda o histórico;
    /// pedido de aprovação nunca é descartado pelo teto.
    #[test]
    fn teto_por_sessao_descarta_excedentes_mas_nunca_pedido() {
        let (n, store, _) = center();
        let s = agent_session(SessionStatus::Idle {
            summary: Some("pronto".into()),
        });
        let inserted = (0..30)
            .filter(|_| n.status_changed(Some(&SessionStatus::Running), &s))
            .count();
        assert_eq!(inserted, SESSION_CAP as usize);
        for id in 0..5 {
            assert!(n.approval_requested(&request(id, s.id, "git push origin feat")));
        }
        // Outra sessão tem o próprio teto.
        let other = agent_session(SessionStatus::Idle { summary: None });
        assert!(n.status_changed(Some(&SessionStatus::Running), &other));

        let list = store.list_notifications(now_ms()).unwrap();
        let mine = |kind| {
            list.iter()
                .filter(|e| e.session_id == s.id && e.kind == kind)
                .count()
        };
        assert_eq!(mine(NotificationKind::AgentFinished), SESSION_CAP as usize);
        assert_eq!(mine(NotificationKind::Approval), 5);
    }

    #[test]
    fn teto_por_sessao_reabre_na_janela_seguinte() {
        let (n, _, _) = center();
        let sid = SessionId::new_v4();
        for _ in 0..SESSION_CAP {
            assert!(n.admit(sid, NOW));
        }
        assert!(!n.admit(sid, NOW + 1));
        assert!(!n.admit(sid, NOW + SESSION_CAP_WINDOW_MS - 1));
        assert!(n.admit(sid, NOW + SESSION_CAP_WINDOW_MS));
        // A janela fechada saiu do mapa: o estado não cresce com o tempo.
        assert_eq!(n.caps.lock().len(), 1);
    }

    #[test]
    fn emissao_agrupada_com_trailing() {
        let t0 = Instant::now();
        let ms = |m: u64| t0 + Duration::from_millis(m);
        let mut t = EmitThrottle::default();
        assert_eq!(t.on_change(ms(0)), EmitAction::Now);
        // Dentro do intervalo: agenda uma, para o fim do intervalo.
        assert_eq!(
            t.on_change(ms(10)),
            EmitAction::After(Duration::from_millis(240))
        );
        // As seguintes cabem na agendada.
        for m in [20, 100, 249] {
            assert_eq!(t.on_change(ms(m)), EmitAction::Skip);
        }
        t.on_timer(ms(250));
        // Logo depois da agendada sair, nova mudança agenda outra (trailing).
        assert_eq!(
            t.on_change(ms(300)),
            EmitAction::After(Duration::from_millis(200))
        );
        t.on_timer(ms(500));
        // Depois de um silêncio, sai na hora.
        assert_eq!(t.on_change(ms(900)), EmitAction::Now);
    }

    /// Interrupção do usuário leva a `Idle` e não é "terminou"; o turno
    /// seguinte volta a avisar.
    #[test]
    fn interrupcao_nao_vira_agent_finished() {
        let (n, store, session) = center();
        assert!(!n.interrupted(session.id));
        let mut s = session.clone();
        s.status = SessionStatus::Idle {
            summary: Some("qualquer texto".into()),
        };
        assert!(!n.status_changed(Some(&SessionStatus::Running), &s));
        s.status = SessionStatus::Running;
        n.status_changed(Some(&SessionStatus::Idle { summary: None }), &s);
        s.status = SessionStatus::Idle { summary: None };
        assert!(n.status_changed(Some(&SessionStatus::Running), &s));
        assert_eq!(store.list_notifications(now_ms()).unwrap().len(), 1);
    }

    /// O settle troca o resumo (`Idle → Idle`): a entrada recente é corrigida,
    /// não duplicada; uma entrada velha fica como está.
    #[test]
    fn settle_corrige_o_resumo_da_entrada_recente() {
        let (n, store, _) = center();
        let mut s = agent_session(SessionStatus::Idle {
            summary: Some("velho".into()),
        });
        assert!(n.status_changed(Some(&SessionStatus::Running), &s));
        let prev = s.status.clone();
        s.status = SessionStatus::Idle {
            summary: Some("novo ghp_abcdefghijklmnopqrstuvwxyz0123456789".into()),
        };
        assert!(n.status_changed(Some(&prev), &s));
        let list = store.list_notifications(now_ms()).unwrap();
        assert_eq!(list.len(), 1);
        let detail = list[0].detail.as_deref().unwrap();
        assert!(detail.starts_with("novo "));
        assert!(!detail.contains("ghp_"));

        // Entrada de mais de 10 s atrás: não é deste turno.
        let old = agent_session(SessionStatus::Idle {
            summary: Some("b".into()),
        });
        let mut e = entry(old.id, now_ms() - SUMMARY_FIX_MS - 1_000);
        e.detail = Some("a".into());
        store.insert_notification(&e, now_ms()).unwrap();
        assert!(!n.status_changed(
            Some(&SessionStatus::Idle {
                summary: Some("a".into())
            }),
            &old
        ));
        let list = store.list_notifications(now_ms()).unwrap();
        let kept = list.iter().find(|e| e.session_id == old.id).unwrap();
        assert_eq!(kept.detail.as_deref(), Some("a"));
    }

    #[test]
    fn titulo_da_sessao_sai_mascarado() {
        let store = Store::open_in_memory().unwrap();
        let mut e = entry(SessionId::new_v4(), NOW);
        e.session_title = "deploy ghp_abcdefghijklmnopqrstuvwxyz0123456789".into();
        store.insert_notification(&e, NOW).unwrap();
        let title = &store.list_notifications(NOW).unwrap()[0].session_title;
        assert!(!title.contains("ghp_"), "segredo vazou: {title}");
    }

    #[test]
    fn descartar_a_sessao_apaga_o_historico_dela() {
        let store = Store::open_in_memory().unwrap();
        let gone = SessionId::new_v4();
        let kept = SessionId::new_v4();
        store
            .insert_notification(&entry(gone, NOW - 2), NOW)
            .unwrap();
        store
            .insert_notification(&entry(kept, NOW - 1), NOW)
            .unwrap();
        store.remove_session(gone).unwrap();
        let list = store.list_notifications(NOW).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_id, kept);
    }

    fn native(store: &Store, session: SessionId) -> Vec<NotificationEntry> {
        store
            .list_notifications(now_ms())
            .unwrap()
            .into_iter()
            .filter(|e| e.session_id == session && e.kind == NotificationKind::NativeApproval)
            .collect()
    }

    /// Amarelo do prompt nativo: abre pendente, e a chamada que rodou fecha a
    /// MESMA entrada como aprovada (deduzido), sem tocar em pedido do TYBA com
    /// o mesmo número.
    #[test]
    fn prompt_nativo_aberto_e_aprovado_na_mesma_entrada() {
        let (n, store, session) = center();
        assert!(n.approval_requested(&request(5, session.id, "git push origin feat")));
        assert!(n.native_prompt_opened(session.id, 5, RiskLevel::Yellow, "cargo test"));
        let list = native(&store, session.id);
        assert_eq!(list.len(), 1);
        let e = &list[0];
        assert!(e.pending);
        assert_eq!(e.category, NotificationCategory::Request);
        assert_eq!(e.approval_id, Some(5));
        assert_eq!(e.risk, Some(RiskLevel::Yellow));
        assert_eq!(e.command.as_deref(), Some("cargo test"));
        assert_eq!(e.agent.as_deref(), Some("claude"));

        assert!(n.native_prompt_approved(5));
        let list = native(&store, session.id);
        assert_eq!(list.len(), 1, "resolver não cria outra entrada");
        assert!(!list[0].pending);
        assert_eq!(list[0].decision, Some(Decision::Approved));
        assert_eq!(list[0].category, NotificationCategory::Done);
        assert!(list[0].read);
        // O pedido do TYBA com o mesmo número segue pendente.
        let tyba = store
            .list_notifications(now_ms())
            .unwrap()
            .into_iter()
            .find(|e| e.kind == NotificationKind::Approval)
            .unwrap();
        assert!(tyba.pending);
        // E o pedido do TYBA resolvido não mexe na entrada nativa.
        assert!(n.approval_resolved(5, Decision::Denied));
        assert_eq!(
            native(&store, session.id)[0].decision,
            Some(Decision::Approved)
        );
    }

    /// O turno seguiu sem rodar: as pendentes nativas DESTA sessão viram
    /// negadas; as de outra sessão e o pedido do TYBA ficam.
    #[test]
    fn prompt_nativo_negado_so_na_sessao() {
        let (n, store, session) = center();
        let other = SessionId::new_v4();
        n.native_prompt_opened(session.id, 1, RiskLevel::Yellow, "cargo test");
        n.native_prompt_opened(session.id, 2, RiskLevel::Yellow, "npm i");
        n.native_prompt_opened(other, 3, RiskLevel::Yellow, "make");
        n.approval_requested(&request(9, session.id, "git push origin feat"));

        assert!(n.native_prompts_denied(session.id));
        let mine = native(&store, session.id);
        assert_eq!(mine.len(), 2);
        assert!(mine
            .iter()
            .all(|e| !e.pending && e.decision == Some(Decision::Denied) && e.read));
        assert!(native(&store, other)[0].pending);
        let tyba = store
            .list_notifications(now_ms())
            .unwrap()
            .into_iter()
            .find(|e| e.kind == NotificationKind::Approval)
            .unwrap();
        assert!(tyba.pending, "negação deduzida nunca fecha pedido do TYBA");
        // Nada mais pendente: negar de novo não muda nada.
        assert!(!n.native_prompts_denied(session.id));
    }

    /// Mesma regra do `approval_history`: o `PostToolUse` prova que a chamada
    /// rodou e conserta uma negação deduzida cedo demais.
    #[test]
    fn prompt_nativo_aprovado_depois_de_negado_por_deducao() {
        let (n, store, session) = center();
        n.native_prompt_opened(session.id, 1, RiskLevel::Yellow, "cargo test");
        n.native_prompts_denied(session.id);
        assert!(n.native_prompt_approved(1));
        assert_eq!(
            native(&store, session.id)[0].decision,
            Some(Decision::Approved)
        );
        // Aprovada não volta a negada nem é aprovada de novo.
        assert!(!n.native_prompts_denied(session.id));
        assert!(!n.native_prompt_approved(1));
    }

    #[test]
    fn prompt_nativo_mascara_o_comando_e_nao_passa_pelo_teto() {
        let (n, store, session) = center();
        for _ in 0..SESSION_CAP {
            n.stop_failure(session.id, "falhou");
        }
        let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let command = format!("curl -H 'x-api-key: {secret}' {}", "x".repeat(5_000));
        assert!(n.native_prompt_opened(session.id, 1, RiskLevel::Yellow, &command));
        let cmd = native(&store, session.id)[0].command.clone().unwrap();
        assert!(!cmd.contains("sk-ant-api03"), "segredo vazou: {cmd}");
        assert!(cmd.len() <= MAX_COMMAND_BYTES);
    }

    #[test]
    fn prompt_nativo_pendente_vira_encerrado_ao_subir() {
        let (n, store, session) = center();
        n.native_prompt_opened(session.id, 1, RiskLevel::Yellow, "cargo test");
        let sessions: SharedSessionManager = Arc::new(SessionManager::new(Arc::clone(&store)));
        let _reboot = Notifications::new(Arc::clone(&store), sessions);
        let e = &native(&store, session.id)[0];
        assert!(!e.pending);
        assert_eq!(e.decision, None);
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
        e.kind = NotificationKind::NativeApproval;
        assert_eq!(
            serde_json::to_value(&e).unwrap()["kind"],
            NotificationKind::NativeApproval.as_str()
        );
        assert_eq!(NotificationKind::NativeApproval.as_str(), "native_approval");
        assert_eq!(
            NotificationKind::parse("native_approval"),
            Some(NotificationKind::NativeApproval)
        );
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
