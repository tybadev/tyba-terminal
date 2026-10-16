//! A view nativa do navegador no Windows: um WebView2 por nó, numa janela
//! filha (HWND) da janela principal, por cima da webview do app, criada pelo
//! `wry` com `build_as_child` (o wry põe a filha em `HWND_TOP`).
//!
//! Tudo aqui roda na main thread (o WebView2 é de apartamento único): o
//! registro das views é `thread_local` e as operações chegam por [`apply`].
//!
//! O que fecha o canal entre a página e o app, por construção:
//! - nenhum `with_ipc_handler`, `IsWebMessageEnabled = false` (sem
//!   `chrome.webview.postMessage`) e `AreHostObjectsAllowed = false`;
//! - um ambiente WebView2 próprio por nó (pasta de dados própria), sem o
//!   `WebResourceRequested` do Tauri: só o `orbit-file` nosso é servido;
//! - toda permissão (câmera, microfone, localização, notificação, área de
//!   transferência) negada no `PermissionRequested`;
//! - toda navegação conferida pela política: o frame principal pelo
//!   `NavigationStarting` do wry, os frames internos pelo
//!   `FrameNavigationStarting` nosso;
//! - sem download, sem devtools para o usuário.
//!
//! O mundo isolado é o do Chromium, pelo protocolo do DevTools chamado pela
//! API (`CallDevToolsProtocolMethod`, nunca porta de depuração):
//! `Page.addScriptToEvaluateOnNewDocument {worldName}` põe o `isolated.js`
//! no início de cada documento, e cada ação é `Runtime.callFunctionOn` no
//! contexto de `Page.createIsolatedWorld` (o mesmo mundo, achado pelo nome),
//! com `action` e `args` como argumentos, nunca colados no código.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, COREWEBVIEW2_PERMISSION_STATE_DENY,
};
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler,
    AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CallDevToolsProtocolMethodCompletedHandler, FocusChangedEventHandler,
    NavigationStartingEventHandler, PermissionRequestedEventHandler,
};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT,
};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebViewBuilder, WebViewExtWindows};

use super::native::{
    png_size, serve_file, HIDDEN_H, HIDDEN_W, ISOLATED_JS, PAGE_SHIM_JS, PREVIEW_MAX_W, RUN_JS,
    WORLD,
};
use super::{
    ensure_store_dir, forward_shortcut_js, policy, route, store_dir, BrowserViews, Focus, KeyRoute,
    Mods, Op, Place, ViewKey,
};

struct NativeView {
    webview: wry::WebView,
    wv: ICoreWebView2,
    roots: Arc<Mutex<Vec<PathBuf>>>,
}

thread_local! {
    static VIEWS: RefCell<HashMap<ViewKey, NativeView>> = RefCell::new(HashMap::new());
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
                    set_place(view, &place);
                    // O WebView2 só carrega esquema próprio como
                    // `http://orbit-file.<host>/…` (o wry traduz de volta).
                    if let Err(e) = view.webview.load_url(&policy::to_windows(&url)) {
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
                // SAFETY: main thread; o WebView2 vive enquanto a entrada.
                let _ = unsafe { view.wv.GoBack() };
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
        Op::Capture { key, reply } => VIEWS.with(|v| match v.borrow().get(&key) {
            Some(view) => capture(app, view, reply),
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
            Some(view) => eval(view, action, args, reply),
            None => {
                let _ = reply.send(Err(super::NOT_OPEN.to_string()));
            }
        }),
        Op::Retain(keep) => destroy(|k| !keep.contains(k), false),
        Op::Destroy { key, give_back } => destroy(|k| *k == key, give_back),
        // A view já fechou (o `Destroy` veio antes): a pasta sai numa thread
        // própria, que espera o processo do WebView2 soltar os arquivos.
        Op::RemoveStore { key, reload, .. } => super::remove_store_later(app, key, reload),
    }
}

/// Destrói as views de `dead`; com `give_back`, o teclado volta ao app antes
/// (o dono estava na página). Tira do registro antes de soltar: o `Drop` do
/// wry destrói a janela filha e pode disparar eventos.
fn destroy(dead: impl Fn(&ViewKey) -> bool, give_back: bool) {
    let gone: Vec<NativeView> = VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        let keys: Vec<ViewKey> = v.keys().filter(|k| dead(k)).cloned().collect();
        keys.iter().filter_map(|k| v.remove(k)).collect()
    });
    if give_back {
        for view in &gone {
            let _ = view.webview.focus_parent();
        }
    }
    drop(gone);
}

