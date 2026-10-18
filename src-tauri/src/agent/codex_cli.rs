//! O `codex` digitado no shell de um nó Terminal do Orbit (fatia 4): a
//! função `codex` do rc só existe nesse shell e chama `tyba _jail-codex --
//! "$@"`. Esta é a decisão — pura, sobre o argv — de quais chamadas o core
//! hospeda com as ferramentas do Orbit. O cliente e o core fazem a mesma
//! conta, cada um do seu lado; o core nunca confia na do cliente.
//!
//! Diferente do `claude`, nada aqui é recusado: o `codex` não tem hooks do
//! TYBA para desligar. O que não é sessão (versão, ajuda, conta, config) roda
//! o `codex` de verdade, intacto.

/// Subcomandos que não abrem sessão de agente: conta, config, manutenção.
/// Conferido contra `codex --help` do codex-cli 0.144.1. Subcomando novo que
/// não esteja aqui só ganha as `--config` do Orbit (inofensivas fora de
/// sessão), nunca deixa de rodar.
const PASSTHROUGH_SUBCOMMANDS: &[&str] = &[
    "login",
    "logout",
    "mcp",
    "plugin",
    "mcp-server",
    "app-server",
    "remote-control",
    "app",
    "completion",
    "update",
    "doctor",
    "sandbox",
    "debug",
    "apply",
    "a",
    "archive",
    "delete",
    "unarchive",
    "cloud",
    "exec-server",
    "features",
    "help",
];

/// Flags que só imprimem e saem.
const PASSTHROUGH_FLAGS: &[&str] = &["-h", "--help", "-V", "--version"];

/// Sessão de agente (`codex`, `codex "<prompt>"`, flags, `resume`, `fork`,
/// `exec`, `review`): o core hospeda com as ferramentas do Orbit. O resto
/// roda o `codex` de verdade, intacto.
pub fn hosts(args: &[String]) -> bool {
    if args.iter().any(|a| PASSTHROUGH_FLAGS.contains(&a.as_str())) {
        return false;
    }
    !args
        .first()
        .is_some_and(|f| PASSTHROUGH_SUBCOMMANDS.contains(&f.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_session_form_is_hosted() {
        for form in [
            &[][..],
            &["conserte o teste"],
            &["-m", "gpt-5.5-codex"],
            &["--sandbox", "workspace-write", "faça o plano"],
            &["resume", "--last"],
            &["fork", "--last"],
            &["exec", "rode os testes"],
            &["e", "rode os testes"],
            &["review"],
        ] {
            assert!(hosts(&args(form)), "{form:?}");
        }
    }

    #[test]
    fn version_help_and_config_run_the_real_codex() {
        for form in [
            &["--version"][..],
            &["-V"],
            &["--help"],
            &["resume", "-h"],
            &["login"],
            &["logout"],
            &["mcp", "list"],
            &["features", "list"],
            &["help"],
        ] {
            assert!(!hosts(&args(form)), "{form:?}");
        }
    }
}
