//! A view nativa do navegador no macOS: um `WKWebView` por nó, filho do
//! `contentView` da janela principal (por cima da webview do app), criado
//! pelo `wry` com `build_as_child`.
//!
//! Tudo aqui roda na main thread (o `WKWebView` não é `Send`): o registro das
//! views é `thread_local` e as operações chegam por [`apply`].
//!
//! O que fecha o canal entre a página e o app, por construção:
//! - nenhum `with_ipc_handler` (o `window.ipc` que o wry injeta lança erro) e
//!   nenhum message handler: a configuração é nova, sem o `tauri://`;
//! - o `WKUIDelegate` é nosso e troca o do wry, que **concede câmera e
//!   microfone sozinho**: aqui toda captura é negada, o seletor de arquivo só
//!   abre com o dono usando, e janela nova cai no mesmo nó (a política de
//!   navegação confere);
//! - sem download, sem área de transferência por script, sem devtools.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::ptr::{null_mut, NonNull};
use std::sync::Arc;

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventModifierFlags, NSModalResponseOK, NSOpenPanel, NSView, NSWindow,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSProcessInfo, NSURL};
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKNavigationAction, WKOpenPanelParameters,
    WKPermissionDecision, WKSecurityOrigin, WKUIDelegate, WKWebView, WKWebViewConfiguration,
    WKWindowFeatures,
};
use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::{header::CONTENT_TYPE, Request, Response, StatusCode};
use wry::{PageLoadEvent, Rect, WebViewBuilder, WebViewBuilderExtDarwin, WebViewExtMacOS};

use super::{
    policy, route_key, store_identity, BrowserViews, Focus, KeyRoute, Mods, Op, Place, ViewKey,
};

/// Teto do arquivo servido pelo `orbit-file`.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

struct NativeView {
    webview: wry::WebView,
    wk: Retained<WKWebView>,
    /// O `UIDelegate` do `WKWebView` é fraco: quem o segura é este campo.
    _ui: Retained<OrbitUiDelegate>,
    roots: Arc<Mutex<Vec<PathBuf>>>,
}

thread_local! {
    static VIEWS: RefCell<HashMap<ViewKey, NativeView>> = RefCell::new(HashMap::new());
    /// Os monitores de evento (clique e tecla), instalados uma vez.
    static MONITORS: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) };
}

/// Executa a operação (na main thread).
pub(super) fn apply(app: &AppHandle, op: Op) {
    match op {
        Op::Navigate {
            key,
            url,
            roots,
            place,
        } => {
            let exists = VIEWS.with(|v| v.borrow().contains_key(&key));
            if !exists {
                match create(app, &key) {
                    Ok(view) => {
                        VIEWS.with(|v| {
                            v.borrow_mut().insert(key.clone(), view);
                        });
                        install_monitors(app);
                        app.state::<BrowserViews>().start_ticking(app);
                    }
                    Err(e) => {
                        eprintln!("[tyba] navegador do orbit: criar a view: {e}");
                        return;
                    }
                }
            }
            VIEWS.with(|v| {
                if let Some(view) = v.borrow().get(&key) {
                    *view.roots.lock() = roots;
                    set_place(view, &place);
                    if let Err(e) = view.webview.load_url(&url) {
                        eprintln!("[tyba] navegador do orbit: carregar: {e}");
                    }
                }
            });
        }
        Op::Place { key, place } => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                set_place(view, &place);
            }
        }),
        Op::Back(key) => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                // SAFETY: main thread; o `WKWebView` vive enquanto a entrada.
                unsafe { view.wk.goBack() };
            }
        }),
        Op::Reload(key) => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                let _ = view.webview.reload();
            }
        }),
        Op::Snapshot(key) => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                snapshot(app, &key, view);
            }
        }),
        Op::Retain(keep) => {
            // Tira do registro antes de soltar: o `Drop` do wry tira a view
            // da janela e pode chamar delegates.
            let gone: Vec<NativeView> = VIEWS.with(|v| {
                let mut v = v.borrow_mut();
                let dead: Vec<ViewKey> = v.keys().filter(|k| !keep.contains(k)).cloned().collect();
                dead.iter().filter_map(|k| v.remove(k)).collect()
            });
            drop(gone);
        }
    }
}

