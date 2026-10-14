//! `~/.tyba/config.toml`: as preferências do usuário num arquivo legível,
//! portátil entre máquinas e sistemas.
//!
//! **O arquivo é a fonte da verdade** das chaves em [`schema::FILE_KEYS`]: a UI
//! grava nele (preservando comentários, ver [`edit`]) e a edição na mão é
//! observada e aplicada ao vivo (ver [`watch`]). O SQLite só serve uma vez, na
//! migração que gera o arquivo; depois disso essas chaves não são lidas de lá.
//! O webview nunca lê o arquivo: recebe os valores do core, como sempre.
//!
//! Tudo que vem do disco é entrada não confiável — ver [`fsguard`] e
//! `docs`/vault `config-tyba/security`.

pub mod edit;
pub mod fsguard;
pub mod schema;
pub mod watch;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::session::store::Store;
use schema::Values;

pub const FILE_NAME: &str = "config.toml";
pub const THEMES_DIR: &str = "themes";
/// Payload: as prefs `pref.*` que moram no arquivo, já validadas.
pub const EVENT_PREFS: &str = "config://prefs";
/// Payload: lista de avisos em pt-BR para o toast.
pub const EVENT_WARNINGS: &str = "config://warnings";
pub const MAX_BYTES: u64 = 64 * 1024;
/// Só em build de desenvolvimento: aponta a configuração para outra pasta, para
/// rodar o app sem tocar no `~/.tyba/` de verdade.
pub const ENV_DIR: &str = "TYBA_CONFIG_DIR";
const MIGRATED_KEY: &str = "config.file_migrated";
const MAX_THEME_BYTES: u64 = 256 * 1024;

const TEMPLATE: &str = r#"# Configuração do TYBA.
#
# Este arquivo é a fonte da verdade: o que você muda na interface é gravado
# aqui, e o que você edita aqui vale na hora, sem reiniciar. Chave inválida ou
# desconhecida é ignorada com um aviso — o arquivo nunca é apagado.
#
# Ações vermelhas (git push, gh pr create, sudo, rede, rm -rf…) nunca são
# configuráveis. Não guarde tokens nem senhas aqui: eles ficam no Keychain.

[theme]
# "dark", "light" ou "system" (segue o sistema operacional)
mode = "dark"
# Tema de cada modo. Embutidos: tyba-dark, tyba-light, dracula, nord,
# solarized-dark, solarized-light… Temas importados ficam em
# ~/.tyba/themes/<id>.json.
dark = "tyba-dark"
light = "tyba-light"

[font]
# Fonte da interface: "mono" (JetBrains Mono) ou "grotesk" (Space Grotesk)
ui = "mono"
# Tamanho da fonte do terminal, de 10 a 20
terminal_size = 13

# Atalhos: ação = "modificadores+tecla", com meta, ctrl, alt e shift.
# Ex.: paletteActions = "meta+p". O que não estiver aqui usa o padrão.
[keybindings]
"#;

pub type SharedConfig = Arc<ConfigManager>;

pub fn is_file_key(key: &str) -> bool {
    schema::FILE_KEYS.contains(&key)
}

/// `~/.tyba/` em todo SO — inclusive Linux, sem XDG (decisão do dono).
pub fn root_dir(home: Option<&Path>) -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os(ENV_DIR).map(PathBuf::from) {
        if dir.is_absolute() {
            return Some(dir);
        }
    }
    home.map(|h| h.join(".tyba"))
}

/// O que mudou num reload — quem emite olha isto para não emitir à toa.
#[derive(Debug, Default, PartialEq)]
pub struct Changes {
    pub theme: bool,
    pub prefs: bool,
    /// `Some` só quando a lista de avisos mudou: salvar de novo o mesmo arquivo
    /// com o mesmo erro não vira outro toast.
    pub warnings: Option<Vec<String>>,
}

#[derive(Default)]
struct State {
    values: Values,
    warnings: Vec<String>,
    /// Último texto aplicado: o watcher acorda também com a gravação do próprio
    /// app, e texto igual não é mudança.
    last_text: Option<String>,
}

