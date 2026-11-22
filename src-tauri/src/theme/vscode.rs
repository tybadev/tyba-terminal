//! Tema de cor do VS Code (`.json`, aceita comentários e vírgula sobrando)
//! virando tema do TYBA: só a paleta do terminal. O resto do arquivo (cores do
//! editor, `tokenColors`) não tem par no TYBA e fica de fora.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{dark_terminal, light_terminal, TerminalPalette, ThemeBase, ThemeError, ThemeFile};

/// As 16 chaves ANSI do terminal do VS Code, na ordem do xterm.
const ANSI_KEYS: [&str; 16] = [
    "terminal.ansiBlack",
    "terminal.ansiRed",
    "terminal.ansiGreen",
    "terminal.ansiYellow",
    "terminal.ansiBlue",
    "terminal.ansiMagenta",
    "terminal.ansiCyan",
    "terminal.ansiWhite",
    "terminal.ansiBrightBlack",
    "terminal.ansiBrightRed",
    "terminal.ansiBrightGreen",
    "terminal.ansiBrightYellow",
    "terminal.ansiBrightBlue",
    "terminal.ansiBrightMagenta",
    "terminal.ansiBrightCyan",
    "terminal.ansiBrightWhite",
];

/// O arquivo parece um tema do VS Code: tem `colors` ou `tokenColors`.
pub fn looks_like_vscode(json: &str) -> bool {
    serde_json::from_str::<Value>(&strip_jsonc(json))
        .ok()
        .and_then(|v| v.as_object().cloned())
        .is_some_and(|o| o.contains_key("colors") || o.contains_key("tokenColors"))
}

/// Converte o tema do VS Code. `fallback_name` (o nome do arquivo) vale
/// quando o tema não traz `name`. Cor que falta vem do TYBA do mesmo lado.
pub fn from_vscode(json: &str, fallback_name: &str) -> Result<ThemeFile, ThemeError> {
    let root: Value = serde_json::from_str(&strip_jsonc(json))?;
    let empty = serde_json::Map::new();
    let colors = root
        .get("colors")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let color = |key: &str| {
        colors
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| super::is_hex_color(c))
            .map(str::to_owned)
    };

    let background = color("terminal.background").or_else(|| color("editor.background"));
    let base = match root.get("type").and_then(Value::as_str) {
        Some("light") | Some("hc-light") => ThemeBase::Light,
        Some(_) => ThemeBase::Dark,
        None => match background.as_deref().and_then(luminance) {
            Some(l) if l > 0.5 => ThemeBase::Light,
            _ => ThemeBase::Dark,
        },
    };
    let fallback = match base {
        ThemeBase::Dark => dark_terminal(),
        ThemeBase::Light => light_terminal(),
    };
    let foreground = color("terminal.foreground").or_else(|| color("editor.foreground"));
    let cursor = color("terminalCursor.foreground")
        .or_else(|| color("editorCursor.foreground"))
        .or_else(|| foreground.clone());
    let ansi = ANSI_KEYS
        .iter()
        .zip(&fallback.ansi)
        .map(|(key, default)| color(key).unwrap_or_else(|| default.clone()))
        .collect();

    let name = root
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback_name)
        .chars()
        .take(super::MAX_NAME_LEN)
        .collect::<String>();

    let file = ThemeFile {
        name,
        base,
        ui: BTreeMap::new(),
        terminal: Some(TerminalPalette {
            background: background.unwrap_or(fallback.background),
            foreground: foreground.unwrap_or(fallback.foreground),
            cursor: cursor.unwrap_or(fallback.cursor),
            cursor_accent: None,
            selection_background: color("terminal.selectionBackground")
                .or_else(|| color("editor.selectionBackground")),
            ansi,
        }),
    };
    super::validate(&file)?;
    Ok(file)
}

/// Luminância relativa (0 a 1) de um `#rgb`/`#rrggbb`/`#rrggbbaa`.
fn luminance(hex: &str) -> Option<f64> {
    let h = hex.strip_prefix('#')?;
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => h[..6].to_string(),
        _ => return None,
    };
    let channel = |i: usize| {
        u8::from_str_radix(&full[i..i + 2], 16)
            .ok()
            .map(|v| v as f64 / 255.0)
    };
    Some(0.2126 * channel(0)? + 0.7152 * channel(2)? + 0.0722 * channel(4)?)
}

/// Tira comentários (`//` e `/* */`) e vírgula antes de `}`/`]`, sem mexer
/// no que está dentro de string: o formato em que o VS Code grava temas.
fn strip_jsonc(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            '}' | ']' => {
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK: &str = r##"{
        // Tema sintético de teste.
        "name": "Noite Teste",
        "type": "dark",
        "colors": {
            "editor.background": "#1e1e2e",
            "editor.foreground": "#cdd6f4",
            "terminal.ansiRed": "#f38ba8",
            "terminal.ansiGreen": "#a6e3a1", /* comentário no meio */
            "terminalCursor.foreground": "#f5e0dc",
            "editor.selectionBackground": "#585b7066",
        },
        "tokenColors": [],
    }"##;

    #[test]
    fn converts_terminal_colors_and_falls_back_to_editor() {
        let file = from_vscode(DARK, "arquivo").unwrap();
        assert_eq!(file.name, "Noite Teste");
        assert_eq!(file.base, ThemeBase::Dark);
        assert!(file.ui.is_empty());
        let term = file.terminal.unwrap();
        assert_eq!(term.background, "#1e1e2e");
        assert_eq!(term.foreground, "#cdd6f4");
        assert_eq!(term.cursor, "#f5e0dc");
        assert_eq!(term.selection_background.as_deref(), Some("#585b7066"));
        assert_eq!(term.ansi[1], "#f38ba8");
        assert_eq!(term.ansi[2], "#a6e3a1");
        // O que o tema não traz vem do TYBA escuro.
        assert_eq!(term.ansi[4], dark_terminal().ansi[4]);
    }

    #[test]
    fn light_by_type_or_by_background() {
        let typed = r##"{ "type": "light", "colors": {} }"##;
        assert_eq!(from_vscode(typed, "a").unwrap().base, ThemeBase::Light);
        let untyped = r##"{ "colors": { "editor.background": "#fafafa" } }"##;
        let file = from_vscode(untyped, "Claro do arquivo").unwrap();
        assert_eq!(file.base, ThemeBase::Light);
        assert_eq!(file.name, "Claro do arquivo");
    }

    #[test]
    fn ignores_invalid_colors() {
        let json =
            r##"{ "colors": { "terminal.background": "red", "terminal.ansiRed": "#zzz" } }"##;
        let term = from_vscode(json, "x").unwrap().terminal.unwrap();
        assert_eq!(term.background, dark_terminal().background);
        assert_eq!(term.ansi[1], dark_terminal().ansi[1]);
    }

    #[test]
    fn detects_vscode_but_not_tyba_files() {
        assert!(looks_like_vscode(DARK));
        assert!(!looks_like_vscode(r#"{ "name": "Nord", "base": "dark" }"#));
        assert!(!looks_like_vscode("não é json"));
    }

    #[test]
    fn strip_jsonc_keeps_slashes_inside_strings() {
        let json = r#"{ "url": "https://x//y", /* c */ "a": [1, 2,], }"#;
        let v: Value = serde_json::from_str(&strip_jsonc(json)).unwrap();
        assert_eq!(v["url"], "https://x//y");
        assert_eq!(v["a"], serde_json::json!([1, 2]));
    }
}