/// Posição, zoom e visibilidade. Mostra só depois de posicionar.
fn set_place(view: &NativeView, place: &Place) {
    let visible = place.visible && place.rect.is_some();
    if let Some(r) = place.rect {
        let _ = view.webview.set_bounds(Rect {
            position: LogicalPosition::new(r.x, r.y).into(),
            size: LogicalSize::new(r.w, r.h).into(),
        });
        // `setPageZoom`: reescala o layout como o `transform` do canvas (o
        // `setMagnification` seria zoom de pinça e deslocaria o viewport).
        let _ = view.webview.zoom(place.zoom.clamp(0.25, 5.0));
    }
    if !visible {
        give_back_if_inside(view);
    }
    let _ = view.webview.set_visible(visible);
}

/// macOS 14+: `WKWebsiteDataStore(forIdentifier:)` por nó. Antes disso o
/// wry cairia no `defaultDataStore`, que é compartilhado entre nós.
fn per_node_store_available() -> bool {
    NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        >= 14
}

fn create(app: &AppHandle, key: &ViewKey) -> Result<NativeView, String> {
    let mtm = MainThreadMarker::new().ok_or("fora da main thread")?;
    let window = app
        .get_webview_window("main")
        .ok_or("janela principal não encontrada")?;
    let roots: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let ephemeral = !per_node_store_available();

    let nav_app = app.clone();
    let nav_key = key.clone();
    let load_app = app.clone();
    let load_key = key.clone();
    let title_app = app.clone();
    let title_key = key.clone();
    let serve_roots = roots.clone();

    let builder = WebViewBuilder::new()
        .with_visible(false)
        .with_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(1.0, 1.0).into(),
        })
        .with_clipboard(false)
        .with_devtools(false)
        .with_hotkeys_zoom(false)
        .with_back_forward_navigation_gestures(false)
        .with_accept_first_mouse(true)
        .with_autoplay(false)
        // Toda navegação, de qualquer frame, passa pela política.
        .with_navigation_handler(move |url| match policy::decide(&url) {
            policy::Decision::Allow => true,
            policy::Decision::Deny(why) => {
                nav_app
                    .state::<BrowserViews>()
                    .on_refused(&nav_app, &nav_key, why);
                false
            }
        })
        .with_download_started_handler(|_, _| false)
        .with_on_page_load_handler(move |event, url| {
            let finished = matches!(event, PageLoadEvent::Finished);
            load_app
                .state::<BrowserViews>()
                .on_load(&load_app, &load_key, &url, finished);
        })
        .with_document_title_changed_handler(move |title| {
            title_app
                .state::<BrowserViews>()
                .on_title(&title_app, &title_key, &title);
        })
        .with_custom_protocol(policy::FILE_SCHEME.into(), move |_id, request| {
            serve_file(&serve_roots.lock(), &request)
        })
        .with_url("about:blank");
    let builder = if ephemeral {
        builder.with_incognito(true)
    } else {
        builder.with_data_store_identifier(store_identity(key))
    };
    let webview = builder.build_as_child(&window).map_err(|e| e.to_string())?;
    // SAFETY: o `WryWebView` do wry é subclasse de `WKWebView`.
    let wk: Retained<WKWebView> = unsafe { Retained::cast_unchecked(webview.webview()) };
    let ui = OrbitUiDelegate::new(mtm, app.clone(), key.clone());
    // SAFETY: main thread; o delegate é segurado pela `NativeView`.
    unsafe { wk.setUIDelegate(Some(ProtocolObject::from_ref(&*ui))) };
    app.state::<BrowserViews>().on_created(app, key, ephemeral);
    Ok(NativeView {
        webview,
        wk,
        _ui: ui,
        roots,
    })
}

/// `orbit-file://r<i>/<caminho>`: só arquivo de dentro da raiz `i`.
fn serve_file(roots: &[PathBuf], request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    let host = uri.host().unwrap_or("");
    let found = policy::serve_path(host, uri.path(), roots).and_then(|path| {
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > MAX_FILE_BYTES {
            return None;
        }
        let body = std::fs::read(&path).ok()?;
        Some((policy::content_type(&path), body))
    });
    let response = match found {
        Some((mime, body)) => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, mime)
            .body(Cow::Owned(body)),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Cow::Borrowed(&b"nao encontrado"[..])),
    };
    response.unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
}