pub struct ConfigManager {
    root: PathBuf,
    path: PathBuf,
    /// Raiz recusada (de outro usuário, gravável por outros…): nada se lê nem
    /// se grava nela, e os valores caem no SQLite (se não migrou) ou no padrão.
    root_error: Option<String>,
    state: Mutex<State>,
}

fn diff(old: &Values, new: &Values) -> (bool, bool) {
    let mut theme = false;
    let mut prefs = false;
    for key in schema::FILE_KEYS {
        if old.get(key) != new.get(key) {
            if key.starts_with("pref.") {
                prefs = true;
            } else {
                theme = true;
            }
        }
    }
    (theme, prefs)
}

/// Só as entradas que o arquivo aceita: um atalho antigo estranho no SQLite não
/// pode derrubar a migração dos outros.
fn sanitize_bindings(json: &str) -> Option<String> {
    let map: BTreeMap<String, String> = serde_json::from_str(json).ok()?;
    let clean: BTreeMap<String, String> = map
        .into_iter()
        .filter(|(a, _)| schema::KEY_ACTIONS.contains(&a.as_str()))
        .filter_map(|(a, c)| schema::normalize_combo(&c).map(|c| (a, c)))
        .collect();
    (!clean.is_empty()).then(|| serde_json::to_string(&clean).ok())?
}

/// Texto do arquivo depois de trazer os valores do SQLite. Arquivo novo nasce
/// do modelo comentado com os valores por cima; arquivo que já existe (outra
/// máquina, ou o `[sandbox]` escrito à mão) ganha só o que ainda não diz — o
/// que está escrito vence.
pub fn migrate_text(existing: Option<&str>, store: &Store) -> Result<String, String> {
    let fresh = existing.is_none();
    let mut out = existing.unwrap_or(TEMPLATE).to_string();
    let present = schema::parse(&out)?.values;
    for key in schema::FILE_KEYS {
        if !fresh && present.contains_key(key) {
            continue;
        }
        let Some(mut value) = store.get_setting(key).ok().flatten() else {
            continue;
        };
        if *key == schema::KEY_BINDINGS {
            match sanitize_bindings(&value) {
                Some(v) => value = v,
                None => continue,
            }
        }
        // Valor velho fora do tipo fica para trás; o padrão do arquivo vale.
        if let Ok(next) = edit::set(&out, key, &value) {
            out = next;
        }
    }
    Ok(out)
}

/// Move os temas importados da pasta antiga (config do app) para
/// `~/.tyba/themes/`. Nada se perde: com nome já ocupado no destino, o antigo
/// fica onde está.
pub fn migrate_themes(old: &Path, new: &Path) -> usize {
    let Ok(entries) = fs::read_dir(old) else {
        return 0;
    };
    let _ = fs::create_dir_all(new);
    let mut moved = 0;
    for entry in entries.flatten() {
        let src = entry.path();
        let Ok(meta) = fs::symlink_metadata(&src) else {
            continue;
        };
        if !meta.is_file()
            || meta.len() > MAX_THEME_BYTES
            || src.extension().and_then(|e| e.to_str()) != Some("json")
        {
            continue;
        }
        let dst = new.join(entry.file_name());
        // Qualquer entrada ocupa o nome — inclusive symlink quebrado, que o
        // `exists()` daria como livre e a cópia seguiria.
        if fs::symlink_metadata(&dst).is_ok() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&src) else {
            continue;
        };
        if content.len() as u64 > MAX_THEME_BYTES {
            continue;
        }
        // `0600` como todo arquivo de `~/.tyba/`; o antigo só sai depois.
        if fsguard::write_atomic(new, &dst, &content).is_ok() {
            let _ = fs::remove_file(&src);
            moved += 1;
        }
    }
    moved
}

