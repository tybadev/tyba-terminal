//! Gravação no `config.toml` que preserva o que o usuário escreveu.
//!
//! A UI muda uma chave; o resto do arquivo — comentários, ordem, espaçamento,
//! chaves que este módulo não conhece — sai byte a byte como entrou. Arquivo
//! com TOML quebrado não é reescrito: a edição falha e o arquivo fica.

use std::collections::BTreeMap;

use toml_edit::{Document, Item, Table, TableLike, Value};

use super::schema::{self, Kind, KEY_ACTIONS, KEY_BINDINGS, SECTION_BINDINGS};

fn section<'a>(doc: &'a mut Document, name: &str) -> Result<&'a mut dyn TableLike, String> {
    if !doc.as_table().contains_key(name) {
        // Comentário depois do último item mora no "trailing" do documento, que
        // sai sempre por último: a seção nova entraria acima dele. Ele passa a
        // ser o prefixo da seção, e a ordem do arquivo se mantém.
        let trailing = doc.trailing().as_str().unwrap_or("").to_string();
        let mut table = Table::new();
        if !trailing.trim().is_empty() {
            let sep = if trailing.ends_with("\n\n") { "" } else { "\n" };
            table.decor_mut().set_prefix(format!("{trailing}{sep}"));
            doc.set_trailing("");
        }
        doc.as_table_mut().insert(name, Item::Table(table));
    }
    let root = doc.as_table_mut();
    root.get_mut(name)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            format!("'{name}' no arquivo não é uma seção; corrija antes de mudar pela interface")
        })
}

/// Troca o valor mantendo a decoração (comentário no fim da linha, espaços) do
/// antigo. Valor igual não é tocado: `'dark'` com aspas simples continua assim.
fn put(table: &mut dyn TableLike, name: &str, mut value: Value) {
    // Troca o item no lugar: `insert` recriaria a chave, e o comentário acima
    // dela (que mora na decoração da chave) iria junto.
    if let Some(item) = table.get_mut(name) {
        if let Some(old) = item.as_value() {
            if same(old, &value) {
                return;
            }
            *value.decor_mut() = old.decor().clone();
        }
        *item = Item::Value(value);
        return;
    }
    table.insert(name, Item::Value(value));
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.value() == y.value(),
        (Value::Integer(x), Value::Integer(y)) => x.value() == y.value(),
        _ => false,
    }
}

fn set_bindings(doc: &mut Document, json: &str) -> Result<(), String> {
    let map: BTreeMap<String, String> =
        serde_json::from_str(json).map_err(|_| "atalhos em formato inválido".to_string())?;
    let mut clean = BTreeMap::new();
    for (action, combo) in &map {
        if !KEY_ACTIONS.contains(&action.as_str()) {
            return Err(format!("ação desconhecida: {action}"));
        }
        let combo = schema::normalize_combo(combo)
            .ok_or_else(|| format!("atalho inválido para {action}"))?;
        clean.insert(action.as_str(), combo);
    }
    let table = section(doc, SECTION_BINDINGS)?;
    // Some só o que é ação conhecida e saiu do mapa; chave escrita à mão que
    // o app não conhece fica onde está.
    let stale: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| KEY_ACTIONS.contains(&k.as_str()) && !clean.contains_key(k.as_str()))
        .collect();
    for k in stale {
        table.remove(&k);
    }
    for (action, combo) in clean {
        put(table, action, Value::from(combo));
    }
    Ok(())
}

