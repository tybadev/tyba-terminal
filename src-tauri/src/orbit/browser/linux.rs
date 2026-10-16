//! A view nativa do navegador no Linux: uma `WebKitWebView` por nó, por cima
//! da webview do app, criada pelo `wry` com `build_gtk` (funciona em X11 e em
//! Wayland; o `build_as_child` do wry só existe em X11).
//!
//! O encaixe na janela: na primeira view, a webview do app sai da `vbox` do
//! Tauri e entra num `gtk::Overlay` como filho principal; por cima dela vai
//! um `gtk::Fixed` que deixa o clique passar (`pass-through`), e cada nó é um
//! filho desse `Fixed`, posicionado pelo retângulo do card. Fora do
//! retângulo de um nó, o clique chega à webview do app.
//!
//! Escondida, a view sai para fora da área visível em vez de sumir: continua
//! com tamanho de página (o agente lê e clica nela, e o print sai).
//!
//! Tudo aqui roda na main thread (GTK): o registro das views é
//! `thread_local` e as operações chegam por [`apply`].
//!
//! O que fecha o canal entre a página e o app, por construção:
//! - nenhum `with_ipc_handler`, e o message handler `ipc` que o wry registra
//!   sempre é tirado logo depois de criar (`window.webkit.messageHandlers`
//!   fica sem nada);
//! - um `WebContext` próprio por nó (pasta de dados própria), nunca o do
//!   Tauri: só o `orbit-file` nosso é registrado nele;
//! - toda permissão negada no `permission-request`;
//! - toda navegação, de qualquer frame, pelo `decide-policy` do wry;
//! - sem download, sem devtools, sem área de transferência por script.
//!
//! O mundo isolado é o do WebKit (`tyba-orbit`): o `isolated.js` entra por
//! `UserScript::for_world` e cada ação é `call_async_javascript_function`
//! nesse mundo, com `action` e `args` num dicionário de argumentos, nunca
//! colados no código.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gtk::prelude::*;
use javascriptcore::ValueExt;
use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use webkit2gtk::{
    PermissionRequestExt, SnapshotOptions, SnapshotRegion, UserContentInjectedFrames,
    UserContentManagerExt, UserScript, UserScriptInjectionTime, WebViewExt,
};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{
    NewWindowResponse, PageLoadEvent, Rect, WebContext, WebViewBuilder, WebViewBuilderExtUnix,
    WebViewExtUnix,
};

use super::native::{
    serve_file, HIDDEN_H, HIDDEN_W, ISOLATED_JS, PAGE_SHIM_JS, PREVIEW_MAX_W, RUN_JS, WORLD,
};
use super::{
    ensure_store_dir, forward_shortcut_js, policy, route, store_dir, BrowserViews, Focus, KeyRoute,
    Mods, Op, Place, ViewKey,
};

/// Onde a view escondida fica: fora da área do `Fixed`, recortada.
const OFFSCREEN: i32 = -30_000;

struct NativeView {
    webview: wry::WebView,
    wk: webkit2gtk::WebView,
    /// O contexto (armazenamento e o `orbit-file`) vive enquanto a view.
    _context: WebContext,
    roots: Arc<Mutex<Vec<PathBuf>>>,
}

/// O encaixe na janela: o `Fixed` dos nós e a webview do app.
struct Layer {
    fixed: gtk::Fixed,
    app: webkit2gtk::WebView,
}