impl ConfigManager {
    /// Abre (criando a pasta e, na primeira vez, o arquivo) e carrega. Nunca
    /// falha: o que der errado vira aviso e o app segue com o padrão.
    pub fn open(root: PathBuf, store: &Store) -> Self {
        let path = root.join(FILE_NAME);
        let root_error = fsguard::ensure_root(&root).err();
        let mgr = Self {
            root,
            path,
            root_error,
            state: Mutex::new(State::default()),
        };
        let mut warnings = Vec::new();
        if mgr.usable() {
            if let Err(e) = mgr.migrate(store) {
                warnings.push(e);
            }
        }
        // Com a raiz recusada, o load avisa o motivo (o `read` devolve o erro).
        mgr.load();
        // Arquivo ilegível falha na migração e de novo no load, com a mesma
        // causa: um aviso basta, e o do load diz o que o app fez.
        if !mgr.state.lock().warnings.is_empty() {
            warnings.clear();
        }
        let migrated = store.get_setting(MIGRATED_KEY).ok().flatten().is_some();
        let mut state = mgr.state.lock();
        if !migrated && state.last_text.is_none() {
            // A migração não rodou (arquivo ilegível): até ela rodar, o SQLite
            // ainda é a melhor resposta — nada some por um arquivo quebrado.
            for key in schema::FILE_KEYS {
                if let Some(v) = store.get_setting(key).ok().flatten() {
                    state.values.insert(key, v);
                }
            }
        }
        warnings.append(&mut state.warnings);
        warnings.dedup();
        state.warnings = warnings;
        drop(state);
        mgr
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.root.join(THEMES_DIR)
    }

    /// `false` quando a raiz foi recusada: nem watcher nem migração de temas.
    pub fn usable(&self) -> bool {
        self.root_error.is_none()
    }

    fn read(&self) -> Result<Option<String>, String> {
        if let Some(e) = &self.root_error {
            return Err(e.clone());
        }
        fsguard::read_capped(&self.root, &self.path, MAX_BYTES)
    }

    fn migrate(&self, store: &Store) -> Result<(), String> {
        if store.get_setting(MIGRATED_KEY).ok().flatten().is_some() {
            return Ok(());
        }
        let existing = self.read()?;
        let out = migrate_text(existing.as_deref(), store)
            .map_err(|e| format!("config.toml: {e}. A migração espera o arquivo ser corrigido."))?;
        if existing.as_deref() != Some(out.as_str()) {
            fsguard::write_atomic(&self.root, &self.path, &out)?;
        }
        store
            .set_setting(MIGRATED_KEY, "1")
            .map_err(|e| e.to_string())
    }

    fn apply(
        state: &mut State,
        text: Option<String>,
        warnings: Vec<String>,
        values: Option<Values>,
    ) -> Changes {
        let mut changes = Changes::default();
        if let Some(values) = values {
            (changes.theme, changes.prefs) = diff(&state.values, &values);
            state.values = values;
        }
        if text.is_some() {
            state.last_text = text;
        }
        if warnings != state.warnings {
            changes.warnings = Some(warnings.clone());
            state.warnings = warnings;
        }
        changes
    }

