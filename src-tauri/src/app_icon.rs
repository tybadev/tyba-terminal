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
//! O padrão (`blackout`) é o ícone do bundle: escolhê-lo devolve o ícone ao
//! sistema em vez de pôr um PNG por cima — no macOS 26 o do bundle acompanha
//! claro/escuro, coisa que um PNG fixo não faz.

use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri::Manager;

/// Payload: o id do ícone em vigor, para as outras janelas acompanharem.
pub const EVENT_CHANGED: &str = "app-icon://changed";

/// Chave provisória no SQLite — ver [`IconPref`].
const STORE_KEY: &str = "appearance.app_icon";

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

    /// O PNG de 1024 embutido; `None` no padrão, que é o ícone do bundle.
    pub fn png(self) -> Option<&'static [u8]> {
        Some(match self {
            Self::Blackout => return None,
            Self::Aurora => include_bytes!("../icons/app/aurora.png"),
            Self::Filamento => include_bytes!("../icons/app/filamento.png"),
            Self::Assinatura => include_bytes!("../icons/app/assinatura.png"),
            Self::Fosforo => include_bytes!("../icons/app/fosforo.png"),
            Self::Violeta => include_bytes!("../icons/app/violeta.png"),
            Self::Grafite => include_bytes!("../icons/app/grafite.png"),
            Self::Papel => include_bytes!("../icons/app/papel.png"),
        })
    }
}

/// Valor salvo → ícone. Ausente ou desconhecido (arquivo editado à mão, versão
/// mais nova que tinha outra variante) cai no padrão.
pub fn resolve(stored: Option<&str>) -> AppIcon {
    stored
        .and_then(AppIcon::from_id)
        .unwrap_or(AppIcon::DEFAULT)
}

/// O que o arranque aplica. O padrão não pede nada: o processo já nasce com o
/// ícone do bundle, e mexer nele só perderia o claro/escuro do macOS 26.
pub fn startup_icon(stored: Option<&str>) -> Option<AppIcon> {
    Some(resolve(stored)).filter(|icon| *icon != AppIcon::DEFAULT)
}

/// Onde a escolha mora. **Provisório:** o destino é `[appearance] app_icon` no
/// `~/.tyba/config.toml`, cuja API (`config`) ainda não expõe a chave; até lá
/// vale o SQLite. Trocar a origem é trocar o `impl` — os comandos não mudam.
pub trait IconPref {
    fn load_icon(&self) -> Option<String>;
    fn save_icon(&self, icon: AppIcon) -> Result<(), String>;
}

impl IconPref for crate::session::store::Store {
    fn load_icon(&self) -> Option<String> {
        self.get_setting(STORE_KEY).ok().flatten()
    }

    fn save_icon(&self, icon: AppIcon) -> Result<(), String> {
        self.set_setting(STORE_KEY, icon.id())
            .map_err(|e| e.to_string())
    }
}

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
        apply_to_window(app, &window, icon);
    }
}

/// Windows/Linux: o ícone mora em cada janela, então janela nova precisa
/// recebê-lo também.
#[cfg(not(target_os = "macos"))]
pub fn apply_to_window(app: &AppHandle, window: &tauri::WebviewWindow, icon: AppIcon) {
    let image = match icon.png() {
        Some(bytes) => tauri::image::Image::from_bytes(bytes).ok(),
        None => app.default_window_icon().cloned(),
    };
    if let Some(image) = image {
        if let Err(e) = window.set_icon(image) {
            eprintln!("[tyba] ícone da janela: {e}");
        }
    }
}

/// `None` devolve o ícone do bundle (contrato do `applicationIconImage`).
#[cfg(target_os = "macos")]
fn set_dock_icon(png: Option<&'static [u8]>) {
    use objc2::AnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let image = match png {
        Some(bytes) => {
            let data = NSData::with_bytes(bytes);
            match NSImage::initWithData(NSImage::alloc(), &data) {
                Some(image) => Some(image),
                // PNG embutido que não decodifica é bug de build: melhor manter
                // o ícone atual que apagar para o do bundle.
                None => return,
            }
        }
        None => None,
    };
    let ns_app = NSApplication::sharedApplication(mtm);
    // SAFETY: na main thread (garantido pelo `MainThreadMarker`), e `nil` é
    // aceito: a documentação da Apple diz que restaura o ícone do bundle.
    unsafe { ns_app.setApplicationIconImage(image.as_deref()) };
}

/// Ícone salvo agora, resolvido para o id que a UI marca.
pub fn current(pref: &dyn IconPref) -> AppIcon {
    resolve(pref.load_icon().as_deref())
}

/// O arranque: reaplica a escolha que não é o padrão.
pub fn apply_at_startup(app: &AppHandle, pref: &dyn IconPref) {
    if let Some(icon) = startup_icon(pref.load_icon().as_deref()) {
        apply(app, icon);
    }
}

/// O comando da UI: valida, salva, aplica e avisa as janelas. Id fora da lista
/// é recusado antes de qualquer efeito.
pub fn choose(app: &AppHandle, pref: &dyn IconPref, id: &str) -> Result<AppIcon, String> {
    let icon = AppIcon::from_id(id).ok_or("ícone desconhecido")?;
    pref.save_icon(icon)?;
    apply(app, icon);
    Ok(icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

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
    fn startup_leaves_the_bundle_icon_alone_for_the_default() {
        assert_eq!(startup_icon(None), None);
        assert_eq!(startup_icon(Some("blackout")), None);
        assert_eq!(startup_icon(Some("lixo")), None);
        assert_eq!(startup_icon(Some("violeta")), Some(AppIcon::Violeta));
    }

    #[test]
    fn every_variant_but_the_default_embeds_a_1024_png() {
        assert!(AppIcon::Blackout.png().is_none());
        for icon in AppIcon::ALL.into_iter().skip(1) {
            let png = icon.png().unwrap();
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

    #[derive(Default)]
    struct Mem(RefCell<Option<String>>);

    impl IconPref for Mem {
        fn load_icon(&self) -> Option<String> {
            self.0.borrow().clone()
        }
        fn save_icon(&self, icon: AppIcon) -> Result<(), String> {
            *self.0.borrow_mut() = Some(icon.id().to_string());
            Ok(())
        }
    }

    #[test]
    fn current_reads_through_the_pref() {
        let pref = Mem::default();
        assert_eq!(current(&pref), AppIcon::Blackout);
        pref.save_icon(AppIcon::Fosforo).unwrap();
        assert_eq!(current(&pref), AppIcon::Fosforo);
    }

    #[test]
    fn store_pref_round_trips() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        assert_eq!(store.load_icon(), None);
        store.save_icon(AppIcon::Grafite).unwrap();
        assert_eq!(current(&store), AppIcon::Grafite);
    }
}
