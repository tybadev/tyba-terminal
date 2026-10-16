//! Navegador nativo do nó (Fase 5, `fase5-navegador`).
//!
//! Cada nó de navegador ganha um webview nativo próprio, preso à janela
//! principal e sem IPC do Tauri dentro dele (nenhum `ipc_handler`, nenhum
//! message handler: a página não tem canal para o app). O estado (url,
//! título, foco, último print) vive aqui em [`BrowserViews`]; o front manda só
//! a intenção — geometria (`orbit_browser_view`) e navegação — e desenha o
//! que o core emite em [`EVENT`].
//!
//! O webview nativo só existe na main thread: cada operação vira um [`Op`]
//! entregue por `run_on_main_thread` ao módulo do sistema (`macos.rs`). Por
//! enquanto só o macOS tem a view nativa; Windows e Linux seguem no iframe.

pub mod policy;

#[cfg(target_os = "macos")]
mod macos;

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use super::{EdgeKind, Graph};

/// O evento com o estado de um navegador (`{graph, node, state}`).
pub const EVENT: &str = "orbit-browser";
/// Teto de views vivas ao mesmo tempo (cada WKWebView é um processo).
pub const MAX_LIVE: usize = 4;
/// Faixa de zoom do canvas em que a view fica viva; fora dela, só o print.
pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2.0;
/// Teto do título guardado (caracteres).
const MAX_TITLE_CHARS: usize = 200;

/// Qual navegador: o nó dentro do grafo.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ViewKey {
    pub graph: String,
    pub node: String,
}

/// Retângulo do marcador, em px CSS da webview do app (= pontos lógicos da
/// janela no macOS).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl ViewRect {
    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    fn valid(&self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
            && self.w >= 1.0
            && self.h >= 1.0
            && self.w <= 20_000.0
            && self.h <= 20_000.0
    }
}

/// Quem está com a página.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Focus {
    #[default]
    None,
    /// O dono clicou na página: teclado e mouse são dele.
    Owner,
    /// Um agente agiu há pouco (por script, sem roubar o teclado).
    Agent,
}

/// O que o card desenha.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    /// A url que o dono reconhece (`orbit-file` volta a `file://`).
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub focus: Focus,
    /// A view nativa está à vista; senão o card mostra o último print.
    pub live: bool,
    /// Quando o último print foi tirado (ms); o front busca a imagem.
    pub preview_at: Option<u64>,
    /// A última navegação recusada, com o motivo.
    pub refused: Option<String>,
    /// macOS 11–13: armazenamento não persistente (o login não fica salvo).
    pub ephemeral: bool,
}

#[derive(Clone, Serialize)]
struct BrowserEvent<'a> {
    graph: &'a str,
    node: &'a str,
    state: &'a BrowserState,
}

/// Onde e como a view nativa fica.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub rect: Option<ViewRect>,
    pub zoom: f64,
    pub visible: bool,
}

/// Operação na view nativa, executada na main thread.
#[derive(Debug, Clone)]
pub enum Op {
    /// Cria a view se preciso e carrega `url` (já aprovada pela política).
    Navigate {
        key: ViewKey,
        url: String,
        roots: Vec<PathBuf>,
        place: Place,
    },
    Place {
        key: ViewKey,
        place: Place,
    },
    Back(ViewKey),
    Reload(ViewKey),
    /// Destrói toda view que não está na lista.
    Retain(Vec<ViewKey>),
}

#[derive(Debug, Default)]
struct Entry {
    state: BrowserState,
    rect: Option<ViewRect>,
    zoom: f64,
    /// Raízes canônicas do `orbit-file` (para mostrar a url).
    roots: Vec<PathBuf>,
}

/// O registro dos navegadores nativos (estado no core).
#[derive(Default)]
pub struct BrowserViews {
    entries: Mutex<HashMap<ViewKey, Entry>>,
    /// Centro da área do canvas (o mais perto dele fica vivo primeiro).
    center: Mutex<(f64, f64)>,
}

/// O navegador nativo existe neste sistema? (Senão o front usa o iframe.)
pub const NATIVE: bool = cfg!(target_os = "macos");

