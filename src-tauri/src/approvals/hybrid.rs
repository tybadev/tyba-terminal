//! Decisão do fluxo híbrido de aprovação: quem responde ao `PreToolUse`.
//!
//! Cada nível de risco tem um dono só (R1 do fluxo híbrido):
//!
//! | runner      | risco              | regra ampla? | decisão |
//! |-------------|--------------------|--------------|---------|
//! | Claude Code | verde              | —            | `Allow` |
//! | Claude Code | amarelo            | não          | `Pass` — o hook sai sem opinar, vale o prompt nativo do Claude |
//! | Claude Code | amarelo            | sim          | `Ask` — força a pergunta que a config do Claude desligou |
//! | Claude Code | vermelho           | —            | `Gate` — o TYBA decide |
//! | Codex/outro | verde              | —            | `Allow` |
//! | Codex/outro | amarelo/vermelho   | —            | `Gate` (fluxo atual) |
//! | qualquer    | push p/ main/master| —            | `Gate`, onde o core recusa sem virar pedido |
//!
//! `Pass` nunca responde vermelho nem Codex, e `Ask` só existe com regra
//! ampla: um `ask` de hook atropela o allow do usuário no Claude (deny → ask →
//! allow), e sem regra ampla isso quebraria o "don't ask again" — a queixa que
//! originou o fluxo.
//!
//! Push para main/master é vermelho pela classificação (todo `git push` é), e
//! [`effective_risk`] garante isso mesmo que a classificação e a recusa do
//! core venham a divergir: o que o core recusa nunca sai do gate, onde
//! `ApprovalsManager` o nega sem pedido (princípio nº 5).

use std::path::Path;

use serde_json::Value;

use super::tool_action::{normalize_tool_use, NormalizedToolUse};
use super::{is_refused_by_core, RedReason, RiskLevel};
use crate::session::AgentRunnerKind;

/// Motivo que o Claude mostra no prompt forçado pelo `ask` (mensagem de UI,
/// pt-BR). O prompt forçado não tem "don't ask again" (medido no B0), então o
/// texto precisa dizer por que o TYBA está perguntando.
pub const BROAD_SETTINGS_ASK_REASON: &str =
    "A configuração do Claude libera tudo sem perguntar; por isso o TYBA pergunta.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HybridDecision {
    /// Verde: o TYBA libera sozinho.
    Allow,
    /// Amarelo do Claude: o hook sai sem decisão e valem as regras do Claude.
    Pass,
    /// Amarelo do Claude com regra ampla: o hook força a pergunta nativa.
    Ask(String),
    /// Gate do TYBA: bloqueia até decisão humana (ou recusa do core).
    Gate,
}

/// A tabela do módulo, inteira. `broad` só é consultado no amarelo do Claude.
pub fn hybrid_decision(risk: RiskLevel, runner: &AgentRunnerKind, broad: bool) -> HybridDecision {
    match (risk, runner) {
        (RiskLevel::Green, _) => HybridDecision::Allow,
        (RiskLevel::Yellow, AgentRunnerKind::ClaudeCode) if broad => {
            HybridDecision::Ask(BROAD_SETTINGS_ASK_REASON.to_string())
        }
        (RiskLevel::Yellow, AgentRunnerKind::ClaudeCode) => HybridDecision::Pass,
        (RiskLevel::Yellow, _) | (RiskLevel::Red, _) => HybridDecision::Gate,
    }
}

/// O que o core recusa (push para main/master) é vermelho, qualquer que seja
/// a classificação: vermelho nunca sai do gate, e é o gate que recusa.
pub fn effective_risk(risk: RiskLevel, refused_by_core: bool) -> RiskLevel {
    if refused_by_core {
        RiskLevel::Red
    } else {
        risk
    }
}

#[derive(Debug)]
pub struct PreToolUsePlan {
    pub normalized: NormalizedToolUse,
    pub risk: RiskLevel,
    /// Por que é vermelho (`None` fora do vermelho).
    pub reason: Option<RedReason>,
    /// Push para main/master, decidido sobre o texto INTEIRO da chamada — o
    /// gate recusa sem virar pedido, mesmo que o texto exibido tenha sido
    /// cortado antes do push.
    pub refused_by_core: bool,
    pub decision: HybridDecision,
}

