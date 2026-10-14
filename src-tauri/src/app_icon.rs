//! Ícone do app escolhido pelo usuário (Configurações → Aparência → "Ícone do
//! app"). Ver vault `tyba/features/icone-app/`.
//!
//! As oito variantes são PNGs **embutidos no binário** (`icons/app/`, gerados
//! por `scripts/gen-app-icon-variants.ts`). Quem escolhe manda só o id; o core
//! nunca lê imagem de um caminho vindo de fora.
//!
//! - macOS: troca o ícone do Dock em tempo de execução
//!   (`NSApplication.applicationIconImage`). Isso não sobrevive ao fim do
//!   processo, então o arranque reaplica a escolha salva. O ícone do Finder e
//!   da pasta Aplicações não muda — fica para depois (ver `rules.md`).
//! - Windows/Linux: o ícone de cada janela (melhor esforço).
//!
//! Toda variante, inclusive o padrão (`blackout`), é aplicada pelo próprio
//! PNG: a miniatura da grade é o que o Dock mostra. O ícone do bundle no macOS
//! 26 acompanha claro/escuro e ficaria branco no modo claro — decisão do dono:
//! BLACKOUT é sempre preto.
//!
//! No perfil de desenvolvimento ([`crate::profile::IS_DEV`]) o Dock ganha o
//! selo "DEV" ([`DEV_BADGE`]), para distinguir o app do `tauri dev` do
//! instalado.

#[cfg(not(target_os = "macos"))]
use tauri::Manager;
use tauri::{AppHandle, Listener};

use crate::config::{self, ConfigManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    Blackout,
    Aurora,
    Filamento,
    Assinatura,
    Fosforo,
    Violeta,
    Grafite,
    Papel,
}

impl AppIcon {
    pub const DEFAULT: Self = Self::Blackout;

    /// Na ordem da grade. Espelho de `APP_ICONS` em `src/lib/appIcon.ts` — o
    /// teste `ids_match_the_front` falha se os dois divergirem.
    pub const ALL: [Self; 8] = [
        Self::Blackout,
        Self::Aurora,
        Self::Filamento,
        Self::Assinatura,
        Self::Fosforo,
        Self::Violeta,
        Self::Grafite,
        Self::Papel,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Blackout => "blackout",
            Self::Aurora => "aurora",
            Self::Filamento => "filamento",
            Self::Assinatura => "assinatura",
            Self::Fosforo => "fosforo",
            Self::Violeta => "violeta",
            Self::Grafite => "grafite",
            Self::Papel => "papel",
        }
    }

    /// Só aceita um id exato da lista: nada de caminho, nada de normalização.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|icon| icon.id() == id)
    }

    /// O PNG de 1024 embutido.
    pub fn png(self) -> &'static [u8] {
        match self {
            Self::Blackout => include_bytes!("../icons/app/blackout.png"),
            Self::Aurora => include_bytes!("../icons/app/aurora.png"),
            Self::Filamento => include_bytes!("../icons/app/filamento.png"),
            Self::Assinatura => include_bytes!("../icons/app/assinatura.png"),
            Self::Fosforo => include_bytes!("../icons/app/fosforo.png"),
            Self::Violeta => include_bytes!("../icons/app/violeta.png"),
            Self::Grafite => include_bytes!("../icons/app/grafite.png"),
            Self::Papel => include_bytes!("../icons/app/papel.png"),
        }
    }
}

/// Valor salvo → ícone. Ausente ou desconhecido (arquivo editado à mão, versão
/// mais nova que tinha outra variante) cai no padrão.
pub fn resolve(stored: Option<&str>) -> AppIcon {
    stored
        .and_then(AppIcon::from_id)
        .unwrap_or(AppIcon::DEFAULT)
}

/// Selo do Dock: só em build de desenvolvimento, nunca no release.
/// Segue o [`crate::profile`], a fonte única de dev × release.
pub const DEV_BADGE: Option<&str> = if crate::profile::IS_DEV {
    Some("DEV")
} else {
    None
};

/// Põe o ícone em vigor. Falha é silenciosa e só no log: um ícone que não
/// trocou não pode atrapalhar o app.
pub fn apply(app: &AppHandle, icon: AppIcon) {
    #[cfg(target_os = "macos")]
    {
        let png = icon.png();
        let result = app.run_on_main_thread(move || set_dock_icon(png));
        if let Err(e) = result {
            eprintln!("[tyba] ícone do app: {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    for window in app.webview_windows().into_values() {
        apply_to_window(&window, icon);
    }
}

/// Windows/Linux: o ícone mora em cada janela, então janela nova precisa
/// recebê-lo também.
#[cfg(not(target_os = "macos"))]
pub fn apply_to_window(window: &tauri::WebviewWindow, icon: AppIcon) {
    match tauri::image::Image::from_bytes(icon.png()) {
        Ok(image) => {
            if let Err(e) = window.set_icon(image) {
                eprintln!("[tyba] ícone da janela: {e}");
            }
        }
        Err(e) => eprintln!("[tyba] ícone da janela: {e}"),
    }
}

#[cfg(target_os = "macos")]
fn set_dock_icon(png: &'static [u8]) {
    use objc2::AnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    // PNG embutido que não decodifica é bug de build: mantém o ícone atual.
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)) else {
        return;
    };
    let ns_app = NSApplication::sharedApplication(mtm);
    // SAFETY: na main thread (garantido pelo `MainThreadMarker`) e com imagem
    // não nula — a única condição que o binding declara.
    unsafe { ns_app.setApplicationIconImage(Some(&image)) };
}