/// 16 bytes estáveis por nó para o armazenamento (cookies, localStorage):
/// os mesmos entre aberturas, nunca os mesmos entre nós ou grafos.
pub fn store_identity(key: &ViewKey) -> [u8; 16] {
    let mut h = Sha256::new();
    h.update(b"tyba-orbit-browser\0");
    h.update(key.graph.as_bytes());
    h.update(b"\0");
    h.update(key.node.as_bytes());
    let digest = h.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

/// Quais views ficam vivas: das que o front quer à vista, a do dono primeiro
/// e depois as mais perto do centro, até [`MAX_LIVE`]. Devolve os índices.
pub fn pick_live(wanted: &[(ViewRect, bool)], center: (f64, f64), max: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    let dist = |r: &ViewRect| {
        let (x, y) = r.center();
        (x - center.0).powi(2) + (y - center.1).powi(2)
    };
    order.sort_by(|&a, &b| {
        let (ra, oa) = &wanted[a];
        let (rb, ob) = &wanted[b];
        ob.cmp(oa).then(dist(ra).total_cmp(&dist(rb)))
    });
    order.truncate(max);
    order.sort_unstable();
    order
}

/// O título como o card e o agente o veem: uma linha, sem controle, curto.
pub fn clean_page_title(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_control() || super::breaks_line(c) {
                ' '
            } else {
                c
            }
        })
        .take(MAX_TITLE_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Entrega a operação à main thread (no macOS); nos outros sistemas não há
/// view nativa e nada acontece.
fn dispatch(app: &AppHandle, op: Op) {
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        if let Err(e) = app.run_on_main_thread(move || macos::apply(&handle, op)) {
            eprintln!("[tyba] navegador do orbit: {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, op);
}

impl BrowserViews {
    fn emit(app: &AppHandle, key: &ViewKey, state: &BrowserState) {
        let _ = app.emit(
            EVENT,
            BrowserEvent {
                graph: &key.graph,
                node: &key.node,
                state,
            },
        );
    }

    /// Muda o estado de um navegador e emite, se mudou.
    fn update(&self, app: &AppHandle, key: &ViewKey, f: impl FnOnce(&mut Entry)) {
        let state = {
            let mut entries = self.entries.lock();
            let entry = entries.entry(key.clone()).or_insert_with(|| Entry {
                zoom: 1.0,
                ..Entry::default()
            });
            let before = entry.state.clone();
            f(entry);
            if entry.state == before {
                return;
            }
            entry.state.clone()
        };
        Self::emit(app, key, &state);
    }

    pub fn state(&self, key: &ViewKey) -> Option<BrowserState> {
        self.entries.lock().get(key).map(|e| e.state.clone())
    }

    /// Navega: `file://` vira `orbit-file` se o arquivo está numa das
    /// `roots`; o resto passa pela [`policy::decide`]. A recusa fica no
    /// estado (o card mostra) e volta como erro.
    pub fn navigate(
        &self,
        app: &AppHandle,
        key: &ViewKey,
        url: &str,
        roots: &[PathBuf],
    ) -> Result<(), String> {
        let roots = policy::canonical_roots(roots);
        let target = if url
            .trim()
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
        {
            policy::resolve_file(url, &roots).map_err(|r| r.message().to_string())
        } else {
            match policy::decide(url) {
                policy::Decision::Allow => Ok(url.trim().to_string()),
                policy::Decision::Deny(why) => Err(why.to_string()),
            }
        };
        let target = match target {
            Ok(t) => t,
            Err(why) => {
                self.update(app, key, |e| e.state.refused = Some(why.clone()));
                return Err(why);
            }
        };
        let mut place = None;
        self.update(app, key, |e| {
            e.roots = roots.clone();
            e.state.refused = None;
            e.state.url = policy::display_url(&target, &roots);
            e.state.loading = true;
            place = Some(Place {
                rect: e.rect,
                zoom: e.zoom,
                visible: e.state.live,
            });
        });
        let place = place.unwrap_or(Place {
            rect: None,
            zoom: 1.0,
            visible: false,
        });
        dispatch(
            app,
            Op::Navigate {
                key: key.clone(),
                url: target,
                roots,
                place,
            },
        );
        Ok(())
    }

    /// A intenção de geometria do front: `rect` = o marcador inteiro à
    /// vista e nada por cima; `None` = esconder (o card mostra o print).
    /// Recalcula quem fica vivo (teto [`MAX_LIVE`]) e reposiciona.
    pub fn set_view(
        &self,
        app: &AppHandle,
        key: &ViewKey,
        rect: Option<ViewRect>,
        zoom: f64,
        center: Option<(f64, f64)>,
    ) {
        let rect = rect.filter(ViewRect::valid);
        let zoom = if zoom.is_finite() {
            zoom.clamp(0.1, 5.0)
        } else {
            1.0
        };
        if let Some(c) = center.filter(|c| c.0.is_finite() && c.1.is_finite()) {
            *self.center.lock() = c;
        }
        let center = *self.center.lock();
        let mut ops = Vec::new();
        let mut changed = Vec::new();
        {
            let mut entries = self.entries.lock();
            let entry = entries.entry(key.clone()).or_insert_with(|| Entry {
                zoom: 1.0,
                ..Entry::default()
            });
            let moved = entry.rect != rect || entry.zoom != zoom;
            entry.rect = rect;
            entry.zoom = zoom;
            // Só as do mesmo grafo disputam o teto (o Orbit mostra um).
            let keys: Vec<ViewKey> = entries
                .iter()
                .filter(|(k, e)| k.graph == key.graph && e.rect.is_some())
                .map(|(k, _)| k.clone())
                .collect();
            let wanted: Vec<(ViewRect, bool)> = keys
                .iter()
                .map(|k| {
                    let e = &entries[k];
                    (
                        e.rect.unwrap_or(ViewRect {
                            x: 0.0,
                            y: 0.0,
                            w: 0.0,
                            h: 0.0,
                        }),
                        e.state.focus == Focus::Owner,
                    )
                })
                .collect();
            let live: Vec<ViewKey> = pick_live(&wanted, center, MAX_LIVE)
                .into_iter()
                .map(|i| keys[i].clone())
                .collect();
            for (k, e) in entries.iter_mut().filter(|(k, _)| k.graph == key.graph) {
                let now_live = live.contains(k);
                if now_live != e.state.live || (k == key && moved) {
                    e.state.live = now_live;
                    ops.push(Op::Place {
                        key: k.clone(),
                        place: Place {
                            rect: e.rect,
                            zoom: e.zoom,
                            visible: now_live,
                        },
                    });
                    changed.push((k.clone(), e.state.clone()));
                }
            }
        }
        for (k, s) in &changed {
            Self::emit(app, k, s);
        }
        for op in ops {
            dispatch(app, op);
        }
    }

    pub fn back(&self, app: &AppHandle, key: &ViewKey) {
        dispatch(app, Op::Back(key.clone()));
    }

    pub fn reload(&self, app: &AppHandle, key: &ViewKey) {
        dispatch(app, Op::Reload(key.clone()));
    }

    /// Só os navegadores de `nodes` no grafo à vista seguem existindo; o
    /// resto (nó apagado, outro grafo, Orbit fechado) é destruído.
    pub fn retain(&self, app: &AppHandle, graph: &str, nodes: &[String]) {
        let keep: Vec<ViewKey> = nodes
            .iter()
            .map(|n| ViewKey {
                graph: graph.to_string(),
                node: n.clone(),
            })
            .collect();
        self.entries.lock().retain(|k, _| keep.contains(k));
        dispatch(app, Op::Retain(keep));
    }

    // ── chamados pela view nativa (main thread) ──────────────────────────

    /// A view nasceu: o armazenamento é não persistente neste macOS?
    pub fn on_created(&self, app: &AppHandle, key: &ViewKey, ephemeral: bool) {
        self.update(app, key, |e| e.state.ephemeral = ephemeral);
    }

    /// Começou (`finished = false`) ou terminou de carregar `url`.
    pub fn on_load(&self, app: &AppHandle, key: &ViewKey, url: &str, finished: bool) {
        self.update(app, key, |e| {
            e.state.url = policy::display_url(url, &e.roots);
            e.state.loading = !finished;
            if !finished {
                e.state.refused = None;
            }
        });
    }

    pub fn on_title(&self, app: &AppHandle, key: &ViewKey, title: &str) {
        let title = clean_page_title(title);
        self.update(app, key, |e| e.state.title = title);
    }

    /// A política recusou uma navegação (de qualquer frame).
    pub fn on_refused(&self, app: &AppHandle, key: &ViewKey, why: &str) {
        self.update(app, key, |e| e.state.refused = Some(why.to_string()));
    }

    /// Quem está com a página agora.
    pub fn focus(&self, key: &ViewKey) -> Focus {
        self.entries
            .lock()
            .get(key)
            .map_or(Focus::None, |e| e.state.focus)
    }
}

/// As pastas de onde o `orbit-file` deste navegador pode servir: a pasta do
/// workspace (`Graph::dir`) e a de cada nó ligado a ele (quem manda a url,
/// quem o controla). Sem duplicata, na ordem: o índice vira o host `r<i>`.
pub fn file_roots(graph: &Graph, node_id: &str) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut push = |dir: &str| {
        let p = PathBuf::from(dir);
        if !dir.is_empty() && p.is_absolute() && !roots.contains(&p) {
            roots.push(p);
        }
    };
    push(&graph.dir);
    for e in graph
        .edges
        .iter()
        .filter(|e| e.to == node_id && matches!(e.kind, EdgeKind::Url | EdgeKind::Control))
    {
        if let Some(n) = graph.nodes.iter().find(|n| n.id == e.from) {
            push(graph.node_dir(n));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orbit::{Edge, Node, NodeKind, Position, Size};

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { w: 10.0, h: 10.0 },
        }
    }

    #[test]
    fn roots_are_the_workspace_and_the_linked_nodes() {
        let process = |dir: &str| NodeKind::Process {
            command: "x".into(),
            output: Default::default(),
            file: None,
            dir: dir.into(),
        };
        let graph = Graph {
            id: "g".into(),
            name: "g".into(),
            dir: "/ws".into(),
            nodes: vec![
                node("b", NodeKind::Browser { url: String::new() }),
                node("p", process("/ws/front")),
                node("q", process("")),
                node("solto", process("/outro")),
                node("rel", process("relativo")),
            ],
            edges: vec![
                Edge {
                    id: "1".into(),
                    from: "p".into(),
                    to: "b".into(),
                    kind: EdgeKind::Url,
                },
                Edge {
                    id: "2".into(),
                    from: "q".into(),
                    to: "b".into(),
                    kind: EdgeKind::Url,
                },
                Edge {
                    id: "3".into(),
                    from: "rel".into(),
                    to: "b".into(),
                    kind: EdgeKind::Url,
                },
            ],
            variables: Default::default(),
        };
        assert_eq!(
            file_roots(&graph, "b"),
            vec![PathBuf::from("/ws"), PathBuf::from("/ws/front")]
        );
        assert_eq!(file_roots(&graph, "outro"), vec![PathBuf::from("/ws")]);
    }

    fn key(g: &str, n: &str) -> ViewKey {
        ViewKey {
            graph: g.into(),
            node: n.into(),
        }
    }

    #[test]
    fn store_identity_is_stable_and_per_node() {
        assert_eq!(
            store_identity(&key("g", "b")),
            store_identity(&key("g", "b"))
        );
        assert_ne!(
            store_identity(&key("g", "b")),
            store_identity(&key("g", "c"))
        );
        assert_ne!(
            store_identity(&key("g", "b")),
            store_identity(&key("h", "b"))
        );
        // O separador impede "g"+"bx" = "gb"+"x".
        assert_ne!(
            store_identity(&key("g", "bx")),
            store_identity(&key("gb", "x"))
        );
    }

    fn rect(x: f64, y: f64) -> ViewRect {
        ViewRect {
            x,
            y,
            w: 100.0,
            h: 100.0,
        }
    }

    #[test]
    fn live_views_are_capped_closest_first_and_owner_always() {
        let center = (50.0, 50.0);
        let wanted: Vec<(ViewRect, bool)> = (0..6)
            .map(|i| (rect(i as f64 * 200.0, 0.0), false))
            .collect();
        assert_eq!(pick_live(&wanted, center, MAX_LIVE), vec![0, 1, 2, 3]);
        let mut with_owner = wanted.clone();
        with_owner[5].1 = true;
        assert_eq!(pick_live(&with_owner, center, MAX_LIVE), vec![0, 1, 2, 5]);
        assert_eq!(pick_live(&wanted[..2], center, MAX_LIVE), vec![0, 1]);
        assert!(pick_live(&[], center, MAX_LIVE).is_empty());
    }

    #[test]
    fn rect_must_be_finite_and_sane() {
        assert!(rect(0.0, 0.0).valid());
        assert!(!ViewRect {
            x: f64::NAN,
            y: 0.0,
            w: 10.0,
            h: 10.0
        }
        .valid());
        assert!(!ViewRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 10.0
        }
        .valid());
        assert!(!ViewRect {
            x: 0.0,
            y: 0.0,
            w: 1e9,
            h: 10.0
        }
        .valid());
    }

    #[test]
    fn page_title_is_one_short_line() {
        assert_eq!(
            clean_page_title("  Entrar\n— App\u{2028}x "),
            "Entrar — App x"
        );
        assert_eq!(clean_page_title(&"a".repeat(500)).chars().count(), 200);
    }

    /// O `wry` direto tem de ser o mesmo do Tauri: dois no Cargo.lock é
    /// deriva (o Tauri subiu e o nosso ficou), e a view nativa passaria a
    /// falar com outro WebKit wrapper.
    #[test]
    fn cargo_lock_has_a_single_wry() {
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
            .expect("Cargo.lock");
        let count = lock.lines().filter(|l| *l == "name = \"wry\"").count();
        assert_eq!(count, 1, "mais de um wry no Cargo.lock");
    }
}
