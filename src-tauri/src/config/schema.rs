//! O que o `~/.tyba/config.toml` pode dizer, e como cada coisa é validada.
//!
//! O arquivo é entrada não confiável: tudo passa por tipo e faixa, e o que não
//! passa vira aviso e cai para o padrão. Erro só existe para TOML quebrado, e
//! mesmo aí quem chama mantém o último estado bom — nunca derruba o app nem
//! reescreve o arquivo.
//!
//! Os valores saem no formato das chaves de `settings` que o resto do core e o
//! front já usam (`theme.mode`, `pref.ui_font`…), para que trocar a origem —
//! SQLite para arquivo — não mude o contrato de ninguém.

use std::collections::BTreeMap;

use toml_edit::{Document, Item, Value};

pub const KEY_THEME_MODE: &str = "theme.mode";
pub const KEY_THEME_DARK: &str = "theme.slot.dark";
pub const KEY_THEME_LIGHT: &str = "theme.slot.light";
pub const KEY_UI_FONT: &str = "pref.ui_font";
pub const KEY_FONT_SIZE: &str = "pref.code.font_size";
pub const KEY_BINDINGS: &str = "pref.keybindings";
/// Não tem par no SQLite: nasceu no arquivo. Lida por
/// [`super::ConfigManager::app_icon`].
pub const KEY_APP_ICON: &str = "appearance.app_icon";
pub const APP_ICONS: &[&str] = &[
    "blackout",
    "aurora",
    "filamento",
    "assinatura",
    "fosforo",
    "violeta",
    "grafite",
    "papel",
];
pub const DEFAULT_APP_ICON: &str = "blackout";

/// Chave de `settings` → valor, no formato que ela tinha no SQLite.
pub type Values = BTreeMap<&'static str, String>;

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Enum(&'static [&'static str]),
    ThemeId,
    Int(i64, i64),
}

pub struct Field {
    pub section: &'static str,
    pub name: &'static str,
    pub key: &'static str,
    pub kind: Kind,
}

pub const FIELDS: &[Field] = &[
    Field {
        section: "theme",
        name: "mode",
        key: KEY_THEME_MODE,
        kind: Kind::Enum(&["dark", "light", "system"]),
    },
    Field {
        section: "theme",
        name: "dark",
        key: KEY_THEME_DARK,
        kind: Kind::ThemeId,
    },
    Field {
        section: "theme",
        name: "light",
        key: KEY_THEME_LIGHT,
        kind: Kind::ThemeId,
    },
    Field {
        section: "font",
        name: "ui",
        key: KEY_UI_FONT,
        kind: Kind::Enum(&["mono", "grotesk"]),
    },
    Field {
        section: "font",
        name: "terminal_size",
        key: KEY_FONT_SIZE,
        kind: Kind::Int(10, 20),
    },
    Field {
        section: "appearance",
        name: "app_icon",
        key: KEY_APP_ICON,
        kind: Kind::Enum(APP_ICONS),
    },
];

pub const SECTION_BINDINGS: &str = "keybindings";
/// Dono: [`crate::user_config`]. Aqui só não vira "seção desconhecida".
const SECTION_SANDBOX: &str = "sandbox";
const SANDBOX_KEYS: &[&str] = &["read_allow"];

/// Toda chave que este módulo grava. Fora daqui, `pref.*` continua no SQLite.
pub const FILE_KEYS: &[&str] = &[
    KEY_THEME_MODE,
    KEY_THEME_DARK,
    KEY_THEME_LIGHT,
    KEY_UI_FONT,
    KEY_FONT_SIZE,
    KEY_BINDINGS,
    KEY_APP_ICON,
];

/// Espelho de `KEY_ACTIONS` em `src/lib/keys.ts` — o teste
/// `key_actions_match_the_front` falha se os dois divergirem.
pub const KEY_ACTIONS: &[&str] = &[
    "paletteActions",
    "paletteSessions",
    "paletteHistory",
    "paletteSnippets",
    "panel",
    "files",
    "filesFinder",
    "clear",
    "settings",
    "newSession",
    "newWorktreeSession",
    "newTab",
    "newWindow",
    "closePane",
    "openFolder",
    "prevSession",
    "nextSession",
    "nextAttentionSession",
    "prevTab",
    "nextTab",
    "tabBack",
    "splitRight",
    "splitDown",
    "nextPane",
    "paneLeft",
    "paneRight",
    "paneUp",
    "paneDown",
    "copy",
    "paste",
    "search",
    "selectAll",
    "richInput",
    "promptLine",
];

