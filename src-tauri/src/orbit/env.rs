//! Variáveis do grafo como variáveis de ambiente (`$NOME`) nos terminais do
//! grafo: o shell do nó Terminal e os processos (processo e integração).
//!
//! Agente não recebe: o env de sessão de agente é por allowlist (CLAUDE.md,
//! princípio 6), e a variável do grafo chega a ele só pelo `{{nome}}` que o
//! dono escreveu no prompt.
//!
//! Só exporta nome válido de env e fora da lista do sistema: uma variável do
//! grafo chamada `PATH` ou `TYBA_IN` mudaria o shell ou o wrapper do TYBA por
//! baixo do dono.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

/// Por que um nome de variável do grafo não vira variável de ambiente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvNameProblem {
    /// Fora de `[A-Za-z_][A-Za-z0-9_]*`.
    Invalid,
    /// Do sistema, do shell ou do TYBA ([`RESERVED`], [`RESERVED_PREFIXES`]).
    Reserved,
}

/// Nomes que a variável do grafo nunca sobrescreve, comparados sem caixa
/// (no zsh `path` é o `PATH`; no Windows o env não tem caixa).
pub const RESERVED: &[&str] = &[
    // Shell e sessão.
    "PATH",
    "HOME",
    "SHELL",
    "USER",
    "LOGNAME",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "IFS",
    "PS1",
    "PS2",
    "PS3",
    "PS4",
    "PROMPT_COMMAND",
    "ZDOTDIR",
    "BASH_ENV",
    "ENV",
    "CDPATH",
    "FPATH",
    "MANPATH",
    "HISTFILE",
    "DISPLAY",
    "_",
    // Windows.
    "SYSTEMROOT",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
];

/// Prefixos reservados: o TYBA (`TYBA_IN`, `TYBA_OUT`, integração do shell),
/// locale, carregador dinâmico (injeção de biblioteca), git (o `pre-push` do
/// TYBA vai por `GIT_CONFIG_*`), ssh-agent e XDG.
pub const RESERVED_PREFIXES: &[&str] = &["TYBA_", "LC_", "LD_", "DYLD_", "GIT_", "SSH_", "XDG_"];

pub fn env_name_problem(name: &str) -> Option<EnvNameProblem> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Some(EnvNameProblem::Invalid);
    }
    let upper = name.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) || RESERVED_PREFIXES.iter().any(|p| upper.starts_with(p))
    {
        return Some(EnvNameProblem::Reserved);
    }
    None
}

/// O env que as variáveis do grafo somam ao terminal: só nome sem problema.
pub fn graph_env(variables: &[(String, String)]) -> Vec<(String, String)> {
    variables
        .iter()
        .filter(|(name, _)| env_name_problem(name).is_none())
        .cloned()
        .collect()
}

/// A impressão (SHA-256 do valor) de cada variável que vira env, por nome:
/// com ela o core sabe se o valor mudou sem guardar o valor. Fica só na
/// memória do core — nunca vai ao front, ao banco nem a log — e é como a
/// secreta se compara sem sair em claro.
pub type EnvPrints = BTreeMap<String, [u8; 32]>;

pub fn env_prints(variables: &[(String, String)]) -> EnvPrints {
    graph_env(variables)
        .into_iter()
        .map(|(name, value)| (name, Sha256::digest(value.as_bytes()).into()))
        .collect()
}

/// Os nomes cujo env mudou de `opened` para `now` (valor trocado, variável
/// nova ou removida), em ordem.
pub fn changed_names(opened: &EnvPrints, now: &EnvPrints) -> Vec<String> {
    let names: BTreeSet<&String> = opened.keys().chain(now.keys()).collect();
    names
        .into_iter()
        .filter(|name| opened.get(*name) != now.get(*name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_pass_and_invalid_ones_are_named() {
        for ok in ["TOKEN", "BASE_URL", "_x", "port", "a1"] {
            assert_eq!(env_name_problem(ok), None, "{ok}");
        }
        for bad in ["", "1A", "BASE-URL", "a b", "nome.x", "ção", "A=B", "x$"] {
            assert_eq!(
                env_name_problem(bad),
                Some(EnvNameProblem::Invalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn system_shell_and_tyba_names_are_reserved_in_any_case() {
        for name in [
            "PATH",
            "path",
            "HOME",
            "SHELL",
            "USER",
            "IFS",
            "ZDOTDIR",
            "BASH_ENV",
            "_",
            "TYBA_IN",
            "TYBA_OUT",
            "tyba_prompt_mode",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "GIT_CONFIG_COUNT",
            "SSH_AUTH_SOCK",
            "LC_ALL",
            "XDG_CONFIG_HOME",
            "ComSpec",
        ] {
            assert_eq!(
                env_name_problem(name),
                Some(EnvNameProblem::Reserved),
                "{name}"
            );
        }
        // Prefixo é prefixo: `GITHUB_TOKEN` e `TYBA` sozinho passam.
        assert_eq!(env_name_problem("GITHUB_TOKEN"), None);
        assert_eq!(env_name_problem("TYBA"), None);
        assert_eq!(env_name_problem("PATHS"), None);
    }

    #[test]
    fn graph_env_exports_only_names_without_problem() {
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        let vars = vec![
            pair("BASE_URL", "http://localhost:3000"),
            pair("PATH", "/tmp/evil"),
            pair("TOKEN", "abc"),
            pair("TYBA_OUT", "/tmp/x"),
            pair("modo-dev", "1"),
        ];
        assert_eq!(
            graph_env(&vars),
            vec![
                pair("BASE_URL", "http://localhost:3000"),
                pair("TOKEN", "abc")
            ]
        );
    }

    #[test]
    fn changed_names_follow_the_value_and_return_when_it_goes_back() {
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        let opened = env_prints(&[
            pair("API_URL", "http://localhost:3000"),
            pair("MODELO", "sonnet"),
        ]);
        assert!(changed_names(&opened, &opened).is_empty());
        let edited = env_prints(&[
            pair("API_URL", "http://localhost:4000"),
            pair("MODELO", "sonnet"),
        ]);
        assert_eq!(changed_names(&opened, &edited), vec!["API_URL".to_string()]);
        // Voltou ao valor de antes: nada mudou.
        let back = env_prints(&[
            pair("API_URL", "http://localhost:3000"),
            pair("MODELO", "sonnet"),
        ]);
        assert!(changed_names(&opened, &back).is_empty());
        // Nova, removida e nome que não vira env.
        let other = env_prints(&[
            pair("API_URL", "http://localhost:3000"),
            pair("NOVA", "1"),
            pair("porta-api", "8080"),
        ]);
        assert_eq!(
            changed_names(&opened, &other),
            vec!["MODELO".to_string(), "NOVA".to_string()]
        );
    }

    /// A impressão não carrega o valor: só o hash.
    #[test]
    fn prints_do_not_hold_the_value() {
        // Fixture sintética.
        let fake = "fixture-print-secret-Zx81";
        let prints = env_prints(&[("TOKEN".into(), fake.into())]);
        let print = prints["TOKEN"];
        assert!(!print.windows(fake.len()).any(|w| w == fake.as_bytes()));
        assert_ne!(
            print,
            env_prints(&[("TOKEN".into(), "outro".into())])["TOKEN"]
        );
    }
}