/// Normaliza, classifica e decide um `PreToolUse`. `broad` é preguiçoso: só
/// roda (e só lê as configurações do Claude do disco) no amarelo do Claude.
pub fn plan_pre_tool_use(
    runner: &AgentRunnerKind,
    tool_name: &str,
    tool_input: Option<&Value>,
    worktree_root: &Path,
    broad: impl FnOnce() -> bool,
) -> PreToolUsePlan {
    let normalized = normalize_tool_use(runner, tool_name, tool_input);
    let (classified, reason) = normalized.action.classify(worktree_root);
    // `description` é o texto inteiro, sem corte: um push para main depois do
    // caractere 500 (ou de 64 KiB) não escapa da recusa.
    let refused_by_core = is_refused_by_core(&normalized.description);
    let risk = effective_risk(classified, refused_by_core);
    let reason = match risk {
        RiskLevel::Red => reason.or(Some(RedReason::GitPush)),
        _ => None,
    };
    let broad =
        risk == RiskLevel::Yellow && matches!(runner, AgentRunnerKind::ClaudeCode) && broad();
    let decision = hybrid_decision(risk, runner, broad);
    PreToolUsePlan {
        normalized,
        risk,
        reason,
        refused_by_core,
        decision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    const RISKS: [RiskLevel; 3] = [RiskLevel::Green, RiskLevel::Yellow, RiskLevel::Red];

    fn runners() -> [AgentRunnerKind; 3] {
        [
            AgentRunnerKind::ClaudeCode,
            AgentRunnerKind::Codex,
            AgentRunnerKind::Custom("outro".into()),
        ]
    }

    fn esperado(risk: RiskLevel, runner: &AgentRunnerKind, broad: bool) -> HybridDecision {
        let claude = matches!(runner, AgentRunnerKind::ClaudeCode);
        match risk {
            RiskLevel::Green => HybridDecision::Allow,
            RiskLevel::Yellow if claude && broad => {
                HybridDecision::Ask(BROAD_SETTINGS_ASK_REASON.to_string())
            }
            RiskLevel::Yellow if claude => HybridDecision::Pass,
            RiskLevel::Yellow | RiskLevel::Red => HybridDecision::Gate,
        }
    }

    /// CA-01, CA-02, CA-04, CA-06, CA-17: a tabela inteira, risco × runner ×
    /// regra ampla, contra a tabela escrita à mão acima.
    #[test]
    fn tabela_completa_de_risco_runner_e_regra_ampla() {
        let mut casos = 0;
        for risk in RISKS {
            for runner in runners() {
                for broad in [false, true] {
                    assert_eq!(
                        hybrid_decision(risk, &runner, broad),
                        esperado(risk, &runner, broad),
                        "{risk:?} × {runner:?} × amplo={broad}"
                    );
                    casos += 1;
                }
            }
        }
        assert_eq!(casos, 18);
    }

    /// Invariantes 1 e 2: vermelho e push para main nunca viram Pass/Ask, em
    /// nenhuma combinação — nem com a classificação dizendo verde ou amarelo.
    #[test]
    fn vermelho_e_push_para_main_nunca_viram_pass_nem_ask() {
        for risk in RISKS {
            for refused in [false, true] {
                for runner in runners() {
                    for broad in [false, true] {
                        let efetivo = effective_risk(risk, refused);
                        let decisao = hybrid_decision(efetivo, &runner, broad);
                        if efetivo == RiskLevel::Red || refused {
                            assert_eq!(
                                decisao,
                                HybridDecision::Gate,
                                "{risk:?} recusado={refused} × {runner:?} × amplo={broad}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn pass_so_existe_no_amarelo_do_claude_sem_regra_ampla() {
        for risk in RISKS {
            for runner in runners() {
                for broad in [false, true] {
                    if hybrid_decision(risk, &runner, broad) == HybridDecision::Pass {
                        assert_eq!(risk, RiskLevel::Yellow);
                        assert_eq!(runner, AgentRunnerKind::ClaudeCode);
                        assert!(!broad);
                    }
                }
            }
        }
    }

    /// Invariante 3: o hook nunca responde `ask` sem regra ampla.
    #[test]
    fn ask_so_existe_com_regra_ampla() {
        for risk in RISKS {
            for runner in runners() {
                assert!(!matches!(
                    hybrid_decision(risk, &runner, false),
                    HybridDecision::Ask(_)
                ));
            }
        }
    }

    #[test]
    fn motivo_do_ask_diz_que_a_config_do_claude_libera_tudo() {
        assert!(BROAD_SETTINGS_ASK_REASON.contains("configuração do Claude"));
        assert!(BROAD_SETTINGS_ASK_REASON.contains("TYBA"));
    }

    fn plano(runner: &AgentRunnerKind, tool: &str, input: Value, broad: bool) -> PreToolUsePlan {
        let root = Path::new("/wt");
        plan_pre_tool_use(runner, tool, Some(&input), root, || broad)
    }

    #[test]
    fn plano_do_claude_segue_a_tabela_pela_classificacao_real() {
        let claude = AgentRunnerKind::ClaudeCode;
        let bash = |c: &str| json!({ "command": c });

        assert_eq!(
            plano(&claude, "Bash", bash("ls"), true).decision,
            HybridDecision::Allow
        );
        assert_eq!(
            plano(&claude, "Bash", bash("cargo test"), false).decision,
            HybridDecision::Pass
        );
        assert!(matches!(
            plano(&claude, "Bash", bash("cargo test"), true).decision,
            HybridDecision::Ask(_)
        ));
        assert_eq!(
            plano(&claude, "Bash", bash("curl https://x.dev"), false).decision,
            HybridDecision::Gate
        );
        assert_eq!(
            plano(&claude, "Bash", bash("curl https://x.dev"), true).decision,
            HybridDecision::Gate
        );
    }

    /// CA-05: push para main é vermelho e vai ao gate — onde o core recusa.
    #[test]
    fn push_para_main_vai_ao_gate_em_todo_runner() {
        // `Custom` nem lê o comando (vira `Unknown`, amarelo → gate); os
        // runners que normalizam Bash são os que importam aqui.
        for runner in [AgentRunnerKind::ClaudeCode, AgentRunnerKind::Codex] {
            for broad in [false, true] {
                let p = plano(
                    &runner,
                    "Bash",
                    json!({ "command": "git push origin main" }),
                    broad,
                );
                assert_eq!(p.risk, RiskLevel::Red);
                assert_eq!(p.decision, HybridDecision::Gate);
                assert!(is_refused_by_core(&p.normalized.description));
            }
        }
    }

    /// Codex continua no fluxo atual: amarelo vai ao gate (CA-17).
    #[test]
    fn amarelo_do_codex_vai_ao_gate() {
        let p = plano(
            &AgentRunnerKind::Codex,
            "Bash",
            json!({ "command": "cargo test" }),
            false,
        );
        assert_eq!(p.risk, RiskLevel::Yellow);
        assert_eq!(p.decision, HybridDecision::Gate);
    }

    /// A leitura de config só acontece onde importa: amarelo do Claude.
    #[test]
    fn regra_ampla_so_e_lida_no_amarelo_do_claude() {
        let casos = [
            (AgentRunnerKind::ClaudeCode, "ls", false),
            (AgentRunnerKind::ClaudeCode, "cargo test", true),
            (AgentRunnerKind::ClaudeCode, "curl https://x.dev", false),
            (AgentRunnerKind::Codex, "cargo test", false),
        ];
        for (runner, command, deve_ler) in casos {
            let lido = Cell::new(false);
            let _ = plan_pre_tool_use(
                &runner,
                "Bash",
                Some(&json!({ "command": command })),
                Path::new("/wt"),
                || {
                    lido.set(true);
                    true
                },
            );
            assert_eq!(lido.get(), deve_ler, "{runner:?} {command}");
        }
    }

    /// Bug de segurança corrigido no Bloco 3: a recusa lia a descrição
    /// cortada em 500 caracteres, e um push para main depois do corte virava
    /// pedido vermelho APROVÁVEL em vez de recusa. Agora lê o texto inteiro —
    /// também depois do limite de 64 KiB do texto exibido.
    #[test]
    fn push_para_main_depois_do_corte_continua_recusado() {
        for pad in [600, 70 * 1024] {
            let command = format!("echo {} ; git push origin main", "a".repeat(pad));
            for runner in [AgentRunnerKind::ClaudeCode, AgentRunnerKind::Codex] {
                let p = plano(
                    &runner,
                    "Bash",
                    json!({ "command": command.clone() }),
                    false,
                );
                assert!(p.refused_by_core, "{runner:?} pad={pad}");
                assert_eq!(p.risk, RiskLevel::Red);
                assert_eq!(p.reason, Some(RedReason::GitPush));
                assert_eq!(p.decision, HybridDecision::Gate);
                // O texto cortado de antes não mostrava o push: era por ele
                // que a recusa escapava.
                assert!(!is_refused_by_core(&p.normalized.allowlist_key()));
            }
        }
        let p = plano(
            &AgentRunnerKind::ClaudeCode,
            "Bash",
            json!({ "command": format!("echo {} ; git push origin main", "a".repeat(70 * 1024)) }),
            false,
        );
        assert!(!is_refused_by_core(&p.normalized.display_text()));
    }

    #[test]
    fn motivo_so_no_vermelho_e_push_recusado_leva_git_push() {
        let claude = AgentRunnerKind::ClaudeCode;
        let bash = |c: &str| json!({ "command": c });
        assert_eq!(plano(&claude, "Bash", bash("ls"), false).reason, None);
        assert_eq!(
            plano(&claude, "Bash", bash("cargo test"), false).reason,
            None
        );
        assert_eq!(
            plano(&claude, "Bash", bash("git push origin main"), false).reason,
            Some(RedReason::GitPush)
        );
        assert_eq!(
            plano(&claude, "WebFetch", json!({ "url": "https://x" }), false).reason,
            Some(RedReason::WebAccess)
        );
    }

    /// Defesa 1 de security: escrever a config do Claude nunca vira `Pass` —
    /// vai ao gate do TYBA, com ou sem regra ampla.
    #[test]
    fn escrita_na_config_do_claude_vai_ao_gate() {
        for broad in [false, true] {
            for (tool, input) in [
                (
                    "Write",
                    json!({ "file_path": ".claude/settings.local.json" }),
                ),
                (
                    "Bash",
                    json!({ "command": "echo x > .claude/settings.local.json" }),
                ),
            ] {
                let p = plano(&AgentRunnerKind::ClaudeCode, tool, input, broad);
                assert_eq!(p.risk, RiskLevel::Red, "{tool}");
                assert_eq!(p.reason, Some(RedReason::ClaudeConfigWrite), "{tool}");
                assert_eq!(p.decision, HybridDecision::Gate, "{tool}");
            }
        }
    }
}
