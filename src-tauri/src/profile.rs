//! Perfil do binário: o build de desenvolvimento é **outro app**.
//!
//! O dono usa o TYBA instalado (DMG) como terminal do dia a dia enquanto
//! desenvolve. Os dois não podem dividir nada: banco, `~/.tyba`, worktrees
//! gerenciados (o GC de órfãos de um apagaria os do outro), dados do app e o
//! socket do canal shim↔core.
//!
//! O identificador do Tauri vem do `tauri.dev.conf.json` (`bun run tauri dev`
//! o injeta). O resto não cabe em config: o `tyba _jail` é o mesmo binário,
//! roda fora do Tauri e precisa calcular o MESMO socket que o core bindou — só
//! uma constante de compilação garante isso aos dois lados. Daí
//! `debug_assertions`, conferido contra o identificador no `.setup()`
//! ([`check_identifier`]).
//!
//! Nos testes de unidade vale o perfil de release, para os caminhos esperados
//! continuarem os de produção.

pub const IS_DEV: bool = cfg!(all(debug_assertions, not(test)));

pub const RELEASE_IDENTIFIER: &str = "dev.tyba.app";
pub const DEV_IDENTIFIER: &str = "dev.tyba.app.dev";
pub const IDENTIFIER: &str = if IS_DEV {
    DEV_IDENTIFIER
} else {
    RELEASE_IDENTIFIER
};

/// Pasta do usuário na home: `~/.tyba` ou `~/.tyba-dev`.
pub const RELEASE_HOME_DIR: &str = ".tyba";
pub const DEV_HOME_DIR: &str = ".tyba-dev";
pub const HOME_DIR: &str = if IS_DEV {
    DEV_HOME_DIR
} else {
    RELEASE_HOME_DIR
};

/// Sufixo dos nomes de socket/pasta temporária do canal.
pub const CHANNEL_TAG: &str = if IS_DEV { "tyba-dev" } else { "tyba" };

/// Nome nas janelas e no menu: o `productName` da config do Tauri ("Tyba", ou
/// "TYBA Dev" pelo `tauri.dev.conf.json`), não uma constante de compilação.
pub fn product_name(config: &tauri::Config) -> String {
    config.product_name.clone().unwrap_or_else(|| "Tyba".into())
}

/// O identificador com que o Tauri subiu tem de ser o deste binário: um build
/// de dev aberto com o `tauri.conf.json` de release abriria o banco do app
/// instalado.
pub fn check_identifier(identifier: &str) -> Result<(), String> {
    if identifier == IDENTIFIER {
        return Ok(());
    }
    Err(format!(
        "identificador '{identifier}' não bate com o perfil deste binário ('{IDENTIFIER}'). \
         Em desenvolvimento, rode `bun run tauri dev` (que usa src-tauri/tauri.dev.conf.json)."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_tests_see_the_release_profile() {
        const { assert!(!IS_DEV) };
        assert_eq!(IDENTIFIER, "dev.tyba.app");
        assert_eq!(HOME_DIR, ".tyba");
    }

    #[test]
    fn product_name_comes_from_the_tauri_config() {
        let mut config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(product_name(&config), "Tyba");
        config.product_name = Some("TYBA Dev".into());
        assert_eq!(product_name(&config), "TYBA Dev");
        config.product_name = None;
        assert_eq!(product_name(&config), "Tyba");
    }

    #[test]
    fn identifier_mismatch_is_refused() {
        assert!(check_identifier(RELEASE_IDENTIFIER).is_ok());
        assert!(check_identifier(DEV_IDENTIFIER).is_err());
    }

    #[test]
    fn dev_conf_declares_the_dev_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.dev.conf.json")).unwrap();
        assert_eq!(conf["identifier"], DEV_IDENTIFIER);
        assert_eq!(conf["productName"], "TYBA Dev");
        let release: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(release["identifier"], RELEASE_IDENTIFIER);
    }
}