/// Grava `key` (chave de `settings`) com `value` (no formato do SQLite) e
/// devolve o texto novo. Valida antes: valor fora do tipo nunca chega ao disco.
pub fn set(text: &str, key: &str, value: &str) -> Result<String, String> {
    let mut doc: Document = text
        .parse()
        .map_err(|_| "o config.toml tem TOML inválido; corrija o arquivo antes".to_string())?;
    if key == KEY_BINDINGS {
        set_bindings(&mut doc, value)?;
        return Ok(doc.to_string());
    }
    let field = schema::field_by_key(key).ok_or_else(|| format!("chave fora do arquivo: {key}"))?;
    let new = match field.kind {
        Kind::Int(min, max) => {
            let n: i64 = value
                .parse()
                .ok()
                .filter(|n| (min..=max).contains(n))
                .ok_or_else(|| format!("{key}: valor fora da faixa {min}–{max}"))?;
            Value::from(n)
        }
        Kind::Enum(options) if !options.contains(&value) => {
            return Err(format!("{key}: valor inválido"));
        }
        Kind::ThemeId if !schema::is_theme_id(value) => {
            return Err(format!("{key}: id de tema inválido"));
        }
        _ => Value::from(value),
    };
    put(section(&mut doc, field.section)?, field.name, new);
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{KEY_FONT_SIZE, KEY_THEME_DARK, KEY_THEME_MODE, KEY_UI_FONT};

    const COMMENTED: &str = r#"# meu config
# segunda linha

[theme]
# o modo
mode = "dark"   # comentário no fim
dark = 'tyba-dark'

[sandbox]
read_allow = ["~/.kube"] # leitura extra

[outra]
x = 1
"#;

    #[test]
    fn changes_only_the_value_and_keeps_every_comment() {
        let out = set(COMMENTED, KEY_THEME_MODE, "light").unwrap();
        assert_eq!(
            out,
            COMMENTED.replace("mode = \"dark\"   #", "mode = \"light\"   #")
        );
    }

    #[test]
    fn same_value_is_byte_identical() {
        assert_eq!(
            set(COMMENTED, KEY_THEME_DARK, "tyba-dark").unwrap(),
            COMMENTED
        );
    }

    #[test]
    fn missing_key_and_section_are_appended() {
        let out = set(COMMENTED, KEY_FONT_SIZE, "15").unwrap();
        assert!(out.starts_with(COMMENTED.trim_end()), "{out}");
        let p = schema::parse(&out).unwrap();
        assert_eq!(p.values[KEY_FONT_SIZE], "15");
        let out = set(&out, KEY_UI_FONT, "grotesk").unwrap();
        assert_eq!(out.matches("[font]").count(), 1, "{out}");
        assert_eq!(schema::parse(&out).unwrap().values[KEY_UI_FONT], "grotesk");
    }

    #[test]
    fn new_section_goes_below_a_comment_only_file() {
        let out = set("# só isto\n", KEY_THEME_MODE, "light").unwrap();
        assert_eq!(out, "# só isto\n\n[theme]\nmode = \"light\"\n");
        let text = "[theme]\nmode = \"dark\"\n# fim\n";
        let out = set(text, KEY_UI_FONT, "mono").unwrap();
        assert!(out.starts_with(text), "{out}");
    }

    #[test]
    fn works_on_an_empty_file() {
        let out = set("", KEY_THEME_MODE, "system").unwrap();
        assert_eq!(out, "[theme]\nmode = \"system\"\n");
    }

    #[test]
    fn refuses_invalid_values_before_touching_anything() {
        assert!(set(COMMENTED, KEY_THEME_MODE, "blue").is_err());
        assert!(set(COMMENTED, KEY_FONT_SIZE, "99").is_err());
        assert!(set(COMMENTED, KEY_FONT_SIZE, "abc").is_err());
        assert!(set(COMMENTED, KEY_THEME_DARK, "../x").is_err());
        assert!(set(COMMENTED, "pref.toolbar", "x").is_err());
    }

    #[test]
    fn broken_file_is_never_rewritten() {
        assert!(set("[theme\n", KEY_THEME_MODE, "dark").is_err());
    }

    #[test]
    fn section_with_wrong_shape_is_not_overwritten() {
        assert!(set("theme = \"dark\"\n", KEY_THEME_MODE, "light").is_err());
    }

    #[test]
    fn bindings_update_in_place_and_keep_unknown_lines() {
        let text = "[keybindings]\n# abre paleta\npaletteActions = \"meta+p\"\nnewTab = \"meta+t\"\nmeuAtalho = \"meta+j\"\n";
        let json = r#"{"paletteActions":"meta+k","copy":"meta+c"}"#;
        let out = set(text, KEY_BINDINGS, json).unwrap();
        assert!(
            out.contains("# abre paleta\npaletteActions = \"meta+k\""),
            "{out}"
        );
        assert!(!out.contains("newTab"), "{out}");
        assert!(out.contains("meuAtalho = \"meta+j\""), "{out}");
        assert!(out.contains("copy = \"meta+c\""), "{out}");
    }

    #[test]
    fn bindings_refuse_bad_input() {
        assert!(set("", KEY_BINDINGS, "nope").is_err());
        assert!(set("", KEY_BINDINGS, r#"{"launch":"meta+l"}"#).is_err());
        assert!(set("", KEY_BINDINGS, r#"{"copy":"c"}"#).is_err());
    }

    #[test]
    fn written_file_round_trips_through_the_parser() {
        let mut text = String::new();
        for (k, v) in [
            (KEY_THEME_MODE, "system"),
            (KEY_THEME_DARK, "dracula"),
            (KEY_UI_FONT, "mono"),
            (KEY_FONT_SIZE, "12"),
            (KEY_BINDINGS, r#"{"copy":"meta+c"}"#),
        ] {
            text = set(&text, k, v).unwrap();
        }
        let p = schema::parse(&text).unwrap();
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.values.len(), 5);
    }
}