struct UiIvars {
    app: AppHandle,
    key: ViewKey,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UiIvars]
    struct OrbitUiDelegate;

    unsafe impl NSObjectProtocol for OrbitUiDelegate {}

    unsafe impl WKUIDelegate for OrbitUiDelegate {
        /// Câmera e microfone: sempre negados (o do wry concedia).
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn request_media_capture_permission(
            &self,
            _webview: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _capture_type: WKMediaCaptureType,
            decision_handler: &Block<dyn Fn(WKPermissionDecision)>,
        ) {
            decision_handler.call((WKPermissionDecision::Deny,));
        }

        /// `<input type=file>`: só abre com o dono usando a página.
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn run_open_panel(
            &self,
            _webview: &WKWebView,
            params: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            handler: &Block<dyn Fn(*const NSArray<NSURL>)>,
        ) {
            let ivars = self.ivars();
            let owner = ivars.app.state::<BrowserViews>().focus(&ivars.key) == Focus::Owner;
            let Some(mtm) = MainThreadMarker::new().filter(|_| owner) else {
                handler.call((null_mut(),));
                return;
            };
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseFiles(true);
            // SAFETY: parâmetros vindos do WebKit, na main thread.
            unsafe {
                panel.setAllowsMultipleSelection(params.allowsMultipleSelection());
                panel.setCanChooseDirectories(params.allowsDirectories());
            }
            if panel.runModal() == NSModalResponseOK {
                let urls = panel.URLs();
                handler.call((Retained::as_ptr(&urls),));
            } else {
                handler.call((null_mut(),));
            }
        }

        /// `target=_blank` / `window.open`: nada de janela nova; a página
        /// abre no mesmo nó, e a política de navegação decide se carrega.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            webview: &WKWebView,
            _configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            // SAFETY: main thread; o pedido vem do próprio WebKit.
            unsafe {
                let request = action.request();
                let _ = webview.loadRequest(&request);
            }
            None
        }
    }
);

impl OrbitUiDelegate {
    fn new(mtm: MainThreadMarker, app: AppHandle, key: ViewKey) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(UiIvars { app, key });
        // SAFETY: `init` do `NSObject`.
        unsafe { msg_send![super(this), init] }
    }
}

// ── foco e teclado ──────────────────────────────────────────────────────────

/// A view do app (a webview do Tauri): a subview do `contentView` que não é
/// de nenhum nó. É para ela que o foco volta.
fn app_view(window: &NSWindow) -> Option<Retained<NSView>> {
    let content = window.contentView()?;
    let subviews = content.subviews();
    VIEWS.with(|v| {
        let v = v.borrow();
        subviews.iter().find(|sub| {
            !v.values().any(|n| {
                let wk: &NSView = &n.wk;
                std::ptr::eq(wk, &**sub)
            })
        })
    })
}

/// Devolve o foco do teclado ao app.
fn give_back(window: &NSWindow) {
    if let Some(app) = app_view(window) {
        window.makeFirstResponder(Some(&app));
    }
}

/// A view vai sumir com o teclado nela: o foco volta ao app antes.
fn give_back_if_inside(view: &NativeView) {
    let Some(window) = view.wk.window() else {
        return;
    };
    let inside = window
        .firstResponder()
        .and_then(|r| r.downcast::<NSView>().ok())
        .is_some_and(|r| r.isDescendantOf(&view.wk));
    if inside {
        give_back(&window);
    }
}

/// O nó cuja view contém `view` (ela mesma ou um descendente).
fn node_of(view: &NSView) -> Option<ViewKey> {
    VIEWS.with(|v| {
        v.borrow()
            .iter()
            .find(|(_, n)| view.isDescendantOf(&n.wk))
            .map(|(k, _)| k.clone())
    })
}

fn is_main_window(app: &AppHandle, window: &NSWindow) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.ns_window().ok())
        .is_some_and(|p| std::ptr::eq(p as *const NSWindow, window))
}