    /// Relê o arquivo. Texto igual ao último aplicado não muda nada; TOML
    /// quebrado mantém o último estado bom e avisa.
    pub fn load(&self) -> Changes {
        let mut state = self.state.lock();
        match self.read() {
            Ok(Some(text)) => {
                if state.last_text.as_deref() == Some(text.as_str()) {
                    return Changes::default();
                }
                match schema::parse(&text) {
                    Ok(parsed) => {
                        Self::apply(&mut state, Some(text), parsed.warnings, Some(parsed.values))
                    }
                    Err(e) => {
                        let w = vec![format!(
                            "config.toml: {e}. Mantive a configuração anterior."
                        )];
                        Self::apply(&mut state, None, w, None)
                    }
                }
            }
            // Arquivo apagado: volta tudo ao padrão; a próxima mudança na UI
            // cria de novo.
            Ok(None) => {
                state.last_text = None;
                Self::apply(&mut state, None, Vec::new(), Some(Values::new()))
            }
            Err(e) => Self::apply(&mut state, None, vec![e], None),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.state.lock().values.get(key).cloned()
    }

    /// As `pref.*` do arquivo, no formato que o front já lê do `boot_snapshot`.
    pub fn prefs(&self) -> HashMap<String, String> {
        self.state
            .lock()
            .values
            .iter()
            .filter(|(k, _)| k.starts_with("pref."))
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    pub fn warnings(&self) -> Vec<String> {
        self.state.lock().warnings.clone()
    }

    /// Mudança vinda da UI: lê o disco (pode ter edição na mão ainda não
    /// aplicada), muda só a chave, grava e aplica. Valor inválido ou arquivo
    /// quebrado: erro, e o disco fica como estava.
    pub fn set(&self, key: &str, value: &str) -> Result<Changes, String> {
        let mut state = self.state.lock();
        // Arquivo apagado renasce do modelo comentado, não vazio.
        let text = self.read()?.unwrap_or_else(|| TEMPLATE.to_string());
        let out = edit::set(&text, key, value)?;
        let parsed = schema::parse(&out)?;
        if out != text {
            fsguard::write_atomic(&self.root, &self.path, &out)?;
        }
        Ok(Self::apply(
            &mut state,
            Some(out),
            parsed.warnings,
            Some(parsed.values),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::schema::*;
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, Store) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".tyba");
        (dir, root, Store::open_in_memory().unwrap())
    }

    #[test]
    fn first_run_generates_the_commented_file_from_sqlite() {
        let (_d, root, store) = setup();
        store.set_setting(KEY_THEME_MODE, "light").unwrap();
        store.set_setting(KEY_THEME_DARK, "dracula").unwrap();
        store.set_setting(KEY_FONT_SIZE, "15").unwrap();
        store.set_setting(KEY_UI_FONT, "grotesk").unwrap();
        store
            .set_setting(
                KEY_BINDINGS,
                r#"{"copy":"meta+c","bogus":"meta+x","paste":"v"}"#,
            )
            .unwrap();
        store.set_setting("pref.toolbar", "{}").unwrap();

        let cfg = ConfigManager::open(root.clone(), &store);
        let text = fs::read_to_string(root.join(FILE_NAME)).unwrap();
        assert!(text.starts_with("# Configuração do TYBA."), "{text}");
        assert!(text.contains("mode = \"light\""), "{text}");
        assert!(text.contains("dark = \"dracula\""), "{text}");
        assert!(text.contains("terminal_size = 15"), "{text}");
        assert!(text.contains("copy = \"meta+c\""), "{text}");
        assert!(
            !text.contains("bogus") && !text.contains("toolbar"),
            "{text}"
        );
        assert!(cfg.warnings().is_empty(), "{:?}", cfg.warnings());
        assert_eq!(cfg.get(KEY_THEME_MODE).as_deref(), Some("light"));
        assert_eq!(cfg.get(KEY_THEME_LIGHT).as_deref(), Some("tyba-light"));
        assert_eq!(cfg.prefs().len(), 3);
    }

    #[test]
    fn fresh_install_gets_the_template_defaults() {
        let (_d, root, store) = setup();
        let cfg = ConfigManager::open(root.clone(), &store);
        assert_eq!(fs::read_to_string(root.join(FILE_NAME)).unwrap(), TEMPLATE);
        assert_eq!(cfg.get(KEY_THEME_MODE).as_deref(), Some("dark"));
        assert_eq!(cfg.get(KEY_BINDINGS), None);
    }

    #[test]
    fn existing_file_wins_and_only_gains_what_it_lacks() {
        let (_d, root, store) = setup();
        fs::create_dir_all(&root).unwrap();
        let mine =
            "# do outro mac\n[sandbox]\nread_allow = [\"~/.kube\"]\n\n[theme]\nmode = \"system\"\n";
        fs::write(root.join(FILE_NAME), mine).unwrap();
        store.set_setting(KEY_THEME_MODE, "dark").unwrap();
        store.set_setting(KEY_THEME_DARK, "nord").unwrap();

        let cfg = ConfigManager::open(root.clone(), &store);
        let text = fs::read_to_string(root.join(FILE_NAME)).unwrap();
        assert!(text.starts_with(mine), "{text}");
        assert_eq!(cfg.get(KEY_THEME_MODE).as_deref(), Some("system"));
        assert_eq!(cfg.get(KEY_THEME_DARK).as_deref(), Some("nord"));
    }

    #[test]
    fn migration_runs_once_then_sqlite_is_never_read_again() {
        let (_d, root, store) = setup();
        drop(ConfigManager::open(root.clone(), &store));
        fs::write(root.join(FILE_NAME), "").unwrap();
        store.set_setting(KEY_THEME_MODE, "light").unwrap();
        let cfg = ConfigManager::open(root.clone(), &store);
        assert_eq!(cfg.get(KEY_THEME_MODE), None);
        assert_eq!(fs::read_to_string(root.join(FILE_NAME)).unwrap(), "");
    }

    #[test]
    fn broken_file_is_not_overwritten_and_sqlite_holds_until_fixed() {
        let (_d, root, store) = setup();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(FILE_NAME), "[theme\n").unwrap();
        store.set_setting(KEY_THEME_MODE, "light").unwrap();

        let cfg = ConfigManager::open(root.clone(), &store);
        assert_eq!(
            fs::read_to_string(root.join(FILE_NAME)).unwrap(),
            "[theme\n"
        );
        assert_eq!(cfg.get(KEY_THEME_MODE).as_deref(), Some("light"));
        assert_eq!(cfg.warnings().len(), 1, "{:?}", cfg.warnings());
        assert!(cfg.set(KEY_THEME_MODE, "dark").is_err());
        assert_eq!(
            fs::read_to_string(root.join(FILE_NAME)).unwrap(),
            "[theme\n"
        );

        // corrigido: a próxima abertura migra e o SQLite sai de cena
        fs::write(root.join(FILE_NAME), "# ok\n").unwrap();
        let cfg = ConfigManager::open(root.clone(), &store);
        assert!(cfg.warnings().is_empty(), "{:?}", cfg.warnings());
        assert_eq!(cfg.get(KEY_THEME_MODE).as_deref(), Some("light"));
        let text = fs::read_to_string(root.join(FILE_NAME)).unwrap();
        assert!(text.starts_with("# ok\n"), "{text}");
    }

    #[test]
    fn set_writes_the_file_and_reports_what_changed() {
        let (_d, root, store) = setup();
        let cfg = ConfigManager::open(root.clone(), &store);
        let ch = cfg.set(KEY_THEME_MODE, "light").unwrap();
        assert_eq!(
            ch,
            Changes {
                theme: true,
                prefs: false,
                warnings: None
            }
        );
        let ch = cfg.set(KEY_FONT_SIZE, "16").unwrap();
        assert_eq!(
            ch,
            Changes {
                theme: false,
                prefs: true,
                warnings: None
            }
        );
        let text = fs::read_to_string(root.join(FILE_NAME)).unwrap();
        assert!(text.contains("mode = \"light\"") && text.contains("terminal_size = 16"));
        assert!(
            text.contains("# Tamanho da fonte do terminal"),
            "comentário preservado"
        );
        assert!(cfg.set(KEY_FONT_SIZE, "50").is_err());
        assert_eq!(cfg.get(KEY_FONT_SIZE).as_deref(), Some("16"));
    }

    #[test]
    fn set_keeps_a_hand_edit_the_watcher_has_not_applied_yet() {
        let (_d, root, store) = setup();
        let cfg = ConfigManager::open(root.clone(), &store);
        let path = root.join(FILE_NAME);
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("ui = \"mono\"", "ui = \"grotesk\"");
        fs::write(&path, &edited).unwrap();
        cfg.set(KEY_THEME_MODE, "system").unwrap();
        assert_eq!(cfg.get(KEY_UI_FONT).as_deref(), Some("grotesk"));
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("ui = \"grotesk\""));
    }