/// Posição, zoom e visibilidade. Mostra só depois de posicionar.
fn set_place(view: &NativeView, place: &Place) {
    let visible = place.visible && place.rect.is_some();
    if let Some(r) = place.rect {
        let _ = view.webview.set_bounds(Rect {
            position: LogicalPosition::new(r.x, r.y).into(),
            size: LogicalSize::new(r.w, r.h).into(),
        });
        // `SetZoomFactor`: reescala o layout como o `transform` do canvas.
        let _ = view.webview.zoom(place.zoom.clamp(0.25, 5.0));
    }
    if !visible && place.give_back {
        // A view vai sumir com o teclado nela: o foco volta ao app antes.
        let _ = view.webview.focus_parent();
    }
    let _ = view.webview.set_visible(visible);
}

fn create(app: &AppHandle, key: &ViewKey) -> Result<NativeView, String> {
    let window = app
        .get_webview_window("main")
        .ok_or("janela principal não encontrada")?;
    let roots: Arc<Mutex<Vec<PathBuf>>> = Arc::default();

    // Uma pasta de dados por nó: cookies e armazenamento não se misturam com
    // os de outro nó nem com os do app. Sem pasta (sem home), incógnito.
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

    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(false)
        // Escondida, mas com tamanho de página: o agente lê e clica nela
        // antes de o card aparecer (o front manda o retângulo de verdade).
        .with_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(HIDDEN_W, HIDDEN_H).into(),
        })
        .with_clipboard(false)
        .with_devtools(false)
        .with_hotkeys_zoom(false)
        .with_back_forward_navigation_gestures(false)
        .with_autoplay(false)
        .with_incognito(ephemeral)
        // O frame principal (o wry só olha o `NavigationStarting`).
        .with_navigation_handler(move |url| refuse_or_allow(&nav_app, &nav_key, &url))
        // `target=_blank` / `window.open`: nada de janela nova; o endereço
        // abre no mesmo nó, pela mesma regra.
        // Sem url ou `about:blank`, nada: a página do nó fica. O `navigate`
        // confere o disco (`file://`), então fora da main thread.
        .with_new_window_req_handler(move |url, _| {
            let url = policy::from_windows(&url);
            if !super::new_window_ignored(&url) {
                let roots = open_roots.lock().clone();
                let (app, key) = (open_app.clone(), open_key.clone());
                std::thread::spawn(move || {
                    let _ = app
                        .state::<BrowserViews>()
                        .navigate(&app, &key, &url, &roots);
                });
            }
            NewWindowResponse::Deny
        })
        .with_download_started_handler(|_, _| false)
        .with_on_page_load_handler(move |event, url| {
            let finished = matches!(event, PageLoadEvent::Finished);
            load_app.state::<BrowserViews>().on_load(
                &load_app,
                &load_key,
                &policy::from_windows(&url),
                finished,
            );
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
    let webview = builder.build_as_child(&window).map_err(|e| e.to_string())?;
    let wv = webview.webview();
    // SAFETY: main thread; o WebView2 acabou de nascer e é nosso.
    unsafe { lock_down(app, key, &webview, &wv) }.map_err(|e| e.message().to_string())?;
    app.state::<BrowserViews>().on_created(app, key, ephemeral);
    Ok(NativeView { webview, wv, roots })
}

/// A política numa navegação (de qualquer frame): recusa vira estado do card.
fn refuse_or_allow(app: &AppHandle, key: &ViewKey, url: &str) -> bool {
    match policy::decide(&policy::from_windows(url)) {
        policy::Decision::Allow => true,
        policy::Decision::Deny(why) => {
            app.state::<BrowserViews>().on_refused(app, key, why);
            false
        }
    }
}

/// O que o wry não faz: sem canal para a página, toda permissão negada,
/// frames internos pela política, os scripts do mundo isolado e o foco e o
/// teclado do dono.
///
/// # Safety
/// Main thread, com o WebView2 vivo.
unsafe fn lock_down(
    app: &AppHandle,
    key: &ViewKey,
    webview: &wry::WebView,
    wv: &ICoreWebView2,
) -> windows::core::Result<()> {
    let mut token = 0i64;

    let settings = wv.Settings()?;
    settings.SetIsWebMessageEnabled(false)?;
    settings.SetAreHostObjectsAllowed(false)?;

    wv.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
            }
            Ok(())
        })),
        &mut token,
    )?;

    let frame_app = app.clone();
    let frame_key = key.clone();
    wv.add_FrameNavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut uri = PWSTR::null();
            args.Uri(&mut uri)?;
            let uri = take_pwstr(uri);
            if !refuse_or_allow(&frame_app, &frame_key, &uri) {
                args.SetCancel(true)?;
            }
            Ok(())
        })),
        &mut token,
    )?;

    // O `page_shim.js` no mundo da página (todo documento novo)…
    wv.AddScriptToExecuteOnDocumentCreated(
        &HSTRING::from(PAGE_SHIM_JS),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(()))),
    )?;
    // …e o `isolated.js` no mundo `tyba-orbit`, pelo DevTools.
    cdp(wv, "Page.enable", json!({}), |_| {});
    cdp(
        wv,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": ISOLATED_JS, "worldName": WORLD }),
        |r| {
            if let Err(e) = r {
                eprintln!("[tyba] navegador do orbit: script isolado: {e}");
            }
        },
    );

    // Foco: clicar na página marca "você"; sair dela devolve.
    let controller = webview.controller();
    let got_app = app.clone();
    let got_key = key.clone();
    controller.add_GotFocus(
        &FocusChangedEventHandler::create(Box::new(move |_, _| {
            let views = got_app.state::<BrowserViews>();
            views.set_owner(&got_app, Some(&got_key));
            views.mark_dirty(&got_key);
            Ok(())
        })),
        &mut token,
    )?;
    let lost_app = app.clone();
    let lost_key = key.clone();
    controller.add_LostFocus(
        &FocusChangedEventHandler::create(Box::new(move |_, _| {
            let views = lost_app.state::<BrowserViews>();
            if views.focus(&lost_key) == Focus::Owner {
                views.set_owner(&lost_app, None);
            }
            Ok(())
        })),
        &mut token,
    )?;

    // Teclado: a tecla passa pelo app antes da página. Esc devolve, ⇧Esc vai
    // para a página, Ctrl+W/K/L voltam ao app (e o atalho vai junto).
    let key_app = app.clone();
    let key_node = key.clone();
    controller.add_AcceleratorKeyPressed(
        &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
            args.KeyEventKind(&mut kind)?;
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
            {
                return Ok(());
            }
            let mut vk = 0u32;
            args.VirtualKey(&mut vk)?;
            let down = |k: u16| GetKeyState(i32::from(k)) < 0;
            let mods = Mods {
                shift: down(VK_SHIFT.0),
                control: false,
                option: down(VK_MENU.0),
                command: down(VK_CONTROL.0),
            };
            // Letra: o código virtual de A–Z é o próprio ASCII maiúsculo.
            let chars = char::from_u32(vk)
                .filter(char::is_ascii_uppercase)
                .map(|c| c.to_ascii_lowercase().to_string())
                .unwrap_or_default();
            let escape = vk == u32::from(VK_ESCAPE.0);
            let views = key_app.state::<BrowserViews>();
            match route(escape, mods, &chars) {
                KeyRoute::Page => views.mark_dirty(&key_node),
                way => {
                    args.SetHandled(true)?;
                    give_back(&key_node);
                    views.set_owner(&key_app, None);
                    if way == KeyRoute::ToApp {
                        if let (Some(js), Some(w)) = (
                            forward_shortcut_js(&chars),
                            key_app.get_webview_window("main"),
                        ) {
                            let _ = w.eval(js);
                        }
                    }
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    Ok(())
}

/// Devolve o foco do teclado ao app: o foco na janela principal volta para
/// a webview do app (o wry move o foco no `WM_SETFOCUS` dela).
fn give_back(key: &ViewKey) {
    VIEWS.with(|v| {
        if let Some(view) = v.borrow().get(key) {
            let _ = view.webview.focus_parent();
        }
    });
}

/// Um método do DevTools na página (pela API, nunca por porta). O resultado
/// em JSON volta em `done`, na main thread.
fn cdp(
    wv: &ICoreWebView2,
    method: &str,
    params: Value,
    done: impl FnOnce(Result<Value, String>) + 'static,
) {
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
        move |result, json: String| {
            done(
                result
                    .map_err(|e| e.message().to_string())
                    .and_then(|()| serde_json::from_str(&json).map_err(|e| e.to_string())),
            );
            Ok(())
        },
    ));
    // SAFETY: main thread; o WebView2 vive enquanto a entrada.
    let sent = unsafe {
        wv.CallDevToolsProtocolMethod(
            &HSTRING::from(method),
            &HSTRING::from(params.to_string()),
            &handler,
        )
    };
    if let Err(e) = sent {
        eprintln!("[tyba] navegador do orbit: {method}: {}", e.message());
    }
}