/// Monitores locais de clique e tecla (uma vez por app). O clique numa
/// view de nó marca "você"; fora de todas, devolve. A tecla com o foco numa
/// página passa por [`route_key`]: Esc devolve (e some), ⌘W/⌘K/⌘L vão ao
/// app, o resto segue para a página. Uma tecla passa pelo app antes da
/// página — é isso que deixa o Esc devolver.
fn install_monitors(app: &AppHandle) {
    if MONITORS.with(|m| !m.borrow().is_empty()) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    let click_app = app.clone();
    let click = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: o AppKit entrega um evento válido durante a chamada.
        let ev = unsafe { event.as_ref() };
        if let Some(window) = ev.window(mtm).filter(|w| is_main_window(&click_app, w)) {
            let hit = window
                .contentView()
                .and_then(|c| c.hitTest(ev.locationInWindow()))
                .and_then(|v| node_of(&v));
            let views = click_app.state::<BrowserViews>();
            views.set_owner(&click_app, hit.as_ref());
            if let Some(k) = &hit {
                views.mark_dirty(k);
            }
        }
        event.as_ptr()
    });

    let key_app = app.clone();
    let key = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: o AppKit entrega um evento válido durante a chamada.
        let ev = unsafe { event.as_ref() };
        let Some(window) = ev.window(mtm).filter(|w| is_main_window(&key_app, w)) else {
            return event.as_ptr();
        };
        let Some(node) = window
            .firstResponder()
            .and_then(|r| r.downcast::<NSView>().ok())
            .and_then(|v| node_of(&v))
        else {
            return event.as_ptr();
        };
        let flags = ev.modifierFlags();
        let mods = Mods {
            shift: flags.contains(NSEventModifierFlags::Shift),
            control: flags.contains(NSEventModifierFlags::Control),
            option: flags.contains(NSEventModifierFlags::Option),
            command: flags.contains(NSEventModifierFlags::Command),
        };
        let chars = ev
            .charactersIgnoringModifiers()
            .map(|c| c.to_string())
            .unwrap_or_default();
        match route_key(ev.keyCode(), mods, &chars) {
            KeyRoute::Page => {
                key_app.state::<BrowserViews>().mark_dirty(&node);
                event.as_ptr()
            }
            KeyRoute::GiveBack => {
                give_back(&window);
                key_app.state::<BrowserViews>().set_owner(&key_app, None);
                null_mut()
            }
            KeyRoute::ToApp => {
                give_back(&window);
                key_app.state::<BrowserViews>().set_owner(&key_app, None);
                event.as_ptr()
            }
        }
    });

    // SAFETY: os blocos devolvem o próprio evento (válido) ou nulo.
    let monitors = unsafe {
        [
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown,
                &click,
            ),
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &key),
        ]
    };
    MONITORS.with(|m| m.borrow_mut().extend(monitors.into_iter().flatten()));
}

// ── print ───────────────────────────────────────────────────────────────────

/// Largura máxima da prévia, em pontos.
const PREVIEW_MAX_W: f64 = 640.0;

/// `takeSnapshot` → JPEG (fator 0,7) em base64 → [`BrowserViews::on_snapshot`].
/// Funciona com a view escondida (conferido no protótipo: o print sai com a
/// página certa, inclusive o que foi digitado).
fn snapshot(app: &AppHandle, key: &ViewKey, view: &NativeView) {
    use base64::Engine;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSImageCompressionFactor,
    };
    use objc2_foundation::{NSDictionary, NSError, NSNumber};
    use objc2_web_kit::WKSnapshotConfiguration;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let width = view.wk.frame().size.width;
    if width < 10.0 {
        return;
    }
    // SAFETY: main thread; a configuração é nossa.
    let config = unsafe { WKSnapshotConfiguration::new(mtm) };
    let number = NSNumber::new_f64(width.min(PREVIEW_MAX_W));
    // SAFETY: `snapshotWidth` aceita um `NSNumber` em pontos.
    unsafe { config.setSnapshotWidth(Some(&number)) };
    let done_app = app.clone();
    let done_key = key.clone();
    let block = RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
        // SAFETY: o WebKit entrega imagem válida ou nula.
        let Some(image) = (unsafe { image.as_ref() }) else {
            return;
        };
        let Some(tiff) = image.TIFFRepresentation() else {
            return;
        };
        let Some(rep) = NSBitmapImageRep::imageRepWithData(&tiff) else {
            return;
        };
        let factor = NSNumber::new_f64(0.7);
        // SAFETY: a chave é a constante do AppKit e o valor um `NSNumber`.
        let jpeg = unsafe {
            let props =
                NSDictionary::from_slices(&[NSImageCompressionFactor], &[&*factor as &AnyObject]);
            rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &props)
        };
        let Some(jpeg) = jpeg else {
            return;
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg.to_vec());
        done_app
            .state::<BrowserViews>()
            .on_snapshot(&done_app, &done_key, b64);
    });
    // SAFETY: main thread; o bloco é copiado pelo WebKit.
    unsafe {
        view.wk
            .takeSnapshotWithConfiguration_completionHandler(Some(&config), &block)
    };
}
