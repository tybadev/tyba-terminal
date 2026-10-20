//! `tyba _codex -- <args>`: o `codex` digitado num pane (decisão do dono,
//! 2026-10-19). Igual ao shim do claude no caminho (função no rc e wrapper na
//! frente do PATH do pane), mas sem hospedar nada: sem gate, sem jaula, sem
//! card. Ele só soma ao codex de verdade o que o codex aberto pelo TYBA já
//! tem:
//!
//! - o hook `pre-push` do TYBA (`core.hooksPath` por `GIT_CONFIG_*`, somado ao
//!   do pane) — no env e pelo `-c shell_environment_policy.set.*`, que o Codex
//!   aplica por último;
//! - (sem `-c hooks.*`: substituiria os hooks do dono — ver [`codex_argv`]);
//! - `FORCE_HYPERLINK=1`.
//!
//! Escotilha: o caminho absoluto do codex pula o shim, e `TYBA_AGENT_SHIM=0`
//! desliga a função e o wrapper — as mesmas do claude.

use std::path::{Path, PathBuf};

/// O argv do codex de verdade: os `--config` do TYBA na frente (opções
/// globais do codex) e os args do usuário intactos depois.
///
/// Sem `-c hooks.*`: o `-c` do codex troca o valor da chave inteira (os
/// eventos de `hooks` e o `hooks.state` de confiança), então passar os nossos
/// desligaria os hooks que o dono tem no `~/.codex/config.toml`. Não dá para
/// garantir a soma sem rodar o codex; o codex digitado fica com o hook git
/// (`pre-push`) e o `shell_environment_policy.set.*`, que é por chave e só
/// acrescenta.
pub fn codex_argv(user_args: &[String], env_get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut argv = Vec::new();
    for (key, value) in crate::agent::git_hooks::hook_vars(env_get) {
        argv.push("--config".to_string());
        argv.push(format!(
            "shell_environment_policy.set.{key}={}",
            crate::agent::codex_hooks::toml_string(&value)
        ));
    }
    argv.extend(user_args.iter().cloned());
    argv
}

/// O codex de verdade: `TYBA_REAL_CODEX` (absoluto, executável, fora da pasta
/// do wrapper) ou o primeiro `codex` do PATH que não seja o wrapper.
pub fn real_codex(
    real_env: Option<&str>,
    path: Option<&str>,
    wrapper_dir: Option<&Path>,
    is_executable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let outside_wrapper = |p: &Path| wrapper_dir.is_none_or(|w| p.parent() != Some(w));
    if let Some(real) = real_env.map(PathBuf::from) {
        if real.is_absolute() && outside_wrapper(&real) && is_executable(&real) {
            return Some(real);
        }
    }
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("codex"))
        .find(|candidate| outside_wrapper(candidate) && is_executable(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Ponto de entrada de `tyba _codex -- <args>`. Nunca trava o terminal: sem
/// o hook (pasta que não grava), o codex sobe assim mesmo, com aviso.
#[cfg(unix)]
pub fn maybe_run_codex_mode() -> Option<i32> {
    use std::os::unix::process::CommandExt;

    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) != Some("_codex") {
        return None;
    }
    let user_args: Vec<String> = match argv.get(2).map(String::as_str) {
        Some("--") => argv[3..].to_vec(),
        _ => argv[2..].to_vec(),
    };
    let wrapper_dir = std::env::var_os("TYBA_WRAPPER_DIR").map(PathBuf::from);
    let Some(real) = real_codex(
        std::env::var("TYBA_REAL_CODEX").ok().as_deref(),
        std::env::var("PATH").ok().as_deref(),
        wrapper_dir.as_deref(),
        is_executable_file,
    ) else {
        eprintln!("tyba _codex: codex não encontrado no PATH");
        return Some(127);
    };
    let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
    match crate::session::ensure_git_hooks() {
        Ok(dir) => crate::agent::git_hooks::with_hooks(&mut env, &dir),
        Err(detail) => eprintln!(
            "tyba: o codex sobe sem o hook pre-push do TYBA (push para main/master): {detail}"
        ),
    }
    env.insert(
        crate::agent::session::FORCE_HYPERLINK.0.into(),
        crate::agent::session::FORCE_HYPERLINK.1.into(),
    );
    let args = codex_argv(&user_args, |k| env.get(k).cloned());
    let err = std::process::Command::new(real)
        .args(args)
        .env_clear()
        .envs(&env)
        .exec();
    eprintln!("tyba _codex: exec do codex falhou: {err}");
    Some(126)
}

#[cfg(not(unix))]
pub fn maybe_run_codex_mode() -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_argv_puts_tyba_options_first_and_keeps_user_args() {
        let mut env = std::collections::HashMap::new();
        crate::agent::git_hooks::with_hooks(&mut env, Path::new("/dados/git-hooks"));
        let argv = codex_argv(&["exec".to_string(), "faz x".to_string()], |k| {
            env.get(k).cloned()
        });
        assert_eq!(&argv[argv.len() - 2..], ["exec", "faz x"]);
        // Nada de `hooks.*`: trocaria os hooks do dono.
        assert!(!argv.iter().any(|a| a.starts_with("hooks.")));
        assert!(argv
            .iter()
            .any(|a| a == "shell_environment_policy.set.GIT_CONFIG_KEY_0=\"core.hooksPath\""));
    }

    #[test]
    fn real_codex_skips_the_wrapper() {
        let exec = |p: &Path| p.starts_with("/w") || p.starts_with("/opt");
        assert_eq!(
            real_codex(
                None,
                Some("/w/bin:/opt/bin"),
                Some(Path::new("/w/bin")),
                exec
            ),
            Some(PathBuf::from("/opt/bin/codex"))
        );
        assert_eq!(
            real_codex(
                Some("/w/bin/codex"),
                Some("/opt/bin"),
                Some(Path::new("/w/bin")),
                exec
            ),
            Some(PathBuf::from("/opt/bin/codex"))
        );
        assert_eq!(
            real_codex(Some("/opt/x/codex"), None, Some(Path::new("/w/bin")), exec),
            Some(PathBuf::from("/opt/x/codex"))
        );
    }
}
