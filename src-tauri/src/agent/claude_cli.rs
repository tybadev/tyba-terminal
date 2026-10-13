//! O que fazer com cada forma de chamar `claude` dentro de um pane do TYBA
//! (M6-A do claude-monitor): toda chamada passa por `tyba _jail -- "$@"`, e
//! esta é a decisão — pura, sobre o argv — que o cliente (`_jail`) e o core
//! (`channel_host`) tomam, cada um do seu lado. O core nunca confia na
//! decisão do cliente: refaz a mesma conta sobre os args que chegaram.
//!
//! Os argumentos viajam sempre como argv (um elemento por argumento), nunca
//! por uma string de shell: nada aqui é interpolado.

use std::path::{Path, PathBuf};

/// Subcomandos que não abrem sessão de agente local: configuração, conta,
/// manutenção. Rodam crus e intactos. Conferido contra `claude --help` da
/// 2.1.296; `config` fica pela compatibilidade com versões anteriores.
const PASSTHROUGH_SUBCOMMANDS: &[&str] = &[
    "auth",
    "auto-mode",
    "config",
    "doctor",
    "gateway",
    "import",
    "install",
    "logs",
    "mcp",
    "plugin",
    "plugins",
    "purge",
    "rm",
    "setup-token",
    "stop",
    "kill",
    "ultrareview",
    "update",
    "upgrade",
];

/// Flags que só imprimem e saem.
const PASSTHROUGH_FLAGS: &[&str] = &["-v", "--version", "-h", "--help"];

/// Sessões em segundo plano: o agente roda num processo do próprio Claude,
/// fora do pane, onde o TYBA não hospeda nem jaula. Rodam cruas, marcadas
/// como sem gate (o pane mostra o estado vermelho), nunca escondidas.
const BACKGROUND_SUBCOMMANDS: &[&str] = &["agents", "attach", "respawn"];
const BACKGROUND_FLAGS: &[&str] = &["--bg", "--background"];

/// Flags que desligam ou contornam o gate do TYBA: pular hooks, trocar as
/// settings que carregam os hooks, pular a checagem de permissão. Dentro do
/// TYBA não rodam — nem hospedadas, nem cruas.
const FORBIDDEN_FLAGS: &[&str] = &[
    "--bare",
    "--safe-mode",
    "--settings",
    "--setting-sources",
    "--dangerously-skip-permissions",
    "--allow-dangerously-skip-permissions",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeRoute {
    /// Sessão de agente: o core hospeda com gate e jaula, args repassados.
    Host,
    /// Não abre sessão (versão, ajuda, `mcp`, `doctor`...): roda cru.
    Passthrough,
    /// Abre sessão fora do pane (segundo plano): roda cru, marcado sem gate.
    Ungated,
    /// Desligaria o gate: recusado, com o motivo para o usuário.
    Forbidden(String),
}

/// Classifica a chamada pelo argv (sem o `claude`).
pub fn route(args: &[String]) -> ClaudeRoute {
    if let Some(reason) = forbidden(args) {
        return ClaudeRoute::Forbidden(reason);
    }
    if args.iter().any(|a| PASSTHROUGH_FLAGS.contains(&a.as_str())) {
        return ClaudeRoute::Passthrough;
    }
    let first = args.first().map(String::as_str);
    if first.is_some_and(|f| PASSTHROUGH_SUBCOMMANDS.contains(&f)) {
        return ClaudeRoute::Passthrough;
    }
    if first.is_some_and(|f| BACKGROUND_SUBCOMMANDS.contains(&f))
        || args.iter().any(|a| BACKGROUND_FLAGS.contains(&a.as_str()))
    {
        return ClaudeRoute::Ungated;
    }
    ClaudeRoute::Host
}

fn flag_name(arg: &str) -> &str {
    arg.split_once('=').map_or(arg, |(name, _)| name)
}

fn forbidden(args: &[String]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        let name = flag_name(arg);
        if FORBIDDEN_FLAGS.contains(&name) {
            return Some(format!("`{name}` desliga o gate do TYBA"));
        }
        if name == "--permission-mode" {
            let mode = match arg.split_once('=') {
                Some((_, v)) => Some(v),
                None => args.get(i + 1).map(String::as_str),
            };
            if mode == Some("bypassPermissions") {
                return Some("`--permission-mode bypassPermissions` desliga o gate do TYBA".into());
            }
        }
        if name == "-r" || name == "--resume" {
            let value = match arg.split_once('=') {
                Some((_, v)) => Some(v),
                None => args
                    .get(i + 1)
                    .map(String::as_str)
                    .filter(|v| !v.starts_with('-')),
            };
            if let Some(id) = value {
                if !crate::agent::conversation::is_plausible(id) {
                    return Some(format!("`{name}` espera um id de conversa válido"));
                }
            }
        }
    }
    None
}

