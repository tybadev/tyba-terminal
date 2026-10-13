//! Dados de uso do Claude (proposta 1 da ia-v2) e falha de turno (proposta 4).
//!
//! A fonte é o JSON que o Claude Code manda no stdin do comando de
//! `statusLine` depois de cada mensagem do assistente. O `tyba _statusline`
//! (ver [`super::statusline`]) extrai daqui só números e o nome do modelo —
//! nunca prompt, `cwd`, `transcript_path` ou `session_id` — e manda ao core
//! pelo socket de hooks da sessão. O core reaplica [`AgentUsage::sanitized`]:
//! o socket é alcançável de dentro da jaula, então o que chega dele é dado do
//! agente, não do TYBA.
//!
//! Nada daqui vai para o SQLite: vive em memória na `Session` e some com ela.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Nome do evento sintético que o `_statusline` manda pelo socket de hooks.
pub const STATUSLINE_HOOK_EVENT: &str = "TybaStatusline";

/// Evento core→webview dos avisos de agente (falha de turno, limite de uso).
pub const EVENT_AGENT_ALERT: &str = "agent://alert";

/// A partir de quanto uso de uma janela de limite o TYBA avisa.
pub const USAGE_WARN_PCT: f64 = 85.0;

const MAX_MODEL_CHARS: usize = 64;
/// Teto de sanidade para percentuais: o `spend_limit` pode passar de 100, as
/// janelas não; acima disso é lixo.
const MAX_PCT: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateWindow {
    pub pct: f64,
    /// Epoch em segundos.
    #[serde(default)]
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentUsage {
    /// `model.display_name`, ou o `model.id` quando não há nome.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context_pct: Option<f64>,
    #[serde(default)]
    pub context_window_size: Option<u64>,
    #[serde(default)]
    pub tokens_in: Option<u64>,
    #[serde(default)]
    pub tokens_out: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub lines_added: Option<u64>,
    #[serde(default)]
    pub lines_removed: Option<u64>,
    #[serde(default)]
    pub rate_5h: Option<RateWindow>,
    #[serde(default)]
    pub rate_7d: Option<RateWindow>,
}

fn num(v: &Value, path: &str) -> Option<f64> {
    v.pointer(path).and_then(Value::as_f64)
}

fn uint(v: &Value, path: &str) -> Option<u64> {
    v.pointer(path).and_then(|n| {
        n.as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
    })
}

fn text(v: &Value, path: &str) -> Option<String> {
    v.pointer(path)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn window(v: &Value, path: &str) -> Option<RateWindow> {
    Some(RateWindow {
        pct: num(v, &format!("{path}/used_percentage"))?,
        resets_at: uint(v, &format!("{path}/resets_at")),
    })
}

/// O JSON do `statusLine` (bytes crus do stdin) nos números que o TYBA usa.
/// `None` para entrada que não é objeto JSON ou que não traz nenhum campo
/// conhecido. Campo ausente, `null` ou de tipo errado fica `None` — o Claude
/// manda `null` em `used_percentage` e `current_usage` no começo da sessão.
pub fn parse_statusline(input: &[u8]) -> Option<AgentUsage> {
    let v: Value = serde_json::from_slice(input).ok()?;
    if !v.is_object() {
        return None;
    }
    let usage = AgentUsage {
        model: text(&v, "/model/display_name").or_else(|| text(&v, "/model/id")),
        context_pct: num(&v, "/context_window/used_percentage"),
        context_window_size: uint(&v, "/context_window/context_window_size"),
        tokens_in: uint(&v, "/context_window/total_input_tokens"),
        tokens_out: uint(&v, "/context_window/total_output_tokens"),
        cost_usd: num(&v, "/cost/total_cost_usd"),
        duration_ms: uint(&v, "/cost/total_duration_ms"),
        lines_added: uint(&v, "/cost/total_lines_added"),
        lines_removed: uint(&v, "/cost/total_lines_removed"),
        rate_5h: window(&v, "/rate_limits/five_hour"),
        rate_7d: window(&v, "/rate_limits/seven_day"),
    }
    .sanitized();
    (usage != AgentUsage::default()).then_some(usage)
}

fn pct_ok(p: f64) -> Option<f64> {
    (p.is_finite() && (0.0..=MAX_PCT).contains(&p)).then_some(p)
}

impl AgentUsage {
    /// Tira o que não é número plausível e corta o nome do modelo. Roda dos
    /// dois lados do socket: no shim, para não mandar lixo; no core, porque o
    /// agente fala com o socket direto.
    pub fn sanitized(self) -> AgentUsage {
        let model = self.model.and_then(|m| {
            let clean: String = m
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_MODEL_CHARS)
                .collect();
            let clean = clean.trim().to_string();
            (!clean.is_empty()).then_some(clean)
        });
        let window = |w: Option<RateWindow>| {
            w.and_then(|w| {
                Some(RateWindow {
                    pct: pct_ok(w.pct)?,
                    resets_at: w.resets_at,
                })
            })
        };
        AgentUsage {
            model,
            context_pct: self.context_pct.and_then(pct_ok),
            context_window_size: self.context_window_size,
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            cost_usd: self.cost_usd.filter(|c| c.is_finite() && *c >= 0.0),
            duration_ms: self.duration_ms,
            lines_added: self.lines_added,
            lines_removed: self.lines_removed,
            rate_5h: window(self.rate_5h),
            rate_7d: window(self.rate_7d),
        }
    }
}

/// A linha mínima que o TYBA imprime quando o usuário não tem `statusLine`
/// próprio: "Opus 5.5 · ctx 13%". Vazia quando não há nada a dizer.
pub fn minimal_line(usage: &AgentUsage) -> String {
    let mut parts = Vec::new();
    if let Some(model) = &usage.model {
        parts.push(model.clone());
    }
    if let Some(pct) = usage.context_pct {
        parts.push(format!("ctx {}%", pct.round() as i64));
    }
    parts.join(" · ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitWindow {
    FiveHour,
    SevenDay,
}

impl LimitWindow {
    fn label(self) -> &'static str {
        match self {
            LimitWindow::FiveHour => "5h",
            LimitWindow::SevenDay => "7 dias",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitWarning {
    pub window: LimitWindow,
    pub pct: f64,
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct WindowState {
    /// Já avisou nesta subida; só rearma quando o uso volta abaixo do limiar
    /// ou a janela que motivou o aviso reseta.
    fired: bool,
    fired_resets_at: Option<u64>,
}

/// Aviso de limite uma vez por cruzamento do limiar, não a cada mensagem.
///
/// Os limites são da conta, não da sessão: um tracker só para o app inteiro,
/// senão quatro sessões dariam quatro avisos do mesmo 87%.
#[derive(Debug, Default)]
pub struct LimitTracker {
    five_hour: WindowState,
    seven_day: WindowState,
}

impl LimitTracker {
    pub fn observe(&mut self, usage: &AgentUsage, now_secs: u64) -> Vec<LimitWarning> {
        let mut out = Vec::new();
        for (kind, window, state) in [
            (LimitWindow::FiveHour, usage.rate_5h, &mut self.five_hour),
            (LimitWindow::SevenDay, usage.rate_7d, &mut self.seven_day),
        ] {
            let Some(w) = window else { continue };
            if state.fired {
                let reset_passed = state.fired_resets_at.is_some_and(|r| now_secs >= r);
                if w.pct < USAGE_WARN_PCT || reset_passed {
                    *state = WindowState::default();
                }
            }
            if !state.fired && w.pct >= USAGE_WARN_PCT {
                state.fired = true;
                state.fired_resets_at = w.resets_at;
                out.push(LimitWarning {
                    window: kind,
                    pct: w.pct,
                    resets_at: w.resets_at,
                });
            }
        }
        out
    }
}

/// O tracker do processo. Global de propósito — ver [`LimitTracker`].
pub fn observe_limits(usage: &AgentUsage) -> Vec<LimitWarning> {
    static TRACKER: std::sync::Mutex<LimitTracker> = std::sync::Mutex::new(LimitTracker {
        five_hour: WindowState {
            fired: false,
            fired_resets_at: None,
        },
        seven_day: WindowState {
            fired: false,
            fired_resets_at: None,
        },
    });
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match TRACKER.lock() {
        Ok(mut t) => t.observe(usage, now),
        Err(poisoned) => poisoned.into_inner().observe(usage, now),
    }
}

/// "18:00" se reseta nas próximas 24h, "14/10 às 18:00" se depois — no fuso
/// local da máquina.
pub fn format_reset(resets_at: u64, now_secs: u64) -> Option<String> {
    use chrono::{Local, TimeZone};
    let at = Local
        .timestamp_opt(i64::try_from(resets_at).ok()?, 0)
        .single()?;
    if resets_at.saturating_sub(now_secs) < 24 * 3600 {
        Some(format!("às {}", at.format("%H:%M")))
    } else {
        Some(format!(
            "em {} às {}",
            at.format("%d/%m"),
            at.format("%H:%M")
        ))
    }
}

/// Corpo do aviso do sistema para o limite: "Limite de 5h em 87%, reseta às 18:00".
pub fn limit_warning_text(w: &LimitWarning, now_secs: u64) -> String {
    let pct = w.pct.round() as i64;
    let base = format!("Limite de {} em {pct}%", w.window.label());
    match w.resets_at.and_then(|r| format_reset(r, now_secs)) {
        Some(when) => format!("{base}, reseta {when}"),
        None => base,
    }
}

/// O motivo em pt-BR de um `StopFailure`, pelo campo `error` do payload
/// (lista conferida em code.claude.com/docs/en/hooks, out/2026). Valor que
/// não está na lista cai no genérico — o Claude pode ganhar tipos novos.
pub fn stop_failure_reason(error: Option<&str>) -> &'static str {
    match error {
        Some("rate_limit") => "limite de uso atingido",
        Some("overloaded") => "API sobrecarregada",
        Some("authentication_failed") => "falha de autenticação",
        Some("oauth_org_not_allowed") => "organização não permitida neste login",
        Some("account_on_hold") => "conta suspensa",
        Some("billing_error") => "problema de cobrança",
        Some("invalid_request") => "requisição inválida",
        Some("model_not_found") => "modelo não encontrado",
        Some("server_error") => "erro no servidor da API",
        Some("max_output_tokens") => "limite de saída atingido",
        Some("cloud_credential_error") => "falha na credencial de nuvem",
        _ => "erro da API",
    }
}

/// O que vai ao webview em [`EVENT_AGENT_ALERT`]. Só tipo, números e o motivo
/// fixo — nunca o `error_details` nem o `last_assistant_message`, que são
/// texto livre da API.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentAlert {
    StopFailure {
        session_id: crate::session::SessionId,
        error: String,
        reason: String,
    },
    UsageLimit {
        session_id: crate::session::SessionId,
        window: LimitWindow,
        pct: f64,
        resets_at: Option<u64>,
    },
}

/// O `error` do payload, só se for um identificador curto (`[a-z_]`): ele vai
/// ao webview, e texto livre não tem o que fazer lá.
pub fn stop_failure_error(raw: &Value) -> Option<String> {
    raw.get("error")
        .and_then(Value::as_str)
        .filter(|e| {
            !e.is_empty() && e.len() <= 40 && e.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Fixture sintética, na forma do exemplo da documentação.
    fn full() -> Value {
        json!({
            "cwd": "/tmp/proj",
            "session_id": "abc123",
            "transcript_path": "/tmp/t.jsonl",
            "model": { "id": "claude-opus-5-5", "display_name": "Opus 5.5" },
            "cost": {
                "total_cost_usd": 0.42,
                "total_duration_ms": 252000,
                "total_api_duration_ms": 2300,
                "total_lines_added": 156,
                "total_lines_removed": 23
            },
            "context_window": {
                "total_input_tokens": 45000,
                "total_output_tokens": 3000,
                "context_window_size": 200000,
                "used_percentage": 13,
                "remaining_percentage": 87,
                "current_usage": {
                    "input_tokens": 8500,
                    "output_tokens": 1200,
                    "cache_creation_input_tokens": 5000,
                    "cache_read_input_tokens": 2000
                }
            },
            "rate_limits": {
                "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600u64 },
                "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600u64 }
            }
        })
    }

    #[test]
    fn parse_completo() {
        let u = parse_statusline(full().to_string().as_bytes()).unwrap();
        assert_eq!(u.model.as_deref(), Some("Opus 5.5"));
        assert_eq!(u.context_pct, Some(13.0));
        assert_eq!(u.context_window_size, Some(200000));
        assert_eq!(u.tokens_in, Some(45000));
        assert_eq!(u.tokens_out, Some(3000));
        assert_eq!(u.cost_usd, Some(0.42));
        assert_eq!(u.duration_ms, Some(252000));
        assert_eq!(u.lines_added, Some(156));
        assert_eq!(u.lines_removed, Some(23));
        assert_eq!(
            u.rate_5h,
            Some(RateWindow {
                pct: 23.5,
                resets_at: Some(1738425600)
            })
        );
        assert_eq!(u.rate_7d.unwrap().pct, 41.2);
    }

    /// Só números e o nome do modelo atravessam: nada de caminho, cwd ou id.
    #[test]
    fn parse_nao_carrega_caminho_nem_id() {
        let u = parse_statusline(full().to_string().as_bytes()).unwrap();
        let out = serde_json::to_string(&u).unwrap();
        for leaked in ["/tmp", "abc123", "t.jsonl"] {
            assert!(!out.contains(leaked), "{leaked} vazou: {out}");
        }
    }

    #[test]
    fn parse_parcial_com_nulls_do_inicio_de_sessao() {
        let input = json!({
            "model": { "id": "claude-opus-5-5" },
            "context_window": { "used_percentage": null, "current_usage": null },
            "cost": { "total_cost_usd": 0 }
        });
        let u = parse_statusline(input.to_string().as_bytes()).unwrap();
        assert_eq!(u.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(u.context_pct, None);
        assert_eq!(u.cost_usd, Some(0.0));
        assert_eq!(u.rate_5h, None);
        assert_eq!(u.rate_7d, None);
    }

    #[test]
    fn parse_janela_sem_percentual_some() {
        let input =
            json!({ "rate_limits": { "five_hour": { "resets_at": 1 } }, "model": {"id": "x"} });
        let u = parse_statusline(input.to_string().as_bytes()).unwrap();
        assert_eq!(u.rate_5h, None);
    }

    #[test]
    fn parse_lixo() {
        assert_eq!(parse_statusline(b""), None);
        assert_eq!(parse_statusline(b"not json"), None);
        assert_eq!(parse_statusline(b"[1,2]"), None);
        assert_eq!(parse_statusline(b"{}"), None);
        assert_eq!(parse_statusline(br#"{"model": 5, "cost": "x"}"#), None);
    }

    #[test]
    fn parse_tipos_errados_viram_none() {
        let input = json!({
            "model": { "display_name": "Sonnet" },
            "context_window": { "used_percentage": "13", "total_input_tokens": -5 },
            "cost": { "total_cost_usd": -1.0 }
        });
        let u = parse_statusline(input.to_string().as_bytes()).unwrap();
        assert_eq!(u.context_pct, None);
        assert_eq!(u.tokens_in, None);
        assert_eq!(u.cost_usd, None);
    }

    #[test]
    fn sanitized_corta_modelo_e_percentual_absurdo() {
        let u = AgentUsage {
            model: Some(format!("a\u{1b}[31m{}", "x".repeat(200))),
            context_pct: Some(f64::NAN),
            rate_5h: Some(RateWindow {
                pct: 5000.0,
                resets_at: None,
            }),
            ..Default::default()
        }
        .sanitized();
        let model = u.model.unwrap();
        assert!(model.chars().count() <= MAX_MODEL_CHARS);
        assert!(!model.contains('\u{1b}'));
        assert_eq!(u.context_pct, None);
        assert_eq!(u.rate_5h, None);
    }

    #[test]
    fn linha_minima() {
        let u = parse_statusline(full().to_string().as_bytes()).unwrap();
        assert_eq!(minimal_line(&u), "Opus 5.5 · ctx 13%");
        assert_eq!(minimal_line(&AgentUsage::default()), "");
        let only_model = AgentUsage {
            model: Some("Opus".into()),
            ..Default::default()
        };
        assert_eq!(minimal_line(&only_model), "Opus");
    }

    fn with_5h(pct: f64, resets_at: u64) -> AgentUsage {
        AgentUsage {
            rate_5h: Some(RateWindow {
                pct,
                resets_at: Some(resets_at),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn limite_avisa_uma_vez_por_cruzamento() {
        let mut t = LimitTracker::default();
        assert!(t.observe(&with_5h(80.0, 1000), 0).is_empty());
        let w = t.observe(&with_5h(87.0, 1000), 0);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].window, LimitWindow::FiveHour);
        assert_eq!(w[0].pct, 87.0);
        // Mensagens seguintes acima do limiar: silêncio.
        assert!(t.observe(&with_5h(88.0, 1000), 10).is_empty());
        assert!(t.observe(&with_5h(99.0, 1000), 20).is_empty());
    }

    #[test]
    fn limite_rearma_quando_desce_ou_a_janela_reseta() {
        let mut t = LimitTracker::default();
        assert_eq!(t.observe(&with_5h(90.0, 1000), 0).len(), 1);
        // Janela resetou e o uso caiu: rearma.
        assert!(t.observe(&with_5h(5.0, 19000), 1001).is_empty());
        assert_eq!(t.observe(&with_5h(86.0, 19000), 1500).len(), 1);
        // A janela que motivou o aviso passou, mesmo sem ver o uso baixo.
        assert_eq!(t.observe(&with_5h(91.0, 40000), 19001).len(), 1);
    }

    #[test]
    fn limite_primeira_leitura_ja_acima_avisa() {
        let mut t = LimitTracker::default();
        assert_eq!(t.observe(&with_5h(85.0, 1000), 0).len(), 1);
    }

    #[test]
    fn limite_janelas_sao_independentes() {
        let mut t = LimitTracker::default();
        let u = AgentUsage {
            rate_5h: Some(RateWindow {
                pct: 86.0,
                resets_at: Some(10),
            }),
            rate_7d: Some(RateWindow {
                pct: 95.0,
                resets_at: Some(10_000),
            }),
            ..Default::default()
        };
        let w = t.observe(&u, 0);
        assert_eq!(w.len(), 2);
        assert!(t.observe(&u, 1).is_empty());
        // Sem `rate_limits` (sessão de API key): nada muda.
        assert!(t.observe(&AgentUsage::default(), 2).is_empty());
    }

    #[test]
    fn texto_do_aviso_de_limite() {
        let w = LimitWarning {
            window: LimitWindow::FiveHour,
            pct: 86.6,
            resets_at: None,
        };
        assert_eq!(limit_warning_text(&w, 0), "Limite de 5h em 87%");
        let w = LimitWarning {
            window: LimitWindow::SevenDay,
            pct: 90.0,
            resets_at: Some(3600),
        };
        let text = limit_warning_text(&w, 0);
        assert!(
            text.starts_with("Limite de 7 dias em 90%, reseta às "),
            "{text}"
        );
        let far = limit_warning_text(
            &LimitWarning {
                resets_at: Some(5 * 24 * 3600),
                ..w
            },
            0,
        );
        assert!(far.contains(", reseta em "), "{far}");
    }

    #[test]
    fn stop_failure_mapeia_os_tipos_da_documentacao() {
        assert_eq!(
            stop_failure_reason(Some("rate_limit")),
            "limite de uso atingido"
        );
        assert_eq!(
            stop_failure_reason(Some("authentication_failed")),
            "falha de autenticação"
        );
        assert_eq!(
            stop_failure_reason(Some("billing_error")),
            "problema de cobrança"
        );
        assert_eq!(
            stop_failure_reason(Some("overloaded")),
            "API sobrecarregada"
        );
        assert_eq!(stop_failure_reason(Some("unknown")), "erro da API");
        assert_eq!(stop_failure_reason(Some("tipo_novo")), "erro da API");
        assert_eq!(stop_failure_reason(None), "erro da API");
    }

    #[test]
    fn stop_failure_error_so_aceita_identificador() {
        assert_eq!(
            stop_failure_error(&json!({ "error": "rate_limit" })).as_deref(),
            Some("rate_limit")
        );
        assert_eq!(
            stop_failure_error(&json!({ "error": "429 Too Many" })),
            None
        );
        assert_eq!(stop_failure_error(&json!({ "error": 5 })), None);
        assert_eq!(stop_failure_error(&json!({})), None);
    }
}