thread_local! {
    static VIEWS: RefCell<HashMap<ViewKey, NativeView>> = RefCell::new(HashMap::new());
    static LAYER: RefCell<Option<Layer>> = const { RefCell::new(None) };
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
                    set_place(app, &key, view, &place);
                    if let Err(e) = view.webview.load_url(&url) {
                        eprintln!("[tyba] navegador do orbit: carregar: {e}");
                    }
                }
            });
        }
        Op::Place { key, place } => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                set_place(app, &key, view, &place);
            }
        }),
        Op::Back(key) => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                view.wk.go_back();
            }
        }),
        Op::Reload(key) => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                let _ = view.webview.reload();
            }
        }),
        Op::Snapshot(key) => VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&key) {
                let done_app = app.clone();
                let done_key = key.clone();
                shot(&view.wk, "jpeg", PREVIEW_MAX_W, move |r| {
                    if let Ok((jpeg, _, _)) = r {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
                        done_app
                            .state::<BrowserViews>()
                            .on_snapshot(&done_app, &done_key, b64);
                    }
                });
            }
        }),
        Op::Capture { key, reply } => VIEWS.with(|v| match v.borrow().get(&key) {
            // O print do agente **não** é redigido (fica na pasta do run, 0600).
            Some(view) => shot(&view.wk, "png", super::MAX_SHOT_PX, move |r| {
                let _ =
                    reply.send(r.map(|(png, width, height)| super::Shot { png, width, height }));
            }),
            None => {
                let _ = reply.send(Err(super::NOT_OPEN.to_string()));
            }
        }),
        Op::Eval {
            key,
            action,
            args,
            reply,
        } => VIEWS.with(|v| match v.borrow().get(&key) {
            Some(view) => eval(view, action, &args, reply),
            None => {
                let _ = reply.send(Err(super::NOT_OPEN.to_string()));
            }
        }),
        Op::Retain(keep) => {
            // Tira do registro antes de soltar: o `Drop` do wry destrói o
            // widget e pode disparar sinais.
            let gone: Vec<NativeView> = VIEWS.with(|v| {
                let mut v = v.borrow_mut();
                let dead: Vec<ViewKey> = v.keys().filter(|k| !keep.contains(k)).cloned().collect();
                dead.iter().filter_map(|k| v.remove(k)).collect()
            });
            drop(gone);
        }
    }
}

/// O `Fixed` dos nós, montando o encaixe na primeira vez: a webview do app
/// passa da `vbox` do Tauri para o filho principal de um `Overlay`.
fn layer(app: &AppHandle) -> Result<gtk::Fixed, String> {
    let ready = LAYER.with(|l| {
        l.borrow()
            .as_ref()
            .filter(|l| l.fixed.parent().is_some())
            .map(|l| l.fixed.clone())
    });
    if let Some(fixed) = ready {
        return Ok(fixed);
    }
    let window = app
        .get_webview_window("main")
        .ok_or("janela principal não encontrada")?;
    let vbox = window.default_vbox().map_err(|e| e.to_string())?;
    let app_view = vbox
        .children()
        .into_iter()
        .find_map(|w| w.downcast::<webkit2gtk::WebView>().ok())
        .ok_or("a webview do app não está na janela")?;
    let had_focus = app_view.has_focus();
    let overlay = gtk::Overlay::new();
    let fixed = gtk::Fixed::new();
    // `app_view` segura a referência enquanto troca de pai.
    vbox.remove(&app_view);
    overlay.add(&app_view);
    overlay.add_overlay(&fixed);
    overlay.set_overlay_pass_through(&fixed, true);
    vbox.pack_start(&overlay, true, true, 0);
    overlay.show_all();
    if had_focus {
        app_view.grab_focus();
    }
    LAYER.with(|l| {
        *l.borrow_mut() = Some(Layer {
            fixed: fixed.clone(),
            app: app_view,
        })
    });
    Ok(fixed)
}

/// A webview do app (para onde o foco volta).
fn app_view() -> Option<webkit2gtk::WebView> {
    LAYER.with(|l| l.borrow().as_ref().map(|l| l.app.clone()))
}

/// Devolve o foco do teclado ao app.
fn give_back() {
    if let Some(app) = app_view() {
        app.grab_focus();
    }
}

/// Posição, tamanho e zoom. Escondida = fora da área visível, com o mesmo
/// tamanho (a página continua lida pelo agente e o print sai).
fn set_place(app: &AppHandle, key: &ViewKey, view: &NativeView, place: &Place) {
    let Some(fixed) = view
        .wk
        .parent()
        .and_then(|p| p.downcast::<gtk::Fixed>().ok())
    else {
        return;
    };
    let visible = place.visible && place.rect.is_some();
    if let Some(r) = place.rect {
        view.wk
            .set_size_request(r.w.round() as i32, r.h.round() as i32);
        // `set_zoom_level`: reescala o layout como o `transform` do canvas.
        let _ = view.webview.zoom(place.zoom.clamp(0.25, 5.0));
    }
    if !visible && view.wk.has_focus() {
        give_back();
        if app.state::<BrowserViews>().focus(key) == Focus::Owner {
            app.state::<BrowserViews>().set_owner(app, None);
        }
    }
    match place.rect.filter(|_| visible) {
        Some(r) => fixed.move_(&view.wk, r.x.round() as i32, r.y.round() as i32),
        None => fixed.move_(&view.wk, OFFSCREEN, OFFSCREEN),
    }
}