    #[test]
    fn load_after_own_write_is_a_no_op() {
        let (_d, root, store) = setup();
        let cfg = ConfigManager::open(root, &store);
        cfg.set(KEY_THEME_MODE, "light").unwrap();
        assert_eq!(cfg.load(), Changes::default());
    }

    #[test]
    fn hand_edits_are_applied_on_load_with_warnings_only_when_they_change() {
        let (_d, root, store) = setup();
        let cfg = ConfigManager::open(root.clone(), &store);
        let path = root.join(FILE_NAME);
        let base = fs::read_to_string(&path).unwrap();

        fs::write(
            &path,
            base.replace("mode = \"dark\"", "mode = \"light\"\nfoo = 1"),
        )
        .unwrap();
        let ch = cfg.load();
        assert!(ch.theme && !ch.prefs);
        assert_eq!(ch.warnings.as_ref().map(Vec::len), Some(1));

        // mesmo aviso, outra edição: sem toast novo
        fs::write(
            &path,
            base.replace("mode = \"dark\"", "mode = \"system\"\nfoo = 1"),
        )
        .unwrap();
        assert_eq!(cfg.load().warnings, None);

        // quebrado: mantém o estado, avisa
        fs::write(&path, "[theme\n").unwrap();
        let ch = cfg.load();
        assert!(!ch.theme);
        assert!(ch.warnings.unwrap()[0].contains("Mantive a configuração anterior"));
        assert_eq!(cfg.get(KEY_THEME_MODE).as_deref(), Some("system"));

        // consertado: aviso some (lista vazia é mudança)
        fs::write(&path, &base).unwrap();
        assert_eq!(cfg.load().warnings, Some(vec![]));
    }