/// macOS: o selo "DEV" no Dock. Windows/Linux não têm onde pô-lo sem gerar
/// outro ícone (o Tauri só expõe o overlay do Windows e a contagem); ficam sem.
fn show_dev_badge(app: &AppHandle) {
    let Some(label) = DEV_BADGE else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let result = app.run_on_main_thread(move || {
            let Some(mtm) = objc2::MainThreadMarker::new() else {
                return;
            };
            let tile = objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile();
            tile.setBadgeLabel(Some(&objc2_foundation::NSString::from_str(label)));
        });
        if let Err(e) = result {
            eprintln!("[tyba] selo DEV: {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, label);
}

/// O ícone salvo no `config.toml` (`[appearance] app_icon`).
pub fn current(config: &ConfigManager) -> AppIcon {
    resolve(Some(config.app_icon()))
}

/// O arranque: reaplica a escolha salva e, em dev, o selo. Roda no
/// `RunEvent::Ready`, e não no `setup`: no `tauri dev` o Tauri põe o ícone de
/// dev no Dock entre os dois, e passaria por cima da escolha.
pub fn apply_at_startup(app: &AppHandle, config: &ConfigManager) {
    apply(app, current(config));
    show_dev_badge(app);
}

/// Aplica toda mudança publicada pelo `config`: a da UI (`set_app_icon`) e a
/// edição na mão do arquivo passam pelo mesmo evento. O ícone vem do
/// `ConfigManager`, nunca do payload: a webview também pode emitir este
/// evento, e assim um emit forjado só reaplica o que está salvo.
pub fn follow_config(app: &AppHandle, config: config::SharedConfig) {
    let handle = app.clone();
    app.listen_any(config::EVENT_APP_ICON, move |_| {
        apply(&handle, current(&config));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_round_trips_and_is_unique() {
        for icon in AppIcon::ALL {
            assert_eq!(AppIcon::from_id(icon.id()), Some(icon));
        }
        let mut ids: Vec<_> = AppIcon::ALL.iter().map(|i| i.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), AppIcon::ALL.len());
    }

    #[test]
    fn only_exact_ids_are_accepted() {
        for bad in [
            "",
            "Aurora",
            " aurora",
            "aurora ",
            "../aurora",
            "aurora.png",
            "icons/app/aurora.png",
            "/etc/passwd",
            "fósforo",
        ] {
            assert_eq!(AppIcon::from_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn missing_or_unknown_resolves_to_default() {
        assert_eq!(resolve(None), AppIcon::Blackout);
        assert_eq!(resolve(Some("neon")), AppIcon::Blackout);
        assert_eq!(resolve(Some("papel")), AppIcon::Papel);
    }

    #[test]
    fn dev_badge_only_in_debug_builds() {
        assert_eq!(DEV_BADGE.is_some(), crate::profile::IS_DEV);
    }

    #[test]
    fn every_variant_embeds_a_1024_png() {
        for icon in AppIcon::ALL {
            let png = icon.png();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{}", icon.id());
            // IHDR: largura e altura em big-endian logo depois da assinatura.
            let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
            let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
            assert_eq!((w, h), (1024, 1024), "{}", icon.id());
        }
    }

    #[test]
    fn ids_match_the_front() {
        let src = include_str!("../../src/lib/appIcon.ts");
        let start = src.find("export const APP_ICONS").unwrap();
        let body = &src[start..];
        let body = &body[body.find("= [").unwrap() + 3..body.find("] as const").unwrap()];
        let front: Vec<&str> = body
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .collect();
        let core: Vec<&str> = AppIcon::ALL.iter().map(|i| i.id()).collect();
        assert_eq!(front, core);
    }

    #[test]
    fn ids_match_the_config_schema() {
        let core: Vec<&str> = AppIcon::ALL.iter().map(|i| i.id()).collect();
        assert_eq!(config::schema::APP_ICONS, core.as_slice());
        assert_eq!(config::schema::DEFAULT_APP_ICON, AppIcon::DEFAULT.id());
    }

    #[test]
    fn current_reads_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::session::store::Store::open_in_memory().unwrap();
        let cfg = ConfigManager::open(dir.path().join(".tyba"), &store);
        assert_eq!(current(&cfg), AppIcon::Blackout);
        cfg.set_app_icon("fosforo").unwrap();
        assert_eq!(current(&cfg), AppIcon::Fosforo);
    }
}