fn create(app: &AppHandle, key: &ViewKey) -> Result<NativeView, String> {
    let fixed = layer(app)?;
    let roots: Arc<Mutex<Vec<PathBuf>>> = Arc::default();

    // Um contexto e uma pasta de dados por nó: cookies e armazenamento não
    // se misturam com os de outro nó nem com os do app. Sem pasta, incógnito.
    let dir = crate::session::cwd::home().map(|h| store_dir(&h, key));
    let dir = dir.filter(|d| match ensure_store_dir(d) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[tyba] navegador do orbit: pasta de dados: {e}");
            false
        }
    });
    let ephemeral = dir.is_none();
    let mut context = WebContext::new(dir);

    let nav_app = app.clone();
    let nav_key = key.clone();
    let load_app = app.clone();
    let load_key = key.clone();
    let title_app = app.clone();
    let title_key = key.clone();
    let open_app = app.clone();
    let open_key = key.clone();
    let open_roots = roots.clone();
    let serve_roots = roots.clone();

    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(false)
        .with_bounds(Rect {
            position: LogicalPosition::new(OFFSCREEN, OFFSCREEN).into(),
            size: LogicalSize::new(HIDDEN_W, HIDDEN_H).into(),
        })
        .with_clipboard(false)
        .with_devtools(false)
        .with_hotkeys_zoom(false)
        .with_back_forward_navigation_gestures(false)
        .with_autoplay(false)
        .with_incognito(ephemeral)
        // Toda navegação, de qualquer frame (`decide-policy`), pela política.
        .with_navigation_handler(move |url| match policy::decide(&url) {
            policy::Decision::Allow => true,
            policy::Decision::Deny(why) => {
                nav_app
                    .state::<BrowserViews>()
                    .on_refused(&nav_app, &nav_key, why);
                false
            }
        })
        // `target=_blank` / `window.open`: nada de janela nova; o endereço
        // abre no mesmo nó, pela mesma regra.
        .with_new_window_req_handler(move |url, _| {
            let roots = open_roots.lock().clone();
            let _ = open_app
                .state::<BrowserViews>()
                .navigate(&open_app, &open_key, &url, &roots);
            NewWindowResponse::Deny
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
        .with_url("about:blank")
        .build_gtk(&fixed)
        .map_err(|e| e.to_string())?;
    let wk = webview.webview();
    lock_down(app, key, &wk);
    // Mostrada, mas fora da área: realizada e com tamanho de página.
    wk.set_size_request(HIDDEN_W as i32, HIDDEN_H as i32);
    fixed.move_(&wk, OFFSCREEN, OFFSCREEN);
    wk.show_all();
    app.state::<BrowserViews>().on_created(app, key, ephemeral);
    Ok(NativeView {
        webview,
        wk,
        _context: context,
        roots,
    })
}

/// O que o wry não faz: tira o canal `ipc`, nega toda permissão, põe os
/// scripts do mundo isolado e liga o foco e o teclado do dono.
fn lock_down(app: &AppHandle, key: &ViewKey, wk: &webkit2gtk::WebView) {
    if let Some(manager) = wk.user_content_manager() {
        // O wry registra `ipc` mesmo sem `ipc_handler` (e ignora o que
        // chega): sem ele, a página não tem nem o objeto.
        manager.unregister_script_message_handler("ipc");
        manager.add_script(&UserScript::for_world(
            ISOLATED_JS,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::Start,
            WORLD,
            &[],
            &[],
        ));
        manager.add_script(&UserScript::new(
            PAGE_SHIM_JS,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::Start,
            &[],
            &[],
        ));
    }

    // Câmera, microfone, localização, notificação…: sempre negados.
    wk.connect_permission_request(|_, request| {
        request.deny();
        true
    });

    // Foco: clicar na página marca "você"; sair dela devolve.
    let in_app = app.clone();
    let in_key = key.clone();
    wk.connect_focus_in_event(move |_, _| {
        let views = in_app.state::<BrowserViews>();
        views.set_owner(&in_app, Some(&in_key));
        views.mark_dirty(&in_key);
        gtk::glib::Propagation::Proceed
    });
    let out_app = app.clone();
    let out_key = key.clone();
    wk.connect_focus_out_event(move |_, _| {
        let views = out_app.state::<BrowserViews>();
        if views.focus(&out_key) == Focus::Owner {
            views.set_owner(&out_app, None);
        }
        gtk::glib::Propagation::Proceed
    });

    // Teclado: o sinal roda antes do tratamento padrão da página. Esc
    // devolve, ⇧Esc vai para a página, Ctrl+W/K/L voltam ao app (e o atalho
    // vai junto).
    let key_app = app.clone();
    let key_node = key.clone();
    wk.connect_key_press_event(move |_, event| {
        use gtk::gdk::ModifierType;
        let state = event.state();
        let mods = Mods {
            shift: state.contains(ModifierType::SHIFT_MASK),
            control: false,
            option: state.contains(ModifierType::MOD1_MASK),
            command: state.contains(ModifierType::CONTROL_MASK),
        };
        let keyval = event.keyval();
        let chars = keyval
            .to_lower()
            .to_unicode()
            .filter(char::is_ascii_alphabetic)
            .map(String::from)
            .unwrap_or_default();
        let escape = keyval == gtk::gdk::keys::constants::Escape;
        let views = key_app.state::<BrowserViews>();
        match route(escape, mods, &chars) {
            KeyRoute::Page => {
                views.mark_dirty(&key_node);
                gtk::glib::Propagation::Proceed
            }
            way => {
                give_back();
                views.set_owner(&key_app, None);
                if way == KeyRoute::ToApp {
                    if let (Some(js), Some(w)) = (
                        forward_shortcut_js(&chars),
                        key_app.get_webview_window("main"),
                    ) {
                        let _ = w.eval(js);
                    }
                }
                gtk::glib::Propagation::Stop
            }
        }
    });
}

/// `call_async_javascript_function` no mundo isolado: o resultado (JSON em
/// string) ou o erro volta por `reply`.
fn eval(
    view: &NativeView,
    action: &str,
    args: &str,
    reply: std::sync::mpsc::Sender<Result<String, String>>,
) {
    let body = format!("{ISOLATED_JS}\n{RUN_JS}");
    let arguments = gtk::glib::VariantDict::new(None);
    arguments.insert("action", action);
    arguments.insert("args", args);
    let arguments = arguments.end();
    view.wk.call_async_javascript_function(
        &body,
        Some(&arguments),
        Some(WORLD),
        None,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let out = match result {
                Err(e) => Err(format!(
                    "a página não aceitou a ação: {}",
                    super::snapshot::one_line(e.message(), 200)
                )),
                Ok(v) if v.is_string() => Ok(v.to_str().to_string()),
                Ok(_) => Err("a página devolveu uma resposta ilegível".to_string()),
            };
            let _ = reply.send(out);
        },
    );
}

