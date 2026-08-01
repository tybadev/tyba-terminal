//! Menu de aplicação (macOS).
//!
//! O Tauri monta um `Menu::default` quando ninguém define menu — é dele o
//! "Ajuda → Learn More" apontando para tauri.app. Aqui o menu é do produto.
//!
//! A estrutura vem do front (`src/lib/appMenu.ts`) porque rótulo é i18n e
//! acelerador é a tabela de atalhos do usuário — as duas coisas vivem no
//! webview e mudam em runtime. O core continua dono do que importa: quem pode
//! carregar acelerador, como o menu é construído e para onde o clique vai.

use serde::Deserialize;

pub const MENU_EVENT: &str = "tyba:menu";

#[derive(Debug, Clone, Deserialize)]
pub struct MenuSpec {
    pub submenus: Vec<SubmenuSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmenuSpec {
    pub label: String,
    pub items: Vec<ItemSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemSpec {
    Separator,
    Predefined {
        name: String,
        #[serde(default)]
        label: Option<String>,
    },
    Action {
        id: String,
        label: String,
        #[serde(default)]
        accelerator: Option<String>,
    },
}

/// Ações que podem carregar acelerador de menu.
///
/// No macOS o AppKit consome o acelerador **antes** do webview: dar um a uma
/// ação de terminal (copiar, colar, buscar, selecionar tudo) é o mesmo que
/// desligar a tecla dentro do xterm. Só entram aqui as ações que o handler
/// global de teclado do front já intercepta hoje — para essas, o caminho muda
/// mas o comportamento não.
const ACCELERATOR_ALLOWED: &[&str] = &[
    "paletteActions",
    "paletteSessions",
    "panel",
    "files",
    "filesFinder",
    "settings",
    "newSession",
    "newWorktreeSession",
    "newTab",
    "newWindow",
    "closePane",
    "openFolder",
    "splitRight",
    "splitDown",
    "nextPane",
];

pub fn sanitize_accelerator(id: &str, accelerator: Option<&str>) -> Option<String> {
    let accelerator = accelerator?.trim();
    if accelerator.is_empty() || !ACCELERATOR_ALLOWED.contains(&id) {
        return None;
    }
    Some(accelerator.to_string())
}

#[cfg(target_os = "macos")]
pub fn install(app: &tauri::AppHandle, spec: &MenuSpec) -> Result<(), String> {
    use tauri::menu::{IsMenuItem, MenuBuilder, Submenu};

    let mut submenus: Vec<Submenu<tauri::Wry>> = Vec::with_capacity(spec.submenus.len());
    for sub in &spec.submenus {
        let items = build_items(app, &sub.items);
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
            items.iter().map(|item| item.as_ref()).collect();
        let built = Submenu::with_items(app, &sub.label, true, &refs).map_err(|e| e.to_string())?;
        submenus.push(built);
    }

    let mut builder = MenuBuilder::new(app);
    for submenu in &submenus {
        builder = builder.item(submenu);
    }
    let menu = builder.build().map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn install(_app: &tauri::AppHandle, _spec: &MenuSpec) -> Result<(), String> {
    Ok(())
}

/// Menu mínimo instalado no boot, antes de o front falar.
///
/// Sem ele, um front que quebrasse antes de montar o menu deixaria o app sem
/// barra nenhuma no macOS — sem ⌘Q e sem os itens de clipboard, dos quais o
/// webview depende. Só itens `predefined`: o rótulo vem localizado do sistema,
/// então este menu não precisa saber o idioma escolhido pelo usuário.
#[cfg(target_os = "macos")]
pub fn install_fallback(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::menu::{MenuBuilder, SubmenuBuilder};

    let app_menu = SubmenuBuilder::new(app, "Tyba")
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()
        .map_err(|e| e.to_string())?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()
        .map_err(|e| e.to_string())?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .fullscreen()
        .separator()
        .close_window()
        .build()
        .map_err(|e| e.to_string())?;

    let menu = MenuBuilder::new(app)
        .item(&app_menu)
        .item(&edit)
        .item(&window)
        .build()
        .map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn install_fallback(_app: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

/// Item que falha a construir é pulado, nunca derruba o menu inteiro: um
/// acelerador inválido não pode deixar o app sem barra de menu.
#[cfg(target_os = "macos")]
fn build_items(
    app: &tauri::AppHandle,
    items: &[ItemSpec],
) -> Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> {
    use tauri::menu::{IsMenuItem, MenuItemBuilder, PredefinedMenuItem};

    let mut built: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            ItemSpec::Separator => {
                if let Ok(sep) = PredefinedMenuItem::separator(app) {
                    built.push(Box::new(sep));
                }
            }
            ItemSpec::Predefined { name, label } => {
                if let Some(predefined) = predefined(app, name, label.as_deref()) {
                    built.push(Box::new(predefined));
                }
            }
            ItemSpec::Action {
                id,
                label,
                accelerator,
            } => {
                let accelerator = sanitize_accelerator(id, accelerator.as_deref());
                let with_accelerator = accelerator.as_ref().and_then(|combo| {
                    MenuItemBuilder::with_id(id.clone(), label)
                        .accelerator(combo)
                        .build(app)
                        .ok()
                });
                let entry = match with_accelerator {
                    Some(entry) => Some(entry),
                    None => MenuItemBuilder::with_id(id.clone(), label).build(app).ok(),
                };
                if let Some(entry) = entry {
                    built.push(Box::new(entry));
                }
            }
        }
    }
    built
}

#[cfg(target_os = "macos")]
fn predefined(
    app: &tauri::AppHandle,
    name: &str,
    label: Option<&str>,
) -> Option<tauri::menu::PredefinedMenuItem<tauri::Wry>> {
    use tauri::menu::{AboutMetadata, PredefinedMenuItem};

    let built = match name {
        "about" => {
            let package = app.package_info();
            let metadata = AboutMetadata {
                name: Some(package.name.clone()),
                version: Some(package.version.to_string()),
                copyright: app.config().bundle.copyright.clone(),
                ..Default::default()
            };
            PredefinedMenuItem::about(app, label, Some(metadata))
        }
        "services" => PredefinedMenuItem::services(app, label),
        "hide" => PredefinedMenuItem::hide(app, label),
        "hide_others" => PredefinedMenuItem::hide_others(app, label),
        "show_all" => PredefinedMenuItem::show_all(app, label),
        "quit" => PredefinedMenuItem::quit(app, label),
        "close_window" => PredefinedMenuItem::close_window(app, label),
        "undo" => PredefinedMenuItem::undo(app, label),
        "redo" => PredefinedMenuItem::redo(app, label),
        "cut" => PredefinedMenuItem::cut(app, label),
        "copy" => PredefinedMenuItem::copy(app, label),
        "paste" => PredefinedMenuItem::paste(app, label),
        "select_all" => PredefinedMenuItem::select_all(app, label),
        "minimize" => PredefinedMenuItem::minimize(app, label),
        "maximize" => PredefinedMenuItem::maximize(app, label),
        "fullscreen" => PredefinedMenuItem::fullscreen(app, label),
        "bring_all_to_front" => PredefinedMenuItem::bring_all_to_front(app, label),
        _ => return None,
    };
    built.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_actions_never_get_an_accelerator() {
        for action in ["copy", "paste", "search", "selectAll", "richInput"] {
            assert_eq!(
                sanitize_accelerator(action, Some("Cmd+C")),
                None,
                "{action}"
            );
        }
    }

    #[test]
    fn allowed_actions_keep_the_accelerator() {
        assert_eq!(
            sanitize_accelerator("paletteActions", Some("Cmd+P")),
            Some("Cmd+P".to_string())
        );
    }

    #[test]
    fn blank_and_absent_accelerators_are_dropped() {
        assert_eq!(sanitize_accelerator("newTab", Some("   ")), None);
        assert_eq!(sanitize_accelerator("newTab", None), None);
    }

    #[test]
    fn unknown_ids_never_get_an_accelerator() {
        assert_eq!(sanitize_accelerator("openDocs", Some("Cmd+D")), None);
    }

    #[test]
    fn spec_parses_every_item_kind() {
        let spec: MenuSpec = serde_json::from_str(
            r#"{"submenus":[{"label":"Arquivo","items":[
                {"kind":"action","id":"newTab","label":"Nova aba","accelerator":"Cmd+T"},
                {"kind":"separator"},
                {"kind":"predefined","name":"close_window","label":"Fechar janela"}
            ]}]}"#,
        )
        .expect("spec válido");
        assert_eq!(spec.submenus.len(), 1);
        assert_eq!(spec.submenus[0].items.len(), 3);
    }
}
