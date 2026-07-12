use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::session::store::Store;

mod schemes;

pub const EVENT_CHANGED: &str = "theme://changed";
pub const BUILTIN_DARK: &str = "tyba-dark";
pub const BUILTIN_LIGHT: &str = "tyba-light";

const KEY_MODE: &str = "theme.mode";
const KEY_SLOT_DARK: &str = "theme.slot.dark";
const KEY_SLOT_LIGHT: &str = "theme.slot.light";
const MAX_THEME_FILE_BYTES: u64 = 256 * 1024;
const MAX_NAME_LEN: usize = 64;

const UI_KEYS: &[&str] = &[
    "bg",
    "surface",
    "raised",
    "overlay",
    "border",
    "borderStrong",
    "sunken",
    "text",
    "textMuted",
    "textFaint",
    "textInverse",
    "primary",
    "primaryHover",
    "green",
    "lime",
    "amber",
    "magenta",
    "violet",
    "blue",
    "cyan",
    "red",
];

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("json inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("nome de tema inválido (1 a {MAX_NAME_LEN} caracteres)")]
    InvalidName,
    #[error("nome reservado: {0}")]
    ReservedName(String),
    #[error("cor inválida em '{0}': {1}")]
    InvalidColor(String, String),
    #[error("chave de UI desconhecida: {0}")]
    UnknownUiKey(String),
    #[error("paleta ansi precisa de 16 cores, veio {0}")]
    AnsiLength(usize),
    #[error("tema não encontrado: {0}")]
    NotFound(String),
    #[error("tema '{id}' tem base {actual}, esperado {expected}")]
    BaseMismatch {
        id: String,
        expected: String,
        actual: String,
    },
    #[error("arquivo excede {MAX_THEME_FILE_BYTES} bytes")]
    TooLarge,
    #[error("origem de importação inválida (não é um arquivo regular): {0}")]
    InvalidSource(String),
    #[error("store: {0}")]
    Store(#[from] crate::session::store::StoreError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeBase {
    Dark,
    Light,
}

impl ThemeBase {
    fn as_str(self) -> &'static str {
        match self {
            ThemeBase::Dark => "dark",
            ThemeBase::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Dark,
    Light,
    System,
}

impl ThemeMode {
    fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
            ThemeMode::System => "system",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "dark" => Some(ThemeMode::Dark),
            "light" => Some(ThemeMode::Light),
            "system" => Some(ThemeMode::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalPalette {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_background: Option<String>,
    pub ansi: Vec<String>,
}

impl TerminalPalette {
    fn colors(&self) -> Vec<(String, &str)> {
        let mut out: Vec<(String, &str)> = vec![
            ("terminal.background".into(), self.background.as_str()),
            ("terminal.foreground".into(), self.foreground.as_str()),
            ("terminal.cursor".into(), self.cursor.as_str()),
        ];
        if let Some(c) = &self.cursor_accent {
            out.push(("terminal.cursorAccent".into(), c.as_str()));
        }
        if let Some(c) = &self.selection_background {
            out.push(("terminal.selectionBackground".into(), c.as_str()));
        }
        for (i, c) in self.ansi.iter().enumerate() {
            out.push((format!("terminal.ansi[{i}]"), c.as_str()));
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    pub name: String,
    pub base: ThemeBase,
    #[serde(default)]
    pub ui: BTreeMap<String, String>,
    #[serde(default)]
    pub terminal: Option<TerminalPalette>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub base: ThemeBase,
    pub builtin: bool,
    pub ui: BTreeMap<String, String>,
    pub terminal: TerminalPalette,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeState {
    pub mode: ThemeMode,
    pub dark: Theme,
    pub light: Theme,
}

fn is_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn parse_theme(json: &str) -> Result<ThemeFile, ThemeError> {
    let theme: ThemeFile = serde_json::from_str(json)?;
    validate(&theme)?;
    Ok(theme)
}

fn validate(t: &ThemeFile) -> Result<(), ThemeError> {
    let name = t.name.trim();
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(ThemeError::InvalidName);
    }
    for (key, value) in &t.ui {
        if !UI_KEYS.contains(&key.as_str()) {
            return Err(ThemeError::UnknownUiKey(key.clone()));
        }
        if !is_hex_color(value) {
            return Err(ThemeError::InvalidColor(format!("ui.{key}"), value.clone()));
        }
    }
    if let Some(term) = &t.terminal {
        if term.ansi.len() != 16 {
            return Err(ThemeError::AnsiLength(term.ansi.len()));
        }
        for (label, color) in term.colors() {
            if !is_hex_color(color) {
                return Err(ThemeError::InvalidColor(label, color.to_string()));
            }
        }
    }
    Ok(())
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true;
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn ansi(colors: [&str; 16]) -> Vec<String> {
    colors.iter().map(|c| c.to_string()).collect()
}

fn dark_terminal() -> TerminalPalette {
    TerminalPalette {
        background: "#000000".into(),
        foreground: "#f2f2f5".into(),
        cursor: "#7cc544".into(),
        cursor_accent: Some("#000000".into()),
        selection_background: Some("#7cc5444d".into()),
        ansi: ansi([
            "#1a1a20", "#f0503c", "#7cc544", "#f5a93b", "#4c7df0", "#ec4899", "#2dd4bf", "#f2f2f5",
            "#6a6a76", "#f4715f", "#a8e05f", "#f5c93b", "#7da3f5", "#a78bfa", "#5eead4", "#ffffff",
        ]),
    }
}

fn light_terminal() -> TerminalPalette {
    TerminalPalette {
        background: "#fbfbfd".into(),
        foreground: "#17171c".into(),
        cursor: "#4e9622".into(),
        cursor_accent: Some("#fbfbfd".into()),
        selection_background: Some("#4e96224d".into()),
        ansi: ansi([
            "#2a2a31", "#d6402e", "#4e9622", "#c47c14", "#3b66d6", "#d33d84", "#0f9d8c", "#e9e9ee",
            "#6d6d78", "#e05a48", "#6fae2e", "#d69a2b", "#5d84e0", "#7a4ce8", "#16b3a0", "#ffffff",
        ]),
    }
}

fn is_reserved(id: &str) -> bool {
    id == BUILTIN_DARK || id == BUILTIN_LIGHT || schemes::IDS.contains(&id)
}

fn cached_schemes() -> &'static [Theme] {
    static CACHE: OnceLock<Vec<Theme>> = OnceLock::new();
    CACHE.get_or_init(schemes::all)
}

fn builtin_by_id(id: &str) -> Option<Theme> {
    match id {
        BUILTIN_DARK => Some(builtin(ThemeBase::Dark)),
        BUILTIN_LIGHT => Some(builtin(ThemeBase::Light)),
        _ => cached_schemes().iter().find(|t| t.id == id).cloned(),
    }
}

fn builtin(base: ThemeBase) -> Theme {
    match base {
        ThemeBase::Dark => Theme {
            id: BUILTIN_DARK.into(),
            name: "TYBA Dark".into(),
            base: ThemeBase::Dark,
            builtin: true,
            ui: BTreeMap::new(),
            terminal: dark_terminal(),
        },
        ThemeBase::Light => Theme {
            id: BUILTIN_LIGHT.into(),
            name: "TYBA Light".into(),
            base: ThemeBase::Light,
            builtin: true,
            ui: BTreeMap::new(),
            terminal: light_terminal(),
        },
    }
}

fn resolve(id: String, file: ThemeFile) -> Theme {
    let terminal = file.terminal.unwrap_or_else(|| match file.base {
        ThemeBase::Dark => dark_terminal(),
        ThemeBase::Light => light_terminal(),
    });
    Theme {
        id,
        name: file.name.trim().to_string(),
        base: file.base,
        builtin: false,
        ui: file.ui,
        terminal,
    }
}

pub struct ThemeManager {
    store: Arc<Store>,
    themes_dir: PathBuf,
}

pub type SharedThemes = Arc<ThemeManager>;

impl ThemeManager {
    pub fn new(store: Arc<Store>, themes_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&themes_dir);
        Self { store, themes_dir }
    }

    pub fn themes_dir(&self) -> &PathBuf {
        &self.themes_dir
    }

    pub fn list(&self) -> Vec<Theme> {
        let mut themes = vec![builtin(ThemeBase::Dark), builtin(ThemeBase::Light)];
        themes.extend(cached_schemes().iter().cloned());
        themes.extend(self.scan());
        themes
    }

    fn scan(&self) -> Vec<Theme> {
        let Ok(entries) = fs::read_dir(&self.themes_dir) else {
            return Vec::new();
        };
        let mut themes: Vec<Theme> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    return None;
                }
                let id = path.file_stem()?.to_str()?.to_string();
                if is_reserved(&id) {
                    return None;
                }
                let json = fs::read_to_string(&path).ok()?;
                match parse_theme(&json) {
                    Ok(file) => Some(resolve(id, file)),
                    Err(err) => {
                        eprintln!("tyba: tema ignorado ({}): {err}", path.display());
                        None
                    }
                }
            })
            .collect();
        themes.sort_by(|a, b| a.name.cmp(&b.name));
        themes
    }

    fn find(&self, id: &str) -> Option<Theme> {
        builtin_by_id(id).or_else(|| self.scan().into_iter().find(|t| t.id == id))
    }

    fn slot(&self, key: &str, base: ThemeBase) -> Theme {
        self.store
            .get_setting(key)
            .ok()
            .flatten()
            .and_then(|id| self.find(&id))
            .filter(|t| t.base == base)
            .unwrap_or_else(|| builtin(base))
    }

    pub fn state(&self) -> ThemeState {
        let mode = self
            .store
            .get_setting(KEY_MODE)
            .ok()
            .flatten()
            .and_then(|s| ThemeMode::parse(&s))
            .unwrap_or(ThemeMode::Dark);
        ThemeState {
            mode,
            dark: self.slot(KEY_SLOT_DARK, ThemeBase::Dark),
            light: self.slot(KEY_SLOT_LIGHT, ThemeBase::Light),
        }
    }

    pub fn set_mode(&self, app: &AppHandle, mode: ThemeMode) -> Result<(), ThemeError> {
        self.store.set_setting(KEY_MODE, mode.as_str())?;
        self.emit(app);
        Ok(())
    }

    pub fn set_slot(&self, app: &AppHandle, base: ThemeBase, id: &str) -> Result<(), ThemeError> {
        let theme = self
            .find(id)
            .ok_or_else(|| ThemeError::NotFound(id.to_string()))?;
        if theme.base != base {
            return Err(ThemeError::BaseMismatch {
                id: id.to_string(),
                expected: base.as_str().to_string(),
                actual: theme.base.as_str().to_string(),
            });
        }
        let key = match base {
            ThemeBase::Dark => KEY_SLOT_DARK,
            ThemeBase::Light => KEY_SLOT_LIGHT,
        };
        self.store.set_setting(key, id)?;
        self.emit(app);
        Ok(())
    }

    fn import_validated(&self, src: &str) -> Result<(String, ThemeFile), ThemeError> {
        let meta = fs::metadata(src)?;
        if !meta.is_file() {
            return Err(ThemeError::InvalidSource(src.to_string()));
        }
        if meta.len() > MAX_THEME_FILE_BYTES {
            return Err(ThemeError::TooLarge);
        }
        let json = fs::read_to_string(src)?;
        let file = parse_theme(&json)?;
        let id = slugify(&file.name);
        if id.is_empty() {
            return Err(ThemeError::InvalidName);
        }
        if is_reserved(&id) {
            return Err(ThemeError::ReservedName(id));
        }
        let canonical = serde_json::to_string_pretty(&file)?;
        fs::create_dir_all(&self.themes_dir)?;
        fs::write(self.themes_dir.join(format!("{id}.json")), canonical)?;
        Ok((id, file))
    }

    pub fn import(&self, app: &AppHandle, src: &str) -> Result<Theme, ThemeError> {
        let (id, file) = self.import_validated(src)?;
        self.emit(app);
        Ok(resolve(id, file))
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit(EVENT_CHANGED, self.state());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r##"{
        "name": "Dracula",
        "base": "dark",
        "ui": { "bg": "#282a36", "text": "#f8f8f2", "primary": "#50fa7b" },
        "terminal": {
            "background": "#282a36",
            "foreground": "#f8f8f2",
            "cursor": "#f8f8f2",
            "selectionBackground": "#44475a80",
            "ansi": ["#21222c", "#ff5555", "#50fa7b", "#f1fa8c",
                     "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
                     "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5",
                     "#d6acff", "#ff92df", "#a4ffff", "#ffffff"]
        }
    }"##;

    #[test]
    fn parses_a_valid_theme() {
        let theme = parse_theme(VALID).unwrap();
        assert_eq!(theme.name, "Dracula");
        assert_eq!(theme.base, ThemeBase::Dark);
        assert_eq!(theme.ui.len(), 3);
        assert_eq!(theme.terminal.unwrap().ansi.len(), 16);
    }

    #[test]
    fn accepts_theme_without_ui_or_terminal() {
        let theme = parse_theme(r#"{ "name": "Mínimo", "base": "light" }"#).unwrap();
        assert!(theme.ui.is_empty());
        assert!(theme.terminal.is_none());
    }

    #[test]
    fn rejects_invalid_hex_color() {
        let err =
            parse_theme(r##"{ "name": "x", "base": "dark", "ui": { "bg": "red" } }"##).unwrap_err();
        assert!(matches!(err, ThemeError::InvalidColor(key, _) if key == "ui.bg"));
    }

    #[test]
    fn rejects_unknown_ui_key() {
        let err = parse_theme(r##"{ "name": "x", "base": "dark", "ui": { "gradient": "#fff" } }"##)
            .unwrap_err();
        assert!(matches!(err, ThemeError::UnknownUiKey(key) if key == "gradient"));
    }

    #[test]
    fn rejects_wrong_ansi_length() {
        let err = parse_theme(
            r##"{ "name": "x", "base": "dark",
                 "terminal": { "background": "#000", "foreground": "#fff",
                               "cursor": "#fff", "ansi": ["#000"] } }"##,
        )
        .unwrap_err();
        assert!(matches!(err, ThemeError::AnsiLength(1)));
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        assert!(parse_theme(r#"{ "name": "x", "base": "dark", "hack": 1 }"#).is_err());
    }

    #[test]
    fn rejects_empty_or_giant_name() {
        assert!(matches!(
            parse_theme(r#"{ "name": "   ", "base": "dark" }"#).unwrap_err(),
            ThemeError::InvalidName
        ));
        let long = "a".repeat(65);
        assert!(matches!(
            parse_theme(&format!(r#"{{ "name": "{long}", "base": "dark" }}"#)).unwrap_err(),
            ThemeError::InvalidName
        ));
    }

    #[test]
    fn hex_color_accepts_3_6_8_digits_only() {
        assert!(is_hex_color("#fff"));
        assert!(is_hex_color("#F2f2F5"));
        assert!(is_hex_color("#7cc5444d"));
        assert!(!is_hex_color("#ffff"));
        assert!(!is_hex_color("fff"));
        assert!(!is_hex_color("#ggg"));
        assert!(!is_hex_color("rgb(0,0,0)"));
    }

    #[test]
    fn slugify_produces_safe_ids() {
        assert_eq!(slugify("Dracula Pro!"), "dracula-pro");
        assert_eq!(slugify("  Café com Leite  "), "caf-com-leite");
        assert_eq!(slugify("___"), "");
    }

    #[test]
    fn resolve_inherits_builtin_terminal_palette() {
        let file = parse_theme(r#"{ "name": "Só UI", "base": "light" }"#).unwrap();
        let theme = resolve("so-ui".into(), file);
        assert_eq!(theme.terminal, light_terminal());
    }

    #[test]
    fn builtin_palettes_are_valid() {
        for palette in [dark_terminal(), light_terminal()] {
            assert_eq!(palette.ansi.len(), 16);
            for (_, color) in palette.colors() {
                assert!(is_hex_color(color), "cor inválida: {color}");
            }
        }
    }

    fn manager() -> (ThemeManager, PathBuf) {
        let dir = std::env::temp_dir().join(format!("tyba-theme-test-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(Store::open_in_memory().unwrap());
        (ThemeManager::new(store, dir.clone()), dir)
    }

    #[test]
    fn list_includes_builtins_schemes_and_scanned_files() {
        let (mgr, dir) = manager();
        fs::write(dir.join("nord.json"), VALID).unwrap();
        fs::write(dir.join("broken.json"), "{ nope").unwrap();
        fs::write(dir.join("notes.txt"), "ignore me").unwrap();

        let themes = mgr.list();
        let ids: Vec<_> = themes.iter().map(|t| t.id.as_str()).collect();
        let mut expected = vec![BUILTIN_DARK, BUILTIN_LIGHT];
        expected.extend_from_slice(schemes::IDS);
        expected.push("nord");
        assert_eq!(ids, expected);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn classic_schemes_are_valid_builtins() {
        let all = schemes::all();
        assert_eq!(all.len(), schemes::IDS.len());
        assert_eq!(all.len(), 16);
        for (theme, id) in all.iter().zip(schemes::IDS) {
            assert_eq!(theme.id, *id);
            assert!(theme.builtin);
            assert!(theme.ui.is_empty());
            assert_eq!(theme.terminal.ansi.len(), 16);
            assert_eq!(
                theme.base == ThemeBase::Light,
                theme.id.ends_with("light"),
                "{}: convenção quebrada — todo tema claro termina em 'light'",
                theme.id
            );
            for (label, color) in theme.terminal.colors() {
                assert!(
                    is_hex_color(color),
                    "{}: cor inválida em {label}: {color}",
                    theme.id
                );
            }
        }
        let dark = all.iter().filter(|t| t.base == ThemeBase::Dark).count();
        let light = all.iter().filter(|t| t.base == ThemeBase::Light).count();
        assert_eq!((dark, light), (11, 5));
    }

    #[test]
    fn scan_skips_files_shadowing_scheme_ids() {
        let (mgr, dir) = manager();
        fs::write(dir.join("dracula.json"), VALID).unwrap();

        let themes = mgr.list();
        let draculas: Vec<_> = themes.iter().filter(|t| t.id == "dracula").collect();
        assert_eq!(draculas.len(), 1);
        assert!(draculas[0].builtin);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_rejects_classic_scheme_slug() {
        let (mgr, dir) = manager();
        let src = temp_source("scheme-slug");
        fs::write(&src, r#"{ "name": "Dracula", "base": "dark" }"#).unwrap();

        let err = mgr.import_validated(src.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ThemeError::ReservedName(id) if id == "dracula"));

        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn state_falls_back_to_builtins_when_slot_is_stale() {
        let (mgr, dir) = manager();
        mgr.store
            .set_setting("theme.slot.dark", "deleted-theme")
            .unwrap();
        let state = mgr.state();
        assert_eq!(state.mode, ThemeMode::Dark);
        assert_eq!(state.dark.id, BUILTIN_DARK);
        assert_eq!(state.light.id, BUILTIN_LIGHT);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn slot_ignores_theme_with_wrong_base() {
        let (mgr, dir) = manager();
        fs::write(dir.join("dracula.json"), VALID).unwrap();
        mgr.store
            .set_setting("theme.slot.light", "dracula")
            .unwrap();
        let state = mgr.state();
        assert_eq!(state.light.id, BUILTIN_LIGHT);
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_source(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tyba-theme-import-src-{}-{}",
            name,
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn import_rejects_reserved_name() {
        let (mgr, dir) = manager();
        let src = temp_source("reserved");
        fs::write(&src, r#"{ "name": "TYBA Dark", "base": "dark" }"#).unwrap();

        let err = mgr.import_validated(src.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ThemeError::ReservedName(id) if id == BUILTIN_DARK));

        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_rejects_oversized_file() {
        let (mgr, dir) = manager();
        let src = temp_source("oversized");
        fs::write(&src, "x".repeat(MAX_THEME_FILE_BYTES as usize + 1)).unwrap();

        let err = mgr.import_validated(src.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ThemeError::TooLarge));

        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_rejects_non_regular_file_source() {
        let (mgr, dir) = manager();
        // um diretório existe, mas não é um arquivo regular: metadata().len()
        // reportaria 0 e passaria pelo gate de tamanho se não checássemos is_file().
        let err = mgr.import_validated(dir.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ThemeError::InvalidSource(_)));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_rejects_missing_source() {
        let (mgr, dir) = manager();
        let src = temp_source("missing");

        let err = mgr.import_validated(src.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ThemeError::Io(_)));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_rejects_invalid_content() {
        let (mgr, dir) = manager();
        let src = temp_source("invalid");
        fs::write(
            &src,
            r#"{ "name": "x", "base": "dark", "ui": { "bg": "red" } }"#,
        )
        .unwrap();

        let err = mgr.import_validated(src.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ThemeError::InvalidColor(key, _) if key == "ui.bg"));

        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_writes_canonical_file_that_scan_picks_up() {
        let (mgr, dir) = manager();
        let src = temp_source("nord");
        fs::write(
            &src,
            r##"{ "name": "Nord", "base": "dark", "ui": { "bg": "#2e3440" } }"##,
        )
        .unwrap();

        let (id, file) = mgr.import_validated(src.to_str().unwrap()).unwrap();
        assert_eq!(id, "nord");
        assert_eq!(file.name, "Nord");
        assert!(dir.join("nord.json").is_file());

        let ids: Vec<_> = mgr.list().into_iter().map(|t| t.id).collect();
        assert!(ids.contains(&"nord".to_string()));

        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_overwrites_existing_file_with_same_slug() {
        let (mgr, dir) = manager();
        let src = temp_source("nord-variant");
        fs::write(
            &src,
            r##"{ "name": "Nord", "base": "dark", "ui": { "bg": "#000000" } }"##,
        )
        .unwrap();

        mgr.import_validated(src.to_str().unwrap()).unwrap();
        let on_disk = fs::read_to_string(dir.join("nord.json")).unwrap();
        assert!(on_disk.contains("#000000"));

        let ids: Vec<_> = mgr.list().into_iter().map(|t| t.id).collect();
        assert_eq!(ids.iter().filter(|id| id.as_str() == "nord").count(), 1);

        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(dir);
    }
}