/// Ordem em que o front monta o combo (`comboOf` em `src/lib/keys.ts`).
const MODIFIERS: &[&str] = &["meta", "ctrl", "alt", "shift"];
const MAX_KEY_LEN: usize = 32;
const MAX_THEME_ID_LEN: usize = 64;

/// Palavras que, numa chave desconhecida, soam como tentar afrouxar uma ação
/// vermelha. A chave já seria ignorada; o aviso diz por quê.
const RED_HINTS: &[&str] = &[
    "allow", "approv", "permit", "trust", "yolo", "skip", "push", "sudo", "rm_", "network",
    "danger", "unsafe", "bypass", "confirm",
];
const SECRET_HINTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "credential",
    "private_key",
];

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub values: Values,
    /// Em pt-BR, para gente ler. Nunca repete o valor que veio no arquivo — ele
    /// pode ser um segredo colado no lugar errado.
    pub warnings: Vec<String>,
}

pub fn field_by_key(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.key == key)
}

pub fn is_theme_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_THEME_ID_LEN
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Combo no formato do front, com os modificadores na ordem canônica — quem
/// escreve à mão `shift+meta+p` recebe `meta+shift+p`, senão o atalho nunca
/// casaria com o `comboOf` do teclado.
pub fn normalize_combo(raw: &str) -> Option<String> {
    // Só o espaço da frente sai: o de trás pode ser a tecla Espaço, que o
    // `comboOf` grava como `"ctrl+ "`.
    let lower = raw.trim_start().to_lowercase();
    let (mods, key) = match lower.rsplit_once('+') {
        Some((mods, key)) => (mods, key),
        None => return None,
    };
    // `meta++` é o atalho da tecla "+": o rsplit deixa `meta+` e "".
    let (mods, key) = if key.is_empty() && mods.ends_with('+') {
        (&mods[..mods.len() - 1], "+")
    } else {
        (mods, key)
    };
    // `space` é o jeito legível de escrever à mão; espaço sobrando em volta de
    // outra tecla é só digitação.
    let key = match key.trim() {
        "space" => " ",
        "" if !key.is_empty() => " ",
        k => k,
    };
    if key.is_empty() || key.chars().count() > MAX_KEY_LEN || key.chars().any(char::is_control) {
        return None;
    }
    if MODIFIERS.contains(&key) {
        return None;
    }
    let mut seen = [false; 4];
    for part in mods.split('+') {
        let i = MODIFIERS.iter().position(|m| *m == part)?;
        if seen[i] {
            return None;
        }
        seen[i] = true;
    }
    let mut out: Vec<&str> = MODIFIERS
        .iter()
        .zip(seen)
        .filter_map(|(m, on)| on.then_some(*m))
        .collect();
    out.push(key);
    Some(out.join("+"))
}

fn check_scalar(field: &Field, value: &Value) -> Option<String> {
    match field.kind {
        Kind::Enum(options) => value
            .as_str()
            .filter(|s| options.contains(s))
            .map(str::to_owned),
        Kind::ThemeId => value.as_str().filter(|s| is_theme_id(s)).map(str::to_owned),
        Kind::Int(min, max) => value
            .as_integer()
            .filter(|n| (min..=max).contains(n))
            .map(|n| n.to_string()),
    }
}

fn expected(kind: Kind) -> String {
    match kind {
        Kind::Enum(options) => format!("use {}", options.join(", ")),
        Kind::ThemeId => "use o id do tema (letras minúsculas, números e '-')".into(),
        Kind::Int(min, max) => format!("use um número de {min} a {max}"),
    }
}

fn unknown_warning(path: &str) -> String {
    let lower = path.to_lowercase();
    if SECRET_HINTS.iter().any(|h| lower.contains(h)) {
        format!(
            "{path}: parece um segredo e foi ignorado. Tokens e senhas ficam no \
             Keychain, nunca neste arquivo."
        )
    } else if RED_HINTS.iter().any(|h| lower.contains(h)) {
        format!(
            "{path}: ações vermelhas (push, sudo, rede, rm -rf…) nunca são \
             configuráveis; chave ignorada."
        )
    } else {
        format!("{path}: chave desconhecida, ignorada.")
    }
}

/// Valores com cara de segredo, em qualquer lugar do arquivo — inclusive nas
/// chaves que este módulo conhece, onde o valor ainda seria recusado pelo tipo.
fn scan_secrets(path: &str, item: &Item, warnings: &mut Vec<String>) {
    match item {
        Item::Value(v) => scan_value_secrets(path, v, warnings),
        Item::Table(t) => {
            for (k, child) in t.iter() {
                scan_secrets(&format!("{path}.{k}"), child, warnings);
            }
        }
        Item::ArrayOfTables(a) => {
            for t in a.iter() {
                for (k, child) in t.iter() {
                    scan_secrets(&format!("{path}.{k}"), child, warnings);
                }
            }
        }
        Item::None => {}
    }
}