/// A ação no mundo isolado: acha o frame principal, o contexto do mundo
/// `tyba-orbit` (o mesmo, pelo nome) e chama a função com os argumentos.
fn eval(
    view: &NativeView,
    action: &str,
    args: String,
    reply: std::sync::mpsc::Sender<Result<String, String>>,
) {
    let wv = view.wv.clone();
    let action = action.to_string();
    cdp(&view.wv, "Page.getFrameTree", json!({}), move |tree| {
        let frame = match tree.map(|t| t["frameTree"]["frame"]["id"].as_str().map(String::from)) {
            Ok(Some(f)) => f,
            _ => {
                let _ = reply.send(Err("a página não respondeu".into()));
                return;
            }
        };
        let wv2 = wv.clone();
        cdp(
            &wv,
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": WORLD, "grantUniveralAccess": false }),
            move |world| {
                let Some(context) = world.ok().and_then(|w| w["executionContextId"].as_i64())
                else {
                    let _ = reply.send(Err("a página não abriu o mundo isolado".into()));
                    return;
                };
                let function =
                    format!("async function (action, args) {{\n{ISOLATED_JS}\n{RUN_JS}\n}}");
                cdp(
                    &wv2,
                    "Runtime.callFunctionOn",
                    json!({
                        "functionDeclaration": function,
                        "executionContextId": context,
                        "arguments": [{ "value": action }, { "value": args }],
                        "awaitPromise": true,
                        "returnByValue": true,
                    }),
                    move |out| {
                        let _ = reply.send(call_result(out));
                    },
                );
            },
        );
    });
}