/// O print da área visível da página, reduzido para caber em `max_w` de
/// largura, em `format` (`"png"` ou `"jpeg"`): os bytes, a largura e a altura.
fn shot(
    wk: &webkit2gtk::WebView,
    format: &'static str,
    max_w: f64,
    done: impl FnOnce(Result<(Vec<u8>, u32, u32), String>) + 'static,
) {
    wk.snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let out = (|| {
                let surface = result.map_err(|_| "o print veio vazio".to_string())?;
                let image = gtk::cairo::ImageSurface::try_from(surface.clone())
                    .map_err(|_| "o print veio vazio".to_string())?;
                let (sx, sy) = surface.device_scale();
                let w = (f64::from(image.width()) / sx.max(1.0)).round() as i32;
                let h = (f64::from(image.height()) / sy.max(1.0)).round() as i32;
                if w < 10 || h < 10 {
                    return Err(
                        "o navegador está escondido sem tamanho; abra o Orbit com o nó à vista"
                            .to_string(),
                    );
                }
                let mut pixbuf = gtk::gdk::pixbuf_get_from_surface(&surface, 0, 0, w, h)
                    .ok_or("o print veio vazio")?;
                if f64::from(pixbuf.width()) > max_w {
                    let nw = max_w as i32;
                    let nh = (f64::from(pixbuf.height()) * max_w / f64::from(pixbuf.width()))
                        .round()
                        .max(1.0) as i32;
                    pixbuf = pixbuf
                        .scale_simple(nw, nh, gtk::gdk_pixbuf::InterpType::Bilinear)
                        .ok_or("o print não reduziu")?;
                }
                let options: &[(&str, &str)] = if format == "jpeg" {
                    &[("quality", "70")]
                } else {
                    &[]
                };
                let bytes = pixbuf
                    .save_to_bufferv(format, options)
                    .map_err(|_| "o print não virou imagem".to_string())?;
                Ok((
                    bytes,
                    pixbuf.width().max(0) as u32,
                    pixbuf.height().max(0) as u32,
                ))
            })();
            done(out);
        },
    );
}