fn scan_value_secrets(path: &str, value: &Value, warnings: &mut Vec<String>) {
    match value {
        Value::String(s) if crate::session::redact::contains_secret(s.value()) => {
            warnings.push(format!(
                "{path}: o valor parece um segredo (token, chave). Remova do \
                 arquivo e guarde no Keychain."
            ));
        }
        Value::Array(a) => {
            for v in a.iter() {
                scan_value_secrets(path, v, warnings);
            }
        }
        Value::InlineTable(t) => {
            for (k, v) in t.iter() {
                scan_value_secrets(&format!("{path}.{k}"), v, warnings);
            }
        }
        _ => {}
    }
}

fn parse_bindings(item: &Item, warnings: &mut Vec<String>) -> Option<String> {
    let Some(table) = item.as_table_like() else {
        warnings.push(format!(
            "{SECTION_BINDINGS}: esperava uma seção [{SECTION_BINDINGS}]; ignorada."
        ));
        return None;
    };
    let mut out = BTreeMap::new();
    for (action, value) in table.iter() {
        let path = format!("{SECTION_BINDINGS}.{action}");
        if !KEY_ACTIONS.contains(&action) {
            warnings.push(format!("{path}: ação desconhecida, ignorada."));
            continue;
        }
        match value.as_str().and_then(normalize_combo) {
            Some(combo) => {
                out.insert(action.to_string(), combo);
            }
            None => warnings.push(format!(
                "{path}: atalho inválido (use modificadores+tecla, ex.: \"meta+shift+p\"); \
                 usando o padrão."
            )),
        }
    }
    if out.is_empty() {
        return None;
    }
    serde_json::to_string(&out).ok()
}

/// Erro de TOML só com a linha: a mensagem da biblioteca cita o trecho do
/// arquivo, e ele pode ser um token colado no lugar errado.
pub fn toml_error(text: &str, span: Option<std::ops::Range<usize>>) -> String {
    match span {
        Some(span) => {
            let start = span.start.min(text.len());
            let line = text.as_bytes()[..start]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1;
            format!("TOML inválido na linha {line}")
        }
        None => "TOML inválido".to_string(),
    }
}