    #[test]
    fn deleted_file_falls_back_to_defaults() {
        let (_d, root, store) = setup();
        let cfg = ConfigManager::open(root.clone(), &store);
        fs::remove_file(root.join(FILE_NAME)).unwrap();
        let ch = cfg.load();
        assert!(ch.theme && ch.prefs);
        assert_eq!(cfg.get(KEY_THEME_MODE), None);
        cfg.set(KEY_THEME_MODE, "light").unwrap();
        let text = fs::read_to_string(root.join(FILE_NAME)).unwrap();
        assert!(text.starts_with("# Configuração do TYBA."), "{text}");
        assert!(text.contains("mode = \"light\""), "{text}");
        assert!(text.contains("# Tamanho da fonte do terminal"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn refused_root_is_neither_read_nor_written() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, root, store) = setup();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(FILE_NAME), "[theme]\nmode = \"light\"\n").unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();

        let cfg = ConfigManager::open(root.clone(), &store);
        assert!(!cfg.usable());
        assert_eq!(cfg.get(KEY_THEME_MODE), None);
        assert_eq!(cfg.warnings().len(), 1, "{:?}", cfg.warnings());
        assert!(cfg.warnings()[0].contains("outros usuários"));
        assert!(cfg.set(KEY_THEME_MODE, "dark").is_err());
        assert_eq!(
            fs::read_to_string(root.join(FILE_NAME)).unwrap(),
            "[theme]\nmode = \"light\"\n"
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn themes_move_without_losing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("nord.json"), "{}").unwrap();
        fs::write(old.join("taken.json"), "old").unwrap();
        fs::write(new.join("taken.json"), "new").unwrap();
        fs::write(old.join("notes.txt"), "x").unwrap();

        assert_eq!(migrate_themes(&old, &new), 1);
        assert!(new.join("nord.json").is_file() && !old.join("nord.json").exists());
        assert_eq!(fs::read_to_string(old.join("taken.json")).unwrap(), "old");
        assert_eq!(fs::read_to_string(new.join("taken.json")).unwrap(), "new");
        assert!(old.join("notes.txt").exists());
        assert_eq!(migrate_themes(&dir.path().join("nada"), &new), 0);
    }

    #[cfg(unix)]
    #[test]
    fn migrated_theme_is_private_and_a_planted_link_is_not_followed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("nord.json"), "{}").unwrap();
        fs::set_permissions(old.join("nord.json"), fs::Permissions::from_mode(0o664)).unwrap();
        fs::write(old.join("evil.json"), "{}").unwrap();
        let outside = dir.path().join("fora.json");
        std::os::unix::fs::symlink(&outside, new.join("evil.json")).unwrap();

        assert_eq!(migrate_themes(&old, &new), 1);
        let mode = fs::metadata(new.join("nord.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!outside.exists(), "a cópia seguiu o link quebrado");
        assert!(old.join("evil.json").exists());
    }

    #[test]
    fn root_dir_is_home_dot_tyba() {
        if std::env::var_os(ENV_DIR).is_none() {
            assert_eq!(
                root_dir(Some(Path::new("/h"))),
                Some(PathBuf::from("/h/.tyba"))
            );
        }
    }
}
