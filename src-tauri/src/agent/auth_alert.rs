//! Entrega C — "sessão de agente nunca morre em silêncio por autenticação".
//!
//! Módulo-semente: o contrato core→webview que tanto o preflight
//! ([`super::auth_preflight`]) quanto o scanner de runtime ([`super::auth_watch`],
//! sobre [`crate::status::auth_scan`]) emitem. Um evento NOVO, não um reuso de
//! `agent://sandbox-warning` (aviso de jaula, sem ação) nem de `agent://open-url`
//! (URL que o dono decide abrir) — nenhum dos dois responde "a sessão parou de
//! autenticar", que é a pergunta desta entrega.

use crate::session::SessionId;

pub const EVENT_AGENT_AUTH_ALERT: &str = "agent://auth-alert";

/// O que quebrou. Cada variante é uma AÇÃO possível pro dono — nunca prosa:
/// quem monta a frase em pt-BR é o front (`authAlert.ts`), o mesmo desenho de
/// `SandboxWarningKind` (`agent/credentials.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AuthAlertKind {
    NotLoggedIn,
    TokenExpiredOrRevoked,
    CreditBalanceLow,
    InvalidApiKey,
}

/// De onde o alerta veio. O preflight roda ANTES do agente falar (silêncio
/// puro, ainda sem prompt na tela); o runtime vem do stream depois que o
/// turno já começou. O front roteia por isto — preflight vira toast, runtime
/// vira faixa na sessão (ver `authAlert.ts` e o handler em `App.tsx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthPhase {
    Preflight,
    Runtime,
}

/// Princípio #10 do CLAUDE.md (secrets nunca em log/scrollback persistido):
/// SÓ `kind` e `phase` viajam pro webview — nunca a string crua do
/// stdout/stderr do `claude`, que pode carregar path de credencial, e-mail de
/// conta ou qualquer outro dado que o processo tenha escrito.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthAlertPayload {
    pub session_id: SessionId,
    pub phase: AuthPhase,
    pub kind: AuthAlertKind,
}