pub fn parse(text: &str) -> Result<Parsed, String> {
    let doc: Document = text
        .parse()
        .map_err(|e: toml_edit::TomlError| toml_error(text, e.span()))?;
    let mut parsed = Parsed::default();
    let warnings = &mut parsed.warnings;

    for (section, item) in doc.iter() {
        scan_secrets(section, item, warnings);
        if section == SECTION_BINDINGS {
            if let Some(json) = parse_bindings(item, warnings) {
                parsed.values.insert(KEY_BINDINGS, json);
            }
            continue;
        }
        let known_names: Vec<&str> = if section == SECTION_SANDBOX {
            SANDBOX_KEYS.to_vec()
        } else {
            FIELDS
                .iter()
                .filter(|f| f.section == section)
                .map(|f| f.name)
                .collect()
        };
        if known_names.is_empty() {
            warnings.push(unknown_warning(section));
            continue;
        }
        let Some(table) = item.as_table_like() else {
            warnings.push(format!(
                "{section}: esperava uma seção [{section}]; ignorada."
            ));
            continue;
        };
        for (name, child) in table.iter() {
            let path = format!("{section}.{name}");
            if !known_names.contains(&name) {
                warnings.push(unknown_warning(&path));
                continue;
            }
            if section == SECTION_SANDBOX {
                continue;
            }
            let field = FIELDS
                .iter()
                .find(|f| f.section == section && f.name == name)
                .expect("nome conhecido tem campo");
            // Segredo já foi avisado pelo `scan_secrets`; aqui só não entra.
            if child
                .as_str()
                .is_some_and(crate::session::redact::contains_secret)
            {
                continue;
            }
            match child.as_value().and_then(|v| check_scalar(field, v)) {
                Some(v) => {
                    parsed.values.insert(field.key, v);
                }
                None => warnings.push(format!(
                    "{path}: valor inválido ({}); usando o padrão.",
                    expected(field.kind)
                )),
            }
        }
    }
    // Caminho de chave também é texto do arquivo: `"sk-…" = 1` não volta.
    for w in &mut parsed.warnings {
        if let std::borrow::Cow::Owned(r) = crate::session::redact::redact(w) {
            *w = r;
        }
    }
    parsed.warnings.dedup();
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Parsed {
        parse(text).unwrap()
    }

    #[test]
    fn empty_file_has_no_values_and_no_warnings() {
        assert_eq!(ok(""), Parsed::default());
        assert_eq!(ok("# só comentário\n"), Parsed::default());
    }

    #[test]
    fn reads_every_known_field() {
        let p = ok(r#"
            [theme]
            mode = "system"
            dark = "dracula"
            light = "solarized-light"
            [font]
            ui = "grotesk"
            terminal_size = 15
        "#);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.values[KEY_THEME_MODE], "system");
        assert_eq!(p.values[KEY_THEME_DARK], "dracula");
        assert_eq!(p.values[KEY_THEME_LIGHT], "solarized-light");
        assert_eq!(p.values[KEY_UI_FONT], "grotesk");
        assert_eq!(p.values[KEY_FONT_SIZE], "15");
    }

    #[test]
    fn invalid_values_fall_back_with_a_warning_that_never_echoes_the_value() {
        let p = ok(r#"
            [theme]
            mode = "blue"
            dark = "../../etc/passwd"
            [font]
            ui = 3
            terminal_size = 99
        "#);
        assert!(p.values.is_empty(), "{:?}", p.values);
        assert_eq!(p.warnings.len(), 4, "{:?}", p.warnings);
        for w in &p.warnings {
            assert!(
                !w.contains("blue") && !w.contains("passwd") && !w.contains("99"),
                "{w}"
            );
        }
        assert!(p.warnings[0].starts_with("theme.mode:"));
    }

    #[test]
    fn font_size_range_is_inclusive() {
        assert_eq!(ok("[font]\nterminal_size = 10").values[KEY_FONT_SIZE], "10");
        assert_eq!(ok("[font]\nterminal_size = 20").values[KEY_FONT_SIZE], "20");
        assert!(ok("[font]\nterminal_size = 9").values.is_empty());
        assert!(ok("[font]\nterminal_size = 21").values.is_empty());
        assert!(ok("[font]\nterminal_size = 13.5").values.is_empty());
        assert!(ok("[font]\nterminal_size = \"13\"").values.is_empty());
    }

    #[test]
    fn unknown_keys_and_sections_warn_and_are_ignored() {
        let p = ok(r##"
            [theme]
            mode = "light"
            accent = "#fff"
            [future]
            x = 1
        "##);
        assert_eq!(p.values.len(), 1);
        assert_eq!(
            p.warnings,
            vec![
                "theme.accent: chave desconhecida, ignorada.".to_string(),
                "future: chave desconhecida, ignorada.".to_string(),
            ]
        );
    }

    #[test]
    fn section_with_wrong_shape_warns() {
        let p = ok("theme = \"dark\"\n");
        assert!(p.values.is_empty());
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("[theme]"));
    }

    #[test]
    fn dotted_and_inline_tables_are_read_too() {
        let p = ok("theme.mode = \"light\"\nfont = { ui = \"grotesk\" }\n");
        assert_eq!(p.values[KEY_THEME_MODE], "light");
        assert_eq!(p.values[KEY_UI_FONT], "grotesk");
    }

    #[test]
    fn sandbox_read_allow_is_known_but_not_ours() {
        let p = ok("[sandbox]\nread_allow = [\"~/.kube\"]\n");
        assert_eq!(p, Parsed::default());
    }

    #[test]
    fn red_action_attempts_are_named_as_such() {
        let p = ok(r#"
            [approvals]
            always_allow = ["git push"]
            [sandbox]
            allow_network = true
        "#);
        assert!(p.values.is_empty());
        assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
        assert!(p
            .warnings
            .iter()
            .all(|w| w.contains("nunca são configuráveis")));
    }

    #[test]
    fn secrets_are_refused_with_a_warning() {
        let p = ok(r#"
            github_token = "x"
            [theme]
            dark = "sk-abcdefghijklmnopqrstuvwx"
        "#);
        assert!(p.values.is_empty());
        assert!(p
            .warnings
            .iter()
            .any(|w| w.starts_with("github_token: parece um segredo")));
        assert!(p
            .warnings
            .iter()
            .any(|w| w.starts_with("theme.dark: o valor parece um segredo")));
        assert!(p.warnings.iter().all(|w| !w.contains("sk-abc")));
    }

    #[test]
    fn broken_toml_is_an_error_not_a_panic() {
        assert!(parse("[theme\nmode = ").is_err());
        assert!(parse("[theme]\nmode = \"a\"\nmode = \"b\"\n").is_err());
    }

    #[test]
    fn broken_toml_error_gives_the_line_but_never_the_text() {
        let e =
            parse("[theme]\nmode = \"dark\"\ndark = sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA\n")
                .unwrap_err();
        assert!(!e.contains("sk-ant"), "{e}");
        assert_eq!(e, "TOML inválido na linha 3");
    }

    #[test]
    fn secret_looking_key_name_is_not_echoed() {
        let p = ok("\"sk-abcdefghijklmnopqrstuvwx\" = 1\n");
        assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
        assert!(
            p.warnings.iter().all(|w| !w.contains("sk-abc")),
            "{:?}",
            p.warnings
        );
    }

    #[test]
    fn keybindings_become_the_json_the_front_reads() {
        let p = ok(r#"
            [keybindings]
            paletteActions = "Shift+Meta+P"
            newTab = "ctrl+t"
        "#);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        let map: BTreeMap<String, String> = serde_json::from_str(&p.values[KEY_BINDINGS]).unwrap();
        assert_eq!(map["paletteActions"], "meta+shift+p");
        assert_eq!(map["newTab"], "ctrl+t");
    }

    #[test]
    fn bad_keybindings_warn_per_entry() {
        let p = ok(r#"
            [keybindings]
            launchMissiles = "meta+m"
            newTab = "t"
            copy = "meta+meta+c"
            paste = "meta+v"
        "#);
        assert_eq!(p.warnings.len(), 3, "{:?}", p.warnings);
        let map: BTreeMap<String, String> = serde_json::from_str(&p.values[KEY_BINDINGS]).unwrap();
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn app_icon_is_validated_strictly() {
        let p = ok("[appearance]\napp_icon = \"aurora\"\n");
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.values[KEY_APP_ICON], "aurora");
        for bad in ["\"Aurora\"", "\"neon\"", "1"] {
            let p = ok(&format!("[appearance]\napp_icon = {bad}\n"));
            assert!(p.values.is_empty());
            assert_eq!(p.warnings.len(), 1);
            assert!(p.warnings[0].starts_with("appearance.app_icon: valor inválido"));
        }
        assert!(APP_ICONS.contains(&DEFAULT_APP_ICON));
    }

    #[test]
    fn empty_keybindings_table_means_defaults() {
        assert!(ok("[keybindings]\n").values.is_empty());
    }

    #[test]
    fn normalize_combo_cases() {
        assert_eq!(normalize_combo("meta+,").as_deref(), Some("meta+,"));
        assert_eq!(
            normalize_combo("alt+ctrl+ArrowLeft").as_deref(),
            Some("ctrl+alt+arrowleft")
        );
        assert_eq!(normalize_combo("meta++").as_deref(), Some("meta++"));
        assert_eq!(normalize_combo("p"), None);
        assert_eq!(normalize_combo("hyper+p"), None);
        assert_eq!(normalize_combo("meta+shift"), None);
        assert_eq!(normalize_combo("meta+\u{7}"), None);
        assert_eq!(normalize_combo(&format!("meta+{}", "x".repeat(33))), None);
    }

    #[test]
    fn space_key_matches_combo_of() {
        assert_eq!(normalize_combo("ctrl+ ").as_deref(), Some("ctrl+ "));
        assert_eq!(normalize_combo("Ctrl+Space").as_deref(), Some("ctrl+ "));
        assert_eq!(
            normalize_combo("meta+shift+ ").as_deref(),
            Some("meta+shift+ ")
        );
        assert_eq!(normalize_combo(" meta+p ").as_deref(), Some("meta+p"));
        assert_eq!(
            normalize_combo("ctrl+LaunchApplication1").as_deref(),
            Some("ctrl+launchapplication1")
        );
        assert_eq!(
            normalize_combo("alt+MediaTrackPrevious").as_deref(),
            Some("alt+mediatrackprevious")
        );
    }

    #[test]
    fn theme_id_shape() {
        assert!(is_theme_id("tyba-dark"));
        assert!(is_theme_id("nord2"));
        assert!(!is_theme_id(""));
        assert!(!is_theme_id("Nord"));
        assert!(!is_theme_id("a/b"));
        assert!(!is_theme_id(&"a".repeat(65)));
    }

    #[test]
    fn key_actions_match_the_front() {
        let src = include_str!("../../../src/lib/keys.ts");
        let start = src.find("export const KEY_ACTIONS").unwrap();
        let body = &src[start..];
        let body = &body[body.find("= [").unwrap() + 3..body.find("];").unwrap()];
        let front: Vec<&str> = body
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(front, KEY_ACTIONS);
    }

    #[test]
    fn every_field_is_a_file_key() {
        for f in FIELDS {
            assert!(FILE_KEYS.contains(&f.key));
        }
    }
}
