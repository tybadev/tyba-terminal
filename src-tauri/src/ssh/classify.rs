//! Por que um Cano morreu antes do login. Puro: lê a saída que o `ssh` deixou no
//! pane antes do marco de login e devolve um motivo que a UI traduz.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    AuthRefused,
    HostKeyChanged,
    HostKeyRejected,
    HostUnresolved,
    NoRoute,
    Unknown,
}

impl FailureReason {
    /// Dentro de uma queda, só rede ausente justifica insistir: senha, chave e
    /// digital não mudam sozinhas entre uma tentativa e outra.
    pub fn retryable_in_drop(self) -> bool {
        matches!(self, FailureReason::NoRoute | FailureReason::HostUnresolved)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanoFailure {
    pub reason: FailureReason,
    pub detail: String,
}

const DETAIL_MAX_CHARS: usize = 300;

/// Ordem é precedência: a primeira linha da tabela que casar vence.
const PATTERNS: &[(FailureReason, &[&str])] = &[
    (
        FailureReason::AuthRefused,
        &["Permission denied (", "Too many authentication failures"],
    ),
    (
        FailureReason::HostKeyChanged,
        &["REMOTE HOST IDENTIFICATION HAS CHANGED"],
    ),
    (
        FailureReason::HostKeyRejected,
        &["Host key verification failed"],
    ),
    (
        FailureReason::HostUnresolved,
        &["Could not resolve hostname"],
    ),
    (
        FailureReason::NoRoute,
        &[
            "Connection timed out",
            "Operation timed out",
            "Connection refused",
            "No route to host",
            "Network is unreachable",
        ],
    ),
];

pub fn classify(output: &str) -> CanoFailure {
    let output = strip_escapes(output);
    let output = output.as_str();
    let reason = PATTERNS
        .iter()
        .find(|(_, needles)| needles.iter().any(|n| output.contains(n)))
        .map_or(FailureReason::Unknown, |(reason, _)| *reason);
    CanoFailure {
        reason,
        detail: last_line(output),
    }
}

/// Tira CSI, OSC e escapes de dois bytes. Uma cor no meio de
/// `Permission denied` bastaria para o motivo virar `unknown`.
pub(crate) fn strip_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn last_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .chars()
        .take(DETAIL_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn senha_ou_chave_recusada_e_auth_refused() {
        let out = "deploy@web.example.test: Permission denied (publickey,password).\r\n";
        assert_eq!(classify(out).reason, FailureReason::AuthRefused);
    }

    #[test]
    fn agente_com_chaves_demais_tambem_e_auth_refused() {
        let out =
            "Received disconnect from 192.0.2.10 port 22:2: Too many authentication failures\r\n\
                   Disconnected from 192.0.2.10 port 22\r\n";
        assert_eq!(classify(out).reason, FailureReason::AuthRefused);
    }

    #[test]
    fn digital_trocada_vence_a_linha_de_verificacao_que_vem_junto() {
        let out = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
                   @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\r\n\
                   @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
                   Host key for web.example.test has changed and you have requested strict checking.\r\n\
                   Host key verification failed.\r\n";
        assert_eq!(classify(out).reason, FailureReason::HostKeyChanged);
    }

    #[test]
    fn digital_recusada_no_prompt_e_host_key_rejected() {
        let out =
            "The authenticity of host 'web.example.test (192.0.2.10)' can't be established.\r\n\
                   ED25519 key fingerprint is SHA256:AAAAtestfingerprintAAAA.\r\n\
                   Are you sure you want to continue connecting (yes/no/[fingerprint])? no\r\n\
                   Host key verification failed.\r\n";
        assert_eq!(classify(out).reason, FailureReason::HostKeyRejected);
    }

    #[test]
    fn nome_que_nao_resolve_e_host_unresolved() {
        let out = "ssh: Could not resolve hostname web.example.invalid: nodename nor servname provided, or not known\r\n";
        assert_eq!(classify(out).reason, FailureReason::HostUnresolved);
    }

    #[test]
    fn rede_que_nao_chega_e_no_route() {
        for out in [
            "ssh: connect to host 192.0.2.10 port 22: Connection timed out\r\n",
            "ssh: connect to host 192.0.2.10 port 22: Operation timed out\r\n",
            "ssh: connect to host 192.0.2.10 port 2222: Connection refused\r\n",
            "ssh: connect to host 192.0.2.10 port 22: No route to host\r\n",
            "ssh: connect to host 192.0.2.10 port 22: Network is unreachable\r\n",
        ] {
            assert_eq!(classify(out).reason, FailureReason::NoRoute, "{out}");
        }
    }

    #[test]
    fn recusa_de_auth_vence_rede_quando_as_duas_aparecem() {
        let out = "ssh: connect to host bastion.example.test port 22: Connection refused\r\n\
                   deploy@web.example.test: Permission denied (publickey).\r\n";
        assert_eq!(classify(out).reason, FailureReason::AuthRefused);
    }

    #[test]
    fn detalhe_e_a_ultima_linha_nao_vazia() {
        let out = "Warning: Permanently added 'web.example.test' to the list of known hosts.\r\n\
                   deploy@web.example.test: Permission denied (publickey).\r\n\r\n  \r\n";
        assert_eq!(
            classify(out).detail,
            "deploy@web.example.test: Permission denied (publickey)."
        );
    }

    #[test]
    fn detalhe_longo_e_cortado_em_300_caracteres() {
        let line = "é".repeat(400);
        let detail = classify(&line).detail;
        assert_eq!(detail.chars().count(), 300);
        assert!(line.starts_with(&detail));
    }

    #[test]
    fn escapes_de_terminal_nao_escondem_o_motivo_nem_sujam_o_detalhe() {
        let out = "\x1b]0;ssh\x07\x1b[1;31mdeploy@web.example.test: Permission\x1b[0m denied (publickey).\x1b[K\r\n";
        let failure = classify(out);
        assert_eq!(failure.reason, FailureReason::AuthRefused);
        assert_eq!(
            failure.detail,
            "deploy@web.example.test: Permission denied (publickey)."
        );
    }

    #[test]
    fn saida_sem_padrao_conhecido_e_unknown_com_detalhe() {
        let out = "kex_exchange_identification: read: Connection reset by peer\r\n";
        let failure = classify(out);
        assert_eq!(failure.reason, FailureReason::Unknown);
        assert_eq!(
            failure.detail,
            "kex_exchange_identification: read: Connection reset by peer"
        );
        assert_eq!(classify("").detail, "");
    }

    #[test]
    fn motivo_serializa_em_snake_case() {
        let json = serde_json::to_string(&classify("Could not resolve hostname x")).unwrap();
        assert!(json.contains("\"reason\":\"host_unresolved\""), "{json}");
    }
}