/// O `claude` de verdade, nunca o wrapper do TYBA: primeiro o que o core
/// resolveu ao abrir o pane (`TYBA_REAL_CLAUDE`), depois o `PATH` pulando o
/// diretório do wrapper. Recusa qualquer candidato dentro do diretório do
/// wrapper — é isso que impede `_jail` → wrapper → `_jail` em loop.
pub fn real_claude(
    resolved: Option<&str>,
    path: Option<&str>,
    wrapper_dir: Option<&Path>,
    is_executable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let wrapper = wrapper_dir.map(crate::repo::canonicalize_or);
    let not_wrapper = |p: &Path| {
        let dir = p.parent().map(crate::repo::canonicalize_or);
        wrapper.is_none() || dir != wrapper
    };
    if let Some(p) = resolved.map(PathBuf::from) {
        if p.is_absolute() && not_wrapper(&p) && is_executable(&p) {
            return Some(p);
        }
    }
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("claude"))
        .find(|p| not_wrapper(p) && is_executable(p))
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
            &["-c"],
            &["--continue"],
            &["--resume", "11111111-2222-3333-4444-555555555555"],
            &["-r"],
            &["--resume"],
            &["-p", "explica o diff"],
            &["--print", "--output-format", "json", "oi"],
            &["corrige o teste"],
            &["--model", "opus", "-n", "tarefa"],
            &["--permission-mode", "plan"],
            &["-w", "minha-arvore"],
        ] {
            assert_eq!(route(&args(form)), ClaudeRoute::Host, "{form:?}");
        }
    }

    #[test]
    fn non_session_commands_pass_through_untouched() {
        for form in [
            &["--version"][..],
            &["-v"],
            &["-h"],
            &["--help"],
            &["mcp", "list"],
            &["plugin", "install", "x"],
            &["doctor"],
            &["update"],
            &["install", "stable"],
            &["setup-token"],
            &["config", "get", "theme"],
        ] {
            assert_eq!(route(&args(form)), ClaudeRoute::Passthrough, "{form:?}");
        }
    }

    #[test]
    fn a_subcommand_name_inside_a_prompt_is_still_a_session() {
        // Só o PRIMEIRO argumento é subcomando; "mcp" no meio do prompt não.
        assert_eq!(route(&args(&["-p", "mcp"])), ClaudeRoute::Host);
        assert_eq!(route(&args(&["explica o --help"])), ClaudeRoute::Host);
    }

    #[test]
    fn background_sessions_run_raw_but_marked_ungated() {
        for form in [&["--bg", "tarefa"][..], &["attach", "x1"], &["agents"]] {
            assert_eq!(route(&args(form)), ClaudeRoute::Ungated, "{form:?}");
        }
    }

    #[test]
    fn gate_disabling_flags_are_forbidden_in_any_form() {
        for form in [
            &["--bare"][..],
            &["--safe-mode"],
            &["--settings", "/tmp/x.json"],
            &["--settings={\"disableAllHooks\":true}"],
            &["--setting-sources", "user"],
            &["--dangerously-skip-permissions"],
            &["-p", "oi", "--allow-dangerously-skip-permissions"],
            &["--permission-mode", "bypassPermissions"],
            &["--permission-mode=bypassPermissions"],
            // Forbidden vence passthrough: `--help` não serve de escotilha.
            &["--dangerously-skip-permissions", "--help"],
        ] {
            assert!(
                matches!(route(&args(form)), ClaudeRoute::Forbidden(_)),
                "{form:?}"
            );
        }
    }

    #[test]
    fn resume_id_must_be_plausible() {
        for bad in [
            &["--resume", "x"][..],
            &["-r", "a b c d e f g h"],
            &["--resume=../../etc/passwd"],
            &["--resume", "$(rm -rf ~)"],
        ] {
            assert!(
                matches!(route(&args(bad)), ClaudeRoute::Forbidden(_)),
                "{bad:?}"
            );
        }
        // Valor que começa com `-` é a próxima flag, não o id: `-r` sozinho
        // abre o seletor.
        assert_eq!(route(&args(&["-r", "--fork-session"])), ClaudeRoute::Host);
        assert_eq!(
            route(&args(&["--resume=11111111-2222-3333-4444-555555555555"])),
            ClaudeRoute::Host
        );
    }

    #[test]
    fn real_claude_prefers_the_resolved_binary_and_never_the_wrapper() {
        let wrapper = tempfile::tempdir().unwrap();
        let real = tempfile::tempdir().unwrap();
        let wrapper_claude = wrapper.path().join("claude");
        let real_claude_path = real.path().join("claude");
        let exists = |p: &Path| p == wrapper_claude || p == real_claude_path;

        // O resolvido pelo core vence.
        assert_eq!(
            real_claude(
                Some(real_claude_path.to_str().unwrap()),
                None,
                Some(wrapper.path()),
                exists
            ),
            Some(real_claude_path.clone())
        );
        // Resolvido apontando para o wrapper é ignorado: cai no PATH, que
        // também pula o wrapper mesmo vindo primeiro.
        let path = std::env::join_paths([wrapper.path(), real.path()]).unwrap();
        assert_eq!(
            real_claude(
                Some(wrapper_claude.to_str().unwrap()),
                path.to_str(),
                Some(wrapper.path()),
                exists
            ),
            Some(real_claude_path.clone())
        );
        // Só o wrapper no PATH: nenhum claude real, nunca o wrapper.
        let only_wrapper = wrapper.path().to_str().unwrap().to_string();
        assert_eq!(
            real_claude(None, Some(&only_wrapper), Some(wrapper.path()), exists),
            None
        );
    }

    #[test]
    fn real_claude_ignores_relative_entries() {
        assert_eq!(
            real_claude(Some("claude"), Some(".:bin"), None, |_| true),
            None
        );
    }
}