/// A resposta do `Runtime.callFunctionOn`: a string devolvida, ou o erro.
fn call_result(out: Result<Value, String>) -> Result<String, String> {
    let out = out.map_err(|e| {
        format!(
            "a página não aceitou a ação: {}",
            super::snapshot::one_line(&e, 200)
        )
    })?;
    if let Some(ex) = out.get("exceptionDetails") {
        let text = ex["exception"]["description"]
            .as_str()
            .or_else(|| ex["text"].as_str())
            .unwrap_or("erro");
        return Err(format!(
            "a página não aceitou a ação: {}",
            super::snapshot::one_line(text, 200)
        ));
    }
    out["result"]["value"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "a página devolveu uma resposta ilegível".to_string())
}

/// O tamanho da página em px CSS (`Page.getLayoutMetrics`), e a escala da
/// tela da janela principal.
fn page_size(
    app: &AppHandle,
    wv: &ICoreWebView2,
    done: impl FnOnce(Option<(f64, f64, f64)>) + 'static,
) {
    let scale = app
        .get_webview_window("main")
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0)
        .max(1.0);
    cdp(wv, "Page.getLayoutMetrics", json!({}), move |m| {
        let size = m.ok().and_then(|m| {
            let v = &m["cssLayoutViewport"];
            Some((v["clientWidth"].as_f64()?, v["clientHeight"].as_f64()?))
        });
        done(
            size.filter(|(w, h)| *w >= 10.0 && *h >= 10.0)
                .map(|(w, h)| (w, h, scale)),
        );
    });
}

/// `Page.captureScreenshot` da página inteira à vista, reduzida para caber
/// em `max_w` pixels de largura.
fn shot(
    app: &AppHandle,
    wv: &ICoreWebView2,
    format: &'static str,
    max_w: f64,
    done: impl FnOnce(Result<String, String>) + 'static,
) {
    let wv2 = wv.clone();
    page_size(app, wv, move |size| {
        let Some((w, h, scale)) = size else {
            done(Err(
                "o navegador está escondido sem tamanho; abra o Orbit com o nó à vista".into(),
            ));
            return;
        };
        let zoom = (max_w / (w * scale)).min(1.0);
        let mut params = json!({
            "format": format,
            "clip": { "x": 0, "y": 0, "width": w, "height": h, "scale": zoom },
        });
        if format == "jpeg" {
            params["quality"] = json!(70);
        }
        cdp(&wv2, "Page.captureScreenshot", params, move |r| {
            done(r.and_then(|v| {
                v["data"]
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| "o print veio vazio".to_string())
            }));
        });
    });
}

/// A prévia do card: JPEG em base64 → [`BrowserViews::on_snapshot`].
fn snapshot(app: &AppHandle, key: &ViewKey, view: &NativeView) {
    let done_app = app.clone();
    let done_key = key.clone();
    let scale = app
        .get_webview_window("main")
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0)
        .max(1.0);
    shot(app, &view.wv, "jpeg", PREVIEW_MAX_W * scale, move |r| {
        if let Ok(b64) = r {
            done_app
                .state::<BrowserViews>()
                .on_snapshot(&done_app, &done_key, b64);
        }
    });
}

/// O print do agente: PNG até [`super::MAX_SHOT_PX`] de largura. O print
/// **não** é redigido (fica na pasta do run, 0600).
fn capture(
    app: &AppHandle,
    view: &NativeView,
    reply: std::sync::mpsc::Sender<Result<super::Shot, String>>,
) {
    use base64::Engine;
    shot(app, &view.wv, "png", super::MAX_SHOT_PX, move |r| {
        let out = r.and_then(|b64| {
            let png = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| "o print veio ilegível".to_string())?;
            let (width, height) = png_size(&png).ok_or("o print não é PNG")?;
            Ok(super::Shot { png, width, height })
        });
        let _ = reply.send(out);
    });
}
