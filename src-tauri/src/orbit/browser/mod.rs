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
//! entregue por `run_on_main_thread` ao módulo do sistema (`macos.rs`,
//! `windows.rs` ou `linux.rs`, todos com o mesmo `apply(app, op)`).

pub mod policy;
pub mod snapshot;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod native;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod os;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod os;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod os;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use super::{EdgeKind, Graph, NodeKind};

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

    /// A parte comum dos dois retângulos (sem área: `None`).
    fn intersect(&self, o: &ViewRect) -> Option<ViewRect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let w = (self.x + self.w).min(o.x + o.w) - x;
        let h = (self.y + self.h).min(o.y + o.h) - y;
        (w > 0.0 && h > 0.0).then_some(ViewRect { x, y, w, h })
    }

    /// O mesmo retângulo, a meio ponto.
    fn same(&self, o: &ViewRect) -> bool {
        (self.x - o.x).abs() < 0.5
            && (self.y - o.y).abs() < 0.5
            && (self.w - o.w).abs() < 0.5
            && (self.h - o.h).abs() < 0.5
    }
}

/// O sistema recorta a view na parte à vista do card? No macOS, sim: a
/// página fica dentro de um container que corta (ver `macos.rs`). No Windows
/// e no Linux ainda não (ver `fase5-navegador`, "Card cortado"): lá só o
/// marcador inteiro à vista deixa a view viva.
pub const CLIPS: bool = cfg!(target_os = "macos");

/// A intenção de geometria do front, conferida: `rect` é o marcador inteiro
/// e `clip` a parte dele à vista (sem `clip`, o marcador inteiro). Devolve
/// `(rect, clip)` com o `clip` sempre dentro do `rect`, ou `None` para
/// esconder: sem marcador, sem parte à vista, ou cortado num sistema que não
/// recorta (`clips = false`).
pub fn visible_part(
    rect: Option<ViewRect>,
    clip: Option<ViewRect>,
    clips: bool,
) -> Option<(ViewRect, ViewRect)> {
    let rect = rect.filter(ViewRect::valid)?;
    let clip = match clip {
        None => rect,
        Some(c) if [c.x, c.y, c.w, c.h].iter().all(|v| v.is_finite()) => {
            rect.intersect(&c).filter(ViewRect::valid)?
        }
        Some(_) => return None,
    };
    if !clips && !clip.same(&rect) {
        return None;
    }
    Some((rect, clip))
}

/// Os dois frames do recorte no macOS, em pontos do `contentView` (altura
/// `parent_h`; sem `flipped`, a origem é embaixo e o y sobe): o container
/// ocupa só `clip`, e a página, dentro dele, fica no deslocamento que a
/// mantém exatamente onde o marcador está — com o tamanho inteiro, então ela
/// não encolhe nem desliza, só é cortada.
pub fn clip_frames(
    rect: ViewRect,
    clip: ViewRect,
    parent_h: f64,
    flipped: bool,
) -> (ViewRect, ViewRect) {
    let (cy, iy) = if flipped {
        (clip.y, rect.y - clip.y)
    } else {
        (
            parent_h - clip.y - clip.h,
            clip.h - (rect.y - clip.y) - rect.h,
        )
    };
    (
        ViewRect {
            x: clip.x,
            y: cy,
            w: clip.w,
            h: clip.h,
        },
        ViewRect {
            x: rect.x - clip.x,
            y: iy,
            w: rect.w,
            h: rect.h,
        },
    )
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
    /// A última navegação recusada: o código do motivo (`notLocal`,
    /// `tauriHost`, `rawFile`, `notHtml`, `notFound`, `outsideRoots`,
    /// `notFileUrl`), que o card traduz.
    pub refused: Option<String>,
    /// macOS 11–13: armazenamento não persistente (o login não fica salvo).
    pub ephemeral: bool,
    /// O último passo de um agente nesta página (o rodapé do card).
    pub agent: Option<AgentStep>,
    /// Erros no console da página atual, do último passo do agente.
    pub console_errors: Option<u32>,
}

/// O que um agente fez por último na página: o card mostra "<agente>
/// usando" e o passo ("clicou em “Entrar”"). `action` é o código
/// (`snapshot`, `click`, `type`, `console`, `screenshot`), que o front traduz.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStep {
    pub by: String,
    pub action: String,
    /// O nome do elemento (clicar, digitar), uma linha, redigido.
    pub target: Option<String>,
    pub at: u64,
}

/// Uma ferramenta de agente no navegador (`orbit_browser_*`).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentAction {
    Snapshot,
    /// Clicar no elemento da ref (do último snapshot).
    Click {
        reference: String,
    },
    /// Trocar o texto do campo da ref; com `submit`, enviar o formulário.
    Type {
        reference: String,
        text: String,
        submit: bool,
    },
    /// As últimas entradas de erro/aviso do console da página atual.
    Console,
    /// Print da página em PNG, gravado como `screenshot-<n>.png` na pasta
    /// `dir` (a `out/` do nó no run, 0700; o arquivo, 0600).
    Screenshot {
        dir: PathBuf,
    },
}

impl AgentAction {
    /// O nome da ação no `isolated.js`.
    fn script(&self) -> &'static str {
        match self {
            AgentAction::Snapshot => "snapshot",
            AgentAction::Click { .. } => "click",
            AgentAction::Type { .. } => "type",
            AgentAction::Console => "console",
            AgentAction::Screenshot { .. } => "screenshot",
        }
    }

    /// Os argumentos da ação (JSON, vão como argumento do script).
    fn args(&self) -> Value {
        match self {
            AgentAction::Snapshot | AgentAction::Console | AgentAction::Screenshot { .. } => {
                serde_json::json!({})
            }
            AgentAction::Click { reference } => serde_json::json!({ "ref": reference }),
            AgentAction::Type {
                reference,
                text,
                submit,
            } => serde_json::json!({ "ref": reference, "text": text, "submit": submit }),
        }
    }

    /// Ler (snapshot, console, print) vale com o dono usando; agir, não.
    pub fn acts(&self) -> bool {
        matches!(self, AgentAction::Click { .. } | AgentAction::Type { .. })
    }
}

/// Print da página para o agente.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Largura máxima do print do agente (px).
pub const MAX_SHOT_PX: f64 = 1600.0;
/// Teto do arquivo do print do agente.
pub const MAX_SHOT_BYTES: usize = 8 * 1024 * 1024;
/// Prefixo dos prints na pasta do run.
const SHOT_PREFIX: &str = "screenshot-";

/// O próximo `screenshot-<n>.png` da pasta (o maior que existe + 1).
pub fn next_shot_path(dir: &std::path::Path) -> PathBuf {
    let last = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_prefix(SHOT_PREFIX)?
                .strip_suffix(".png")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    dir.join(format!("{SHOT_PREFIX}{}.png", last + 1))
}

/// Grava o print novo, 0600, sem sobrescrever nada.
pub fn write_shot(path: &std::path::Path, png: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(png)
}

/// Quanto tempo a ação do agente segura o foco "agente" no card.
pub const AGENT_HOLD: Duration = Duration::from_millis(1500);
/// Assentar: sem navegação nesse tempo, a ação terminou.
pub const SETTLE_QUIET: Duration = Duration::from_millis(1500);
/// Assentar: no máximo isto.
pub const SETTLE_MAX: Duration = Duration::from_secs(5);

/// A página assentou depois de clicar ou digitar? Com navegação começada,
/// quando ela termina de carregar; sem, depois de [`SETTLE_QUIET`]; nunca
/// além de [`SETTLE_MAX`].
pub fn settled(navigated: bool, loading: bool, elapsed: Duration) -> bool {
    if elapsed >= SETTLE_MAX {
        return true;
    }
    if navigated {
        !loading
    } else {
        elapsed >= SETTLE_QUIET
    }
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
    /// O marcador inteiro (o tamanho da página).
    pub rect: Option<ViewRect>,
    /// A parte do marcador à vista (só no sistema que recorta, [`CLIPS`]);
    /// `None` = o marcador inteiro.
    pub clip: Option<ViewRect>,
    pub zoom: f64,
    pub visible: bool,
    /// A view some com o dono usando: o teclado volta ao app (o core já
    /// tirou o "você").
    pub give_back: bool,
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
    /// Tira o print da página (a prévia do card).
    Snapshot(ViewKey),
    /// Roda a ação `action` do `isolated.js` no mundo isolado, com `args`
    /// (JSON) como argumento; o JSON da resposta volta por `reply`.
    Eval {
        key: ViewKey,
        action: &'static str,
        args: String,
        reply: std::sync::mpsc::Sender<Result<String, String>>,
    },
    /// Print em PNG para o agente (até [`MAX_SHOT_PX`] de largura).
    Capture {
        key: ViewKey,
        reply: std::sync::mpsc::Sender<Result<Shot, String>>,
    },
    /// Destrói toda view que não está na lista.
    Retain(Vec<ViewKey>),
    /// Destrói a view do nó (o armazenamento vai ser apagado); com
    /// `give_back`, o dono estava nela e o teclado volta ao app antes.
    Destroy {
        key: ViewKey,
        give_back: bool,
    },
    /// Apaga o armazenamento do nó (a view já foi destruída por um
    /// [`Op::Destroy`] anterior, noutra volta da main thread) e avisa
    /// [`BrowserViews::on_cleared`]; `reload` carrega a página de novo.
    RemoveStore {
        key: ViewKey,
        reload: bool,
        attempt: u32,
    },
}

#[derive(Debug, Default)]
struct Entry {
    state: BrowserState,
    rect: Option<ViewRect>,
    /// A parte de `rect` à vista (macOS recorta a view nela).
    clip: Option<ViewRect>,
    zoom: f64,
    /// Raízes canônicas do `orbit-file` (para mostrar a url).
    roots: Vec<PathBuf>,
    /// O último print (JPEG em base64) para a prévia do card.
    preview: Option<String>,
    /// A página mudou desde o último print (título, url, clique ou tecla).
    dirty: bool,
    last_shot: u64,
    /// A view nativa já existe (o agente só lê/age numa view criada).
    created: bool,
    /// Navegações começadas (o assentar do clique compara).
    nav_seq: u64,
    /// Até quando o foco "agente" vale (ms).
    agent_until: u64,
}

impl Entry {
    /// Navegador novo: zoom 1 (o `Default` dá 0).
    fn fresh() -> Self {
        Entry {
            zoom: 1.0,
            ..Entry::default()
        }
    }
}

/// Aplica `f` à entrada de `key`, **se existir**; devolve o estado novo
/// quando ele mudou (o que se emite).
fn change(
    entries: &mut HashMap<ViewKey, Entry>,
    key: &ViewKey,
    f: impl FnOnce(&mut Entry),
) -> Option<BrowserState> {
    let entry = entries.get_mut(key)?;
    let before = entry.state.clone();
    f(entry);
    (entry.state != before).then(|| entry.state.clone())
}

/// A vez de cada navegador: uma ação de agente em voo por nó, e até
/// [`MAX_QUEUED`] esperando.
#[derive(Default)]
struct Lanes {
    lanes: Mutex<HashMap<ViewKey, (bool, usize)>>,
    freed: Condvar,
}

/// Quantas ações de agente esperam a vez num navegador; passando, "ocupado".
pub const MAX_QUEUED: usize = 4;
/// Teto de cada ação no script (e da espera pela vez).
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(5);
/// Quanto o agente espera a view nascer e a página carregar.
const READY_TIMEOUT: Duration = Duration::from_secs(5);
/// Navegador ocupado com ações de agente.
pub const BUSY: &str =
    "o navegador está ocupado com outras ações; tente de novo em alguns segundos";
/// Sem view nativa (o Orbit fechado, ou outro grafo à vista).
pub const NOT_OPEN: &str = "o navegador não está aberto: o dono precisa abrir este grafo no Orbit";

struct Turn<'a> {
    lanes: &'a Lanes,
    key: ViewKey,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let mut lanes = self.lanes.lanes.lock();
        if let Some(lane) = lanes.get_mut(&self.key) {
            lane.0 = false;
            if lane.1 == 0 {
                lanes.remove(&self.key);
            }
        }
        self.lanes.freed.notify_all();
    }
}

impl Lanes {
    /// Espera a vez em `key` (no máximo `wait`); fila cheia é "ocupado".
    fn turn(&self, key: &ViewKey, wait: Duration) -> Result<Turn<'_>, &'static str> {
        let deadline = std::time::Instant::now() + wait;
        let mut lanes = self.lanes.lock();
        let lane = lanes.entry(key.clone()).or_default();
        if lane.0 {
            if lane.1 >= MAX_QUEUED {
                return Err(BUSY);
            }
            lane.1 += 1;
            loop {
                let timed_out = self.freed.wait_until(&mut lanes, deadline).timed_out();
                let lane = lanes.entry(key.clone()).or_default();
                if !lane.0 {
                    lane.1 -= 1;
                    break;
                }
                if timed_out {
                    lane.1 -= 1;
                    return Err(BUSY);
                }
            }
        }
        lanes.entry(key.clone()).or_default().0 = true;
        Ok(Turn {
            lanes: self,
            key: key.clone(),
        })
    }
}

/// O registro dos navegadores nativos (estado no core).
#[derive(Default)]
pub struct BrowserViews {
    entries: Mutex<HashMap<ViewKey, Entry>>,
    /// O relógio dos prints periódicos já roda?
    ticking: std::sync::atomic::AtomicBool,
    /// Centro da área do canvas (o mais perto dele fica vivo primeiro).
    center: Mutex<(f64, f64)>,
    lanes: Lanes,
}

/// O navegador nativo existe neste sistema? (Senão o front usa o iframe.)
pub const NATIVE: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

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

/// A pasta do armazenamento de um nó no Windows e no Linux (cookies,
/// localStorage, cache): `<home>/<perfil>/orbit/browser/<identidade em hex>`.
/// O macOS 14+ guarda pelo identificador, sem pasta nossa.
pub fn store_dir(home: &std::path::Path, key: &ViewKey) -> PathBuf {
    let id: String = store_identity(key)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    home.join(crate::profile::HOME_DIR)
        .join("orbit")
        .join("browser")
        .join(id)
}

/// Cria a pasta do armazenamento só para o dono (0700 fora do Windows).
pub fn ensure_store_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
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

/// A intenção de geometria de `key` aplicada ao registro: recalcula quem
/// fica vivo no grafo (teto [`MAX_LIVE`], o mais perto de `center` primeiro)
/// e devolve cada view que muda de lugar ou de estado. A view que some com o
/// dono usando perde o "você" (o teclado volta ao app). `rect = None` de nó
/// que nem existe não cria nada (card saindo depois do `retain`).
fn place_all(
    entries: &mut HashMap<ViewKey, Entry>,
    key: &ViewKey,
    part: Option<(ViewRect, ViewRect)>,
    zoom: f64,
    center: (f64, f64),
) -> Vec<(ViewKey, Place, BrowserState)> {
    if part.is_none() && !entries.contains_key(key) {
        return Vec::new();
    }
    let rect = part.map(|(r, _)| r);
    let clip = part.map(|(_, c)| c);
    let entry = entries.entry(key.clone()).or_insert_with(Entry::fresh);
    let moved = entry.rect != rect || entry.clip != clip || entry.zoom != zoom;
    entry.rect = rect;
    entry.clip = clip;
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
    let mut out = Vec::new();
    for (k, e) in entries.iter_mut().filter(|(k, _)| k.graph == key.graph) {
        let now_live = live.contains(k);
        if now_live != e.state.live || (k == key && moved) {
            e.state.live = now_live;
            let give_back = !now_live && e.state.focus == Focus::Owner;
            if give_back {
                e.state.focus = Focus::None;
            }
            out.push((
                k.clone(),
                Place {
                    rect: e.rect,
                    clip: e.clip,
                    zoom: e.zoom,
                    visible: now_live,
                    give_back,
                },
                e.state.clone(),
            ));
        }
    }
    out
}

/// O que pede um print novo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotTrigger {
    /// A página terminou de carregar.
    Loaded,
    /// O dono parou de usar a página.
    OwnerLeft,
    /// Um agente agiu (clicou, digitou).
    AgentAction,
    /// O relógio: a cada [`SHOT_EVERY_MS`], só view viva e página mudada.
    Tick,
}

/// De quanto em quanto tempo, no máximo, a view viva e mudada ganha print.
pub const SHOT_EVERY_MS: u64 = 5_000;
/// Teto da prévia em base64 (acima, fica a anterior).
pub const MAX_PREVIEW_B64: usize = 400 * 1024;

/// Tirar print agora? Esconder nunca espera print: usa o último que existe.
pub fn should_shoot(
    trigger: ShotTrigger,
    live: bool,
    dirty: bool,
    last_shot: u64,
    now: u64,
) -> bool {
    match trigger {
        ShotTrigger::Loaded | ShotTrigger::OwnerLeft | ShotTrigger::AgentAction => true,
        ShotTrigger::Tick => live && dirty && now.saturating_sub(last_shot) >= SHOT_EVERY_MS,
    }
}

/// A recusa a `click`/`type` de agente com o dono usando a página: na hora,
/// sem esperar (o agente tenta de novo ou usa `orbit_wait`).
pub const OWNER_BUSY: &str = "o dono está usando este navegador; tente de novo em alguns segundos";

/// O agente pode agir (clicar, digitar) na página agora? Ler (snapshot,
/// console, print) vale sempre; agir, só sem o dono usando.
pub fn agent_may_act(focus: Focus) -> Result<(), &'static str> {
    match focus {
        Focus::Owner => Err(OWNER_BUSY),
        Focus::None | Focus::Agent => Ok(()),
    }
}

/// Para onde vai uma tecla com o teclado numa página de nó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoute {
    /// Segue para a página.
    Page,
    /// Esc: a tecla some e o foco volta ao app.
    GiveBack,
    /// Atalho do app (⌘W, ⌘K, ⌘L): o foco volta ao app e a tecla vai junto.
    ToApp,
}

/// Código de tecla do Esc no macOS (`kVK_Escape`).
pub const KEY_ESCAPE: u16 = 53;

/// Modificadores de uma tecla.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
}

/// Esc sozinho devolve o foco; ⇧Esc entrega um Esc à página (fechar o modal
/// dela); ⌘W, ⌘K e ⌘L são do app; o resto (⌘C, ⌘V, ⌘A…) é da página.
pub fn route_key(key_code: u16, mods: Mods, chars: &str) -> KeyRoute {
    route(key_code == KEY_ESCAPE, mods, chars)
}

/// A mesma regra de [`route_key`] sem o código de tecla do macOS: no Windows
/// e no Linux o Ctrl faz o papel do ⌘ (vem em `mods.command`).
pub fn route(escape: bool, mods: Mods, chars: &str) -> KeyRoute {
    let only_command = mods.command && !mods.shift && !mods.control && !mods.option;
    if escape {
        let bare = !mods.shift && !mods.control && !mods.option && !mods.command;
        return if bare {
            KeyRoute::GiveBack
        } else {
            KeyRoute::Page
        };
    }
    if only_command && matches!(chars.to_ascii_lowercase().as_str(), "w" | "k" | "l") {
        return KeyRoute::ToApp;
    }
    KeyRoute::Page
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

/// Texto da página para o card e o agente: uma linha, até `max`, redigido.
fn redact_line(s: &str, max: usize) -> String {
    crate::session::redact::redact(&snapshot::one_line(s, max)).into_owned()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// O atalho do app (Ctrl+W/K/L) que a página do nó recebeu no Windows e no
/// Linux, reemitido na webview do app depois de o foco voltar a ela. `None`
/// para qualquer outra tecla: só essas três letras entram no script.
pub fn forward_shortcut_js(chars: &str) -> Option<String> {
    let key = chars.to_ascii_lowercase();
    if !matches!(key.as_str(), "w" | "k" | "l") {
        return None;
    }
    let code = format!("Key{}", key.to_ascii_uppercase());
    Some(format!(
        "window.dispatchEvent(new KeyboardEvent(\"keydown\", {{ key: \"{key}\", code: \"{code}\", ctrlKey: true, bubbles: true, cancelable: true }}));"
    ))
}

/// Entrega a operação à main thread do sistema; sem view nativa (outro
/// sistema), nada acontece.
fn dispatch(app: &AppHandle, op: Op) {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let handle = app.clone();
        if let Err(e) = app.run_on_main_thread(move || os::apply(&handle, op)) {
            eprintln!("[tyba] navegador do orbit: {e}");
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
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

    /// Muda o estado de um navegador que existe e emite, se mudou. Entrada
    /// que o `retain` já tirou não volta: o aviso atrasado da view (título,
    /// load, foco do agente) vira nada.
    fn update(&self, app: &AppHandle, key: &ViewKey, f: impl FnOnce(&mut Entry)) {
        if let Some(state) = change(&mut self.entries.lock(), key, f) {
            Self::emit(app, key, &state);
        }
    }

    /// Como [`Self::update`], mas cria a entrada: só a intenção do front
    /// (navegar) faz nascer um navegador.
    fn upsert(&self, app: &AppHandle, key: &ViewKey, f: impl FnOnce(&mut Entry)) {
        let state = {
            let mut entries = self.entries.lock();
            entries.entry(key.clone()).or_insert_with(Entry::fresh);
            change(&mut entries, key, f)
        };
        if let Some(state) = state {
            Self::emit(app, key, &state);
        }
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
            policy::resolve_file(url, &roots).map_err(|r| r.code().to_string())
        } else {
            match policy::decide(url) {
                policy::Decision::Allow => Ok(url.trim().to_string()),
                policy::Decision::Deny(why) => Err(why.to_string()),
            }
        };
        let target = match target {
            Ok(t) => t,
            Err(why) => {
                self.upsert(app, key, |e| e.state.refused = Some(why.clone()));
                return Err(why);
            }
        };
        let mut place = None;
        self.upsert(app, key, |e| {
            e.roots = roots.clone();
            e.state.refused = None;
            e.state.url = policy::display_url(&target, &roots);
            e.state.loading = true;
            place = Some(Place {
                rect: e.rect,
                clip: e.clip,
                zoom: e.zoom,
                visible: e.state.live,
                give_back: false,
            });
        });
        let place = place.unwrap_or(Place {
            rect: None,
            clip: None,
            zoom: 1.0,
            visible: false,
            give_back: false,
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

    /// A intenção de geometria do front: `rect` = o marcador, com nada por
    /// cima da parte à vista (`clip`; sem ele, o marcador inteiro); `None` =
    /// esconder (o card mostra o print). Cortado num sistema que não recorta
    /// também esconde ([`visible_part`]). Recalcula quem fica vivo (teto
    /// [`MAX_LIVE`]) e reposiciona.
    pub fn set_view(
        &self,
        app: &AppHandle,
        key: &ViewKey,
        rect: Option<ViewRect>,
        clip: Option<ViewRect>,
        zoom: f64,
        center: Option<(f64, f64)>,
    ) {
        let part = visible_part(rect, clip, CLIPS);
        let zoom = if zoom.is_finite() {
            zoom.clamp(0.1, 5.0)
        } else {
            1.0
        };
        if let Some(c) = center.filter(|c| c.0.is_finite() && c.1.is_finite()) {
            *self.center.lock() = c;
        }
        let center = *self.center.lock();
        let placed = place_all(&mut self.entries.lock(), key, part, zoom, center);
        for (k, _, s) in &placed {
            Self::emit(app, k, s);
        }
        for (k, place, _) in placed {
            // O dono saiu porque a view sumiu: print antes de esconder.
            if place.give_back {
                self.shoot(app, &k, ShotTrigger::OwnerLeft);
            }
            dispatch(app, Op::Place { key: k, place });
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

    /// "Apagar dados do navegador" (o ⋯ do nó): destrói a view, apaga
    /// cookies, login e armazenamento do nó e carrega a mesma página de novo,
    /// já sem eles. Não espera: o apagar é da main thread (macOS) ou de uma
    /// thread própria (Windows e Linux).
    pub fn clear_data(&self, app: &AppHandle, key: &ViewKey) {
        let (state, give_back) = {
            let mut entries = self.entries.lock();
            match entries.get_mut(key) {
                Some(e) => {
                    let give_back = e.state.focus == Focus::Owner;
                    e.created = false;
                    e.preview = None;
                    e.state.preview_at = None;
                    e.state.loading = false;
                    e.state.focus = Focus::None;
                    (Some(e.state.clone()), give_back)
                }
                None => (None, false),
            }
        };
        if let Some(s) = &state {
            Self::emit(app, key, s);
        }
        self.wipe(app, key, give_back, state.is_some());
    }

    /// O nó saiu do grafo: o armazenamento dele vai junto (cookies e login
    /// não sobram para um nó que não existe).
    pub fn forget(&self, app: &AppHandle, key: &ViewKey) {
        let give_back = self
            .entries
            .lock()
            .remove(key)
            .is_some_and(|e| e.state.focus == Focus::Owner);
        self.wipe(app, key, give_back, false);
    }

    /// Destrói a view e, numa volta seguinte da main thread (a view já
    /// solta), apaga o armazenamento.
    fn wipe(&self, app: &AppHandle, key: &ViewKey, give_back: bool, reload: bool) {
        dispatch(
            app,
            Op::Destroy {
                key: key.clone(),
                give_back,
            },
        );
        dispatch(
            app,
            Op::RemoveStore {
                key: key.clone(),
                reload,
                attempt: 0,
            },
        );
    }

    /// O armazenamento do nó foi apagado: com `reload`, a página volta (o
    /// `navigate` confere o disco, então fora da main thread).
    pub fn on_cleared(&self, app: &AppHandle, key: &ViewKey, reload: bool) {
        if !reload {
            return;
        }
        let Some((url, roots)) = self
            .entries
            .lock()
            .get(key)
            .map(|e| (e.state.url.clone(), e.roots.clone()))
        else {
            return;
        };
        if url.is_empty() {
            return;
        }
        let app = app.clone();
        let key = key.clone();
        std::thread::spawn(move || {
            let _ = app
                .state::<BrowserViews>()
                .navigate(&app, &key, &url, &roots);
        });
    }

    // ── chamados pela view nativa (main thread) ──────────────────────────

    /// A view nasceu: o armazenamento é não persistente neste macOS?
    pub fn on_created(&self, app: &AppHandle, key: &ViewKey, ephemeral: bool) {
        self.update(app, key, |e| {
            e.created = true;
            e.state.ephemeral = ephemeral;
        });
    }

    /// Começou (`finished = false`) ou terminou de carregar `url`.
    pub fn on_load(&self, app: &AppHandle, key: &ViewKey, url: &str, finished: bool) {
        self.update(app, key, |e| {
            e.state.url = policy::display_url(url, &e.roots);
            e.state.loading = !finished;
            e.dirty = true;
            if !finished {
                e.state.refused = None;
                e.nav_seq += 1;
            }
        });
        if finished {
            // O WebKit pinta logo depois do `load`: um respiro antes do print.
            let app = app.clone();
            let key = key.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(400));
                app.state::<BrowserViews>()
                    .shoot(&app, &key, ShotTrigger::Loaded);
            });
        }
    }

    pub fn on_title(&self, app: &AppHandle, key: &ViewKey, title: &str) {
        let title = clean_page_title(title);
        self.update(app, key, |e| {
            e.dirty = true;
            e.state.title = title;
        });
    }

    /// O dono clicou ou digitou na página: o próximo tique tira print.
    pub fn mark_dirty(&self, key: &ViewKey) {
        if let Some(e) = self.entries.lock().get_mut(key) {
            e.dirty = true;
        }
    }

    /// Pede o print, se a regra ([`should_shoot`]) deixa.
    pub fn shoot(&self, app: &AppHandle, key: &ViewKey, trigger: ShotTrigger) {
        let go =
            self.entries.lock().get(key).is_some_and(|e| {
                should_shoot(trigger, e.state.live, e.dirty, e.last_shot, now_ms())
            });
        if go {
            dispatch(app, Op::Snapshot(key.clone()));
        }
    }

    /// O print chegou (JPEG em base64): vira a prévia do card.
    pub fn on_snapshot(&self, app: &AppHandle, key: &ViewKey, b64: String) {
        if b64.len() > MAX_PREVIEW_B64 {
            return;
        }
        let at = now_ms();
        self.update(app, key, |e| {
            e.preview = Some(b64);
            e.dirty = false;
            e.last_shot = at;
            e.state.preview_at = Some(at);
        });
    }

    /// A prévia guardada (o front a busca quando `previewAt` muda).
    pub fn preview(&self, key: &ViewKey) -> Option<String> {
        self.entries.lock().get(key).and_then(|e| e.preview.clone())
    }

    /// O relógio dos prints periódicos (um por app, a cada segundo confere
    /// [`ShotTrigger::Tick`] em cada navegador).
    pub fn start_ticking(&self, app: &AppHandle) {
        use std::sync::atomic::Ordering;
        if self.ticking.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let views = app.state::<BrowserViews>();
            let keys: Vec<ViewKey> = views.entries.lock().keys().cloned().collect();
            for key in keys {
                views.shoot(&app, &key, ShotTrigger::Tick);
            }
        });
    }

    /// A política recusou uma navegação (de qualquer frame).
    pub fn on_refused(&self, app: &AppHandle, key: &ViewKey, why: &str) {
        self.update(app, key, |e| e.state.refused = Some(why.to_string()));
    }

    /// O dono clicou na página de `key` (ou, com `None`, fora de todas):
    /// ela vira "você" e qualquer outra que estava com ele volta a ninguém.
    pub fn set_owner(&self, app: &AppHandle, key: Option<&ViewKey>) {
        let changed: Vec<(ViewKey, BrowserState)> = {
            let mut entries = self.entries.lock();
            let mut out = Vec::new();
            for (k, e) in entries.iter_mut() {
                let want = if Some(k) == key {
                    Focus::Owner
                } else if e.state.focus == Focus::Owner {
                    Focus::None
                } else {
                    continue;
                };
                if e.state.focus != want {
                    e.state.focus = want;
                    out.push((k.clone(), e.state.clone()));
                }
            }
            out
        };
        for (k, s) in &changed {
            Self::emit(app, k, s);
            if s.focus == Focus::None {
                self.shoot(app, k, ShotTrigger::OwnerLeft);
            }
        }
    }

    /// Quem está com a página agora.
    pub fn focus(&self, key: &ViewKey) -> Focus {
        self.entries
            .lock()
            .get(key)
            .map_or(Focus::None, |e| e.state.focus)
    }

    // ── ferramentas do agente (`orbit_browser_*`) ────────────────────────

    /// Uma ferramenta de agente na página de `key`, já com a permissão
    /// conferida pelo run (ligação `controla` atual). Bloqueia: espera a
    /// view existir e a página carregar, a vez do nó (uma ação em voo, fila
    /// [`MAX_QUEUED`]) e a resposta do script ([`ACTION_TIMEOUT`]). Agir com
    /// o dono usando a página é recusado na hora ([`OWNER_BUSY`]).
    pub fn agent(
        &self,
        app: &AppHandle,
        key: &ViewKey,
        by: &str,
        action: AgentAction,
    ) -> Result<Value, String> {
        if !NATIVE {
            return Err("ler e usar o navegador pelo agente não existe neste sistema".to_string());
        }
        self.wait_ready(key)?;
        let _turn = self
            .lanes
            .turn(key, ACTION_TIMEOUT)
            .map_err(str::to_string)?;
        if action.acts() {
            agent_may_act(self.focus(key)).map_err(str::to_string)?;
        }
        eprintln!(
            "[orbit] navegador {}: {} pelo agente",
            key.node,
            action.script()
        );
        if let AgentAction::Screenshot { dir } = &action {
            let shot = self.capture(app, key)?;
            // O nome sai com a vez do nó: dois prints não disputam o número.
            let path = next_shot_path(dir);
            self.note_step(app, key, by, &action, None, None);
            if shot.png.len() > MAX_SHOT_BYTES {
                return Err("o print passou de 8 MB".to_string());
            }
            write_shot(&path, &shot.png).map_err(|e| format!("gravar o print: {e}"))?;
            return Ok(serde_json::json!({
                "path": path.to_string_lossy(),
                "width": shot.width,
                "height": shot.height,
            }));
        }
        let nav_before = self.nav_seq(key);
        if action.acts() {
            self.hold_agent_focus(app, key);
        }
        let reply = self.eval(app, key, action.script(), action.args())?;
        let ok = reply.ok.clone().unwrap_or(Value::Null);
        let name = ok
            .get("name")
            .and_then(Value::as_str)
            .map(|n| redact_line(n, 80));
        self.note_step(app, key, by, &action, name.clone(), Some(reply.errors));
        if let Some(code) = &reply.error {
            return Err(snapshot::error_message(code, reply.detail.as_deref()));
        }
        let roots = self.roots(key);
        let url = policy::display_url(&reply.url, &roots);
        let done = |verb: &str| {
            let role = ok.get("role").and_then(Value::as_str).unwrap_or("");
            format!(
                "{verb} {} \"{}\"",
                snapshot::one_line(role, 40),
                name.clone().unwrap_or_default()
            )
        };
        match action {
            AgentAction::Snapshot => Ok(Value::String(snapshot::format_snapshot(
                &url,
                &reply.title,
                &ok,
            ))),
            AgentAction::Console => Ok(Value::String(snapshot::format_console(
                &url,
                &reply.title,
                &ok,
            ))),
            // O print saiu antes, sem script.
            AgentAction::Screenshot { .. } => Err("o print não passa pelo script".to_string()),
            AgentAction::Click { .. } | AgentAction::Type { .. } => {
                let what = match &action {
                    AgentAction::Type { .. } => {
                        let chars = ok.get("chars").and_then(Value::as_u64).unwrap_or(0);
                        format!(
                            "digitou {chars} caracteres em \"{}\"",
                            name.unwrap_or_default()
                        )
                    }
                    _ => done("clicou em"),
                };
                // Espera a página assentar e diz onde ela ficou.
                self.settle(key, nav_before);
                self.shoot(app, key, ShotTrigger::AgentAction);
                // A url da página (rota de SPA não dispara `load`); se a
                // página ainda navega, a do último `load`.
                let (url, title, errors) = match self.eval(app, key, "where", Value::Null) {
                    Ok(r) if r.error.is_none() => {
                        (policy::display_url(&r.url, &roots), r.title, r.errors)
                    }
                    _ => {
                        let (url, title) = self.page(key);
                        (url, title, self.console_errors(key))
                    }
                };
                self.update(app, key, |e| e.state.console_errors = Some(errors));
                Ok(serde_json::json!({
                    "done": what,
                    "url": redact_line(&url, 300),
                    "title": redact_line(&title, 200),
                    "console_errors": errors,
                }))
            }
        }
    }

    /// Quantas navegações a página já começou (o assentar compara).
    fn nav_seq(&self, key: &ViewKey) -> u64 {
        self.entries.lock().get(key).map_or(0, |e| e.nav_seq)
    }

    /// Url (como o dono a vê) e título atuais.
    fn page(&self, key: &ViewKey) -> (String, String) {
        self.entries
            .lock()
            .get(key)
            .map(|e| (e.state.url.clone(), e.state.title.clone()))
            .unwrap_or_default()
    }

    fn console_errors(&self, key: &ViewKey) -> u32 {
        self.entries
            .lock()
            .get(key)
            .and_then(|e| e.state.console_errors)
            .unwrap_or(0)
    }

    /// Espera a página assentar ([`settled`]).
    fn settle(&self, key: &ViewKey, nav_before: u64) {
        let start = std::time::Instant::now();
        loop {
            let (seq, loading) = self
                .entries
                .lock()
                .get(key)
                .map_or((nav_before, false), |e| (e.nav_seq, e.state.loading));
            if settled(seq != nav_before, loading, start.elapsed()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// O card mostra "<agente> usando" por [`AGENT_HOLD`]: a ação é por
    /// script, a view não rouba o teclado do app.
    fn hold_agent_focus(&self, app: &AppHandle, key: &ViewKey) {
        let mut changed = false;
        self.update(app, key, |e| {
            if e.state.focus == Focus::None {
                e.state.focus = Focus::Agent;
                changed = true;
            }
            e.agent_until = now_ms() + AGENT_HOLD.as_millis() as u64;
        });
        if !changed {
            return;
        }
        let app = app.clone();
        let key = key.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            let views = app.state::<BrowserViews>();
            let mut done = false;
            views.update(&app, &key, |e| {
                if e.state.focus != Focus::Agent {
                    done = true;
                } else if now_ms() >= e.agent_until {
                    e.state.focus = Focus::None;
                    done = true;
                }
            });
            if done || views.entries.lock().get(&key).is_none() {
                return;
            }
        });
    }

    fn roots(&self, key: &ViewKey) -> Vec<PathBuf> {
        self.entries
            .lock()
            .get(key)
            .map(|e| e.roots.clone())
            .unwrap_or_default()
    }

    /// Espera a view nativa existir e a página terminar de carregar (no
    /// máximo [`READY_TIMEOUT`]; página lenta segue mesmo carregando).
    fn wait_ready(&self, key: &ViewKey) -> Result<(), String> {
        let deadline = std::time::Instant::now() + READY_TIMEOUT;
        loop {
            let (created, loading) = self
                .entries
                .lock()
                .get(key)
                .map_or((false, false), |e| (e.created, e.state.loading));
            let late = std::time::Instant::now() >= deadline;
            if created && (!loading || late) {
                return Ok(());
            }
            if late {
                return Err(NOT_OPEN.to_string());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Roda a ação no mundo isolado e lê a resposta.
    fn eval(
        &self,
        app: &AppHandle,
        key: &ViewKey,
        action: &'static str,
        args: Value,
    ) -> Result<snapshot::PageReply, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        dispatch(
            app,
            Op::Eval {
                key: key.clone(),
                action,
                args: args.to_string(),
                reply: tx,
            },
        );
        let raw = match rx.recv_timeout(ACTION_TIMEOUT) {
            Ok(r) => r?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err("a página não respondeu em 5 s".to_string())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(NOT_OPEN.to_string())
            }
        };
        snapshot::parse_reply(&raw)
    }

    /// O print em PNG da view (escondida também).
    fn capture(&self, app: &AppHandle, key: &ViewKey) -> Result<Shot, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        dispatch(
            app,
            Op::Capture {
                key: key.clone(),
                reply: tx,
            },
        );
        match rx.recv_timeout(ACTION_TIMEOUT) {
            Ok(r) => r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err("o print não saiu em 5 s".to_string())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(NOT_OPEN.to_string()),
        }
    }

    /// O passo do agente vira o rodapé do card.
    fn note_step(
        &self,
        app: &AppHandle,
        key: &ViewKey,
        by: &str,
        action: &AgentAction,
        target: Option<String>,
        errors: Option<u32>,
    ) {
        let step = AgentStep {
            by: snapshot::one_line(by, 120),
            action: action.script().to_string(),
            target,
            at: now_ms(),
        };
        self.update(app, key, |e| {
            e.state.agent = Some(step);
            if errors.is_some() {
                e.state.console_errors = errors;
            }
        });
    }
}

/// Os navegadores que saíram do grafo ao salvar (`old` → `new`): o core
/// apaga o armazenamento deles ([`BrowserViews::forget`]).
pub fn removed_browsers(old: &Graph, new: &Graph) -> Vec<String> {
    old.nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Browser { .. }))
        .filter(|n| !new.nodes.iter().any(|m| m.id == n.id))
        .map(|n| n.id.clone())
        .collect()
}

/// Os navegadores de um grafo (apagar o grafo apaga o armazenamento deles).
pub fn browsers(graph: &Graph) -> Vec<String> {
    graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Browser { .. }))
        .map(|n| n.id.clone())
        .collect()
}

/// Apaga a pasta do armazenamento de um nó (Windows e Linux). O processo do
/// navegador pode segurar arquivo por um instante depois de a view fechar:
/// tenta `tries` vezes, `wait` entre elas. Pasta que não existe já está
/// apagada.
pub fn remove_store_dir(dir: &std::path::Path, tries: u32, wait: Duration) -> std::io::Result<()> {
    let mut last = Ok(());
    for i in 0..tries.max(1) {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Err(e),
        }
        if i + 1 < tries {
            std::thread::sleep(wait);
        }
    }
    last
}

/// Windows e Linux: apaga a pasta do nó numa thread própria (a main thread
/// não espera o disco) e avisa [`BrowserViews::on_cleared`].
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn remove_store_later(app: &AppHandle, key: ViewKey, reload: bool) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(home) = crate::session::cwd::home() {
            let dir = store_dir(&home, &key);
            if let Err(e) = remove_store_dir(&dir, 20, Duration::from_millis(250)) {
                eprintln!("[tyba] navegador do orbit: apagar os dados: {e}");
            }
        }
        app.state::<BrowserViews>().on_cleared(&app, &key, reload);
    });
}

/// `window.open()` sem url, ou com `about:blank`: nada a abrir. A página do
/// nó fica onde está (navegar o nó para `about:blank` apagaria a página).
pub fn new_window_ignored(url: &str) -> bool {
    let url = url.trim();
    url.is_empty()
        || url
            .get(..11)
            .is_some_and(|s| s.eq_ignore_ascii_case("about:blank"))
            && matches!(url.as_bytes().get(11), None | Some(b'?' | b'#'))
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

    #[test]
    fn one_agent_action_at_a_time_and_a_short_queue() {
        let lanes = std::sync::Arc::new(Lanes::default());
        let k = key("g", "b");
        let first = lanes.turn(&k, ACTION_TIMEOUT).unwrap();
        // Outro nó não espera.
        drop(lanes.turn(&key("g", "c"), ACTION_TIMEOUT).unwrap());
        // Fila cheia: "ocupado" na hora.
        lanes.lanes.lock().get_mut(&k).unwrap().1 = MAX_QUEUED;
        assert_eq!(lanes.turn(&k, ACTION_TIMEOUT).err(), Some(BUSY));
        lanes.lanes.lock().get_mut(&k).unwrap().1 = 0;
        // Esperando a vez: passa quando a primeira termina.
        let waiter = {
            let lanes = lanes.clone();
            let k = k.clone();
            std::thread::spawn(move || lanes.turn(&k, ACTION_TIMEOUT).map(|_| ()))
        };
        std::thread::sleep(Duration::from_millis(50));
        drop(first);
        assert_eq!(waiter.join().unwrap(), Ok(()));
        // E sem a vez dentro do prazo, "ocupado".
        let held = lanes.turn(&k, ACTION_TIMEOUT).unwrap();
        assert_eq!(lanes.turn(&k, Duration::from_millis(30)).err(), Some(BUSY));
        drop(held);
        assert!(lanes.lanes.lock().is_empty());
    }

    #[test]
    fn only_click_and_type_act_and_their_args_go_as_data() {
        let click = AgentAction::Click {
            reference: "e3".into(),
        };
        let typed = AgentAction::Type {
            reference: "e2".into(),
            text: "a\"); alert(1); (\"".into(),
            submit: true,
        };
        assert!(!AgentAction::Snapshot.acts());
        assert!(click.acts() && typed.acts());
        assert_eq!(click.args(), serde_json::json!({"ref": "e3"}));
        // O texto vai como valor JSON (argumento do script), intacto.
        assert_eq!(typed.args()["text"].as_str(), Some("a\"); alert(1); (\""));
        assert_eq!(typed.args()["submit"], true);
    }

    #[test]
    fn screenshots_are_numbered_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let first = next_shot_path(dir.path());
        assert_eq!(first, dir.path().join("screenshot-1.png"));
        write_shot(&first, b"png").unwrap();
        std::fs::write(dir.path().join("screenshot-7.png"), b"x").unwrap();
        std::fs::write(dir.path().join("screenshot-x.png"), b"x").unwrap();
        std::fs::write(dir.path().join("log.txt"), b"x").unwrap();
        assert_eq!(
            next_shot_path(dir.path()),
            dir.path().join("screenshot-8.png")
        );
        // Nunca sobrescreve.
        assert!(write_shot(&first, b"outro").is_err());
        assert_eq!(std::fs::read(&first).unwrap(), b"png");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn settle_waits_for_the_load_or_a_quiet_moment() {
        let ms = Duration::from_millis;
        // Sem navegação: 1,5 s de calma.
        assert!(!settled(false, false, ms(1_000)));
        assert!(settled(false, false, SETTLE_QUIET));
        // Navegou: até terminar de carregar.
        assert!(!settled(true, true, ms(2_000)));
        assert!(settled(true, false, ms(200)));
        // Nunca além do teto.
        assert!(settled(true, true, SETTLE_MAX));
    }

    #[test]
    fn agent_is_refused_while_the_owner_uses_the_page() {
        assert_eq!(agent_may_act(Focus::Owner), Err(OWNER_BUSY));
        assert_eq!(agent_may_act(Focus::None), Ok(()));
        assert_eq!(agent_may_act(Focus::Agent), Ok(()));
    }

    #[test]
    fn keys_route_escape_back_and_app_shortcuts_to_the_app() {
        let none = Mods::default();
        let shift = Mods {
            shift: true,
            ..none
        };
        let cmd = Mods {
            command: true,
            ..none
        };
        assert_eq!(route_key(KEY_ESCAPE, none, "\u{1b}"), KeyRoute::GiveBack);
        assert_eq!(route_key(KEY_ESCAPE, shift, "\u{1b}"), KeyRoute::Page);
        assert_eq!(route_key(KEY_ESCAPE, cmd, "\u{1b}"), KeyRoute::Page);
        for c in ["w", "k", "l", "K"] {
            assert_eq!(route_key(0, cmd, c), KeyRoute::ToApp, "{c}");
        }
        for c in ["c", "v", "a", "z"] {
            assert_eq!(route_key(0, cmd, c), KeyRoute::Page, "{c}");
        }
        assert_eq!(
            route_key(0, Mods { shift: true, ..cmd }, "w"),
            KeyRoute::Page
        );
        assert_eq!(route_key(0, none, "w"), KeyRoute::Page);
        // Windows e Linux: Esc pelo próprio código, Ctrl no lugar do ⌘.
        assert_eq!(route(true, none, ""), KeyRoute::GiveBack);
        assert_eq!(route(true, shift, ""), KeyRoute::Page);
        assert_eq!(route(false, cmd, "k"), KeyRoute::ToApp);
        assert_eq!(route(false, cmd, "c"), KeyRoute::Page);
    }

    #[test]
    fn only_the_three_app_shortcuts_are_forwarded() {
        let js = forward_shortcut_js("K").unwrap();
        assert!(js.contains(r#"key: "k""#) && js.contains(r#"code: "KeyK""#));
        assert!(js.contains("ctrlKey: true"));
        for c in ["c", "\"", "</script>", "kk", ""] {
            assert_eq!(forward_shortcut_js(c), None, "{c}");
        }
    }

    #[test]
    fn store_dir_is_per_node_inside_the_profile() {
        let home = std::path::Path::new("/home/eu");
        let a = store_dir(home, &key("g", "b"));
        assert!(a.starts_with(home.join(crate::profile::HOME_DIR).join("orbit/browser")));
        assert_eq!(a.file_name().unwrap().len(), 32);
        assert_eq!(a, store_dir(home, &key("g", "b")));
        assert_ne!(a, store_dir(home, &key("g", "c")));
    }

    #[cfg(unix)]
    #[test]
    fn store_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b");
        ensure_store_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    /// Depois do `retain`, o aviso atrasado da view (título, load, foco do
    /// agente) não recria a entrada órfã.
    #[test]
    fn late_updates_do_not_recreate_a_removed_entry() {
        let mut entries: HashMap<ViewKey, Entry> = HashMap::new();
        let k = key("g", "b");
        assert_eq!(
            change(&mut entries, &k, |e| e.state.title = "x".into()),
            None
        );
        assert!(entries.is_empty());
        entries.insert(k.clone(), Entry::fresh());
        let s = change(&mut entries, &k, |e| e.state.title = "x".into()).unwrap();
        assert_eq!(s.title, "x");
        // Sem mudança, nada a emitir.
        assert_eq!(change(&mut entries, &k, |e| e.dirty = true), None);
        entries.clear();
        assert_eq!(
            change(&mut entries, &k, |e| e.state.focus = Focus::Agent),
            None
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn removed_and_all_browsers_of_a_graph() {
        let browser = |id: &str| node(id, NodeKind::Browser { url: String::new() });
        let note = |id: &str| {
            node(
                id,
                NodeKind::Note {
                    markdown: String::new(),
                    locked: false,
                },
            )
        };
        let graph = |nodes: Vec<Node>| Graph {
            id: "g".into(),
            name: "g".into(),
            dir: "/ws".into(),
            nodes,
            edges: vec![],
            variables: Default::default(),
        };
        let old = graph(vec![browser("a"), browser("b"), note("n")]);
        let new = graph(vec![browser("b")]);
        // Só navegador apaga dados; o que ficou não.
        assert_eq!(removed_browsers(&old, &new), vec!["a".to_string()]);
        assert!(removed_browsers(&new, &old).is_empty());
        assert_eq!(browsers(&old), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn store_dir_removal_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = store_dir(tmp.path(), &key("g", "b"));
        ensure_store_dir(&dir.join("data/Cookies")).unwrap();
        std::fs::write(dir.join("data/Cookies/x"), "sessão").unwrap();
        let other = store_dir(tmp.path(), &key("g", "c"));
        ensure_store_dir(&other).unwrap();
        remove_store_dir(&dir, 3, Duration::from_millis(1)).unwrap();
        assert!(!dir.exists());
        // O de outro nó fica.
        assert!(other.exists());
        // Já apagada: nada a fazer.
        remove_store_dir(&dir, 3, Duration::from_millis(1)).unwrap();
    }

    #[test]
    fn empty_or_blank_new_window_is_ignored() {
        for url in [
            "",
            "  ",
            "about:blank",
            "ABOUT:BLANK",
            "about:blank#x",
            "about:blank?y",
        ] {
            assert!(new_window_ignored(url), "{url:?}");
        }
        for url in [
            "http://localhost:3000/",
            "about:blankx",
            "about:srcdoc",
            "/x",
        ] {
            assert!(!new_window_ignored(url), "{url:?}");
        }
    }

    /// A view que some (oclusão) com o dono usando perde o "você" e manda o
    /// teclado de volta; a que segue viva com ele, não.
    #[test]
    fn hiding_the_owner_view_releases_the_owner() {
        let mut entries: HashMap<ViewKey, Entry> = HashMap::new();
        let k = key("g", "b");
        let center = (0.0, 0.0);
        let placed = place_all(
            &mut entries,
            &k,
            Some((rect(0.0, 0.0), rect(0.0, 0.0))),
            1.0,
            center,
        );
        assert_eq!(placed.len(), 1);
        assert!(placed[0].1.visible && !placed[0].1.give_back);
        entries.get_mut(&k).unwrap().state.focus = Focus::Owner;
        // Mexeu, segue viva: o dono continua.
        let placed = place_all(
            &mut entries,
            &k,
            Some((rect(5.0, 0.0), rect(5.0, 0.0))),
            1.0,
            center,
        );
        assert!(!placed[0].1.give_back);
        assert_eq!(entries[&k].state.focus, Focus::Owner);
        // Some: o "você" sai e o estado emitido já diz isso.
        let placed = place_all(&mut entries, &k, None, 1.0, center);
        assert!(!placed[0].1.visible && placed[0].1.give_back);
        assert_eq!(placed[0].2.focus, Focus::None);
        assert_eq!(entries[&k].state.focus, Focus::None);
        // Esconder o que nem existe não cria nada.
        assert!(place_all(&mut entries, &key("g", "x"), None, 1.0, center).is_empty());
        assert!(!entries.contains_key(&key("g", "x")));
    }

    fn r(x: f64, y: f64, w: f64, h: f64) -> ViewRect {
        ViewRect { x, y, w, h }
    }

    #[test]
    fn visible_part_clips_to_the_marker_and_needs_a_clipping_system() {
        let marker = r(100.0, 400.0, 400.0, 300.0);
        // Inteiro: vale em todo sistema.
        assert_eq!(
            visible_part(Some(marker), None, false),
            Some((marker, marker))
        );
        assert_eq!(
            visible_part(Some(marker), Some(marker), false),
            Some((marker, marker))
        );
        // Cortado embaixo: só onde recorta.
        let bottom = r(100.0, 400.0, 400.0, 200.0);
        assert_eq!(
            visible_part(Some(marker), Some(bottom), true),
            Some((marker, bottom))
        );
        assert_eq!(visible_part(Some(marker), Some(bottom), false), None);
        // O recorte nunca passa do marcador.
        assert_eq!(
            visible_part(Some(marker), Some(r(0.0, 0.0, 450.0, 2000.0)), true),
            Some((marker, r(100.0, 400.0, 350.0, 300.0)))
        );
        // Nada à vista, recorte inválido ou sem marcador: esconde.
        assert_eq!(
            visible_part(Some(marker), Some(r(0.0, 0.0, 50.0, 50.0)), true),
            None
        );
        assert_eq!(
            visible_part(Some(marker), Some(r(f64::NAN, 0.0, 1.0, 1.0)), true),
            None
        );
        assert_eq!(visible_part(None, Some(bottom), true), None);
    }

    #[test]
    fn clip_frames_keep_the_page_in_place() {
        // contentView de 1000 de altura, origem embaixo (o padrão do macOS).
        let marker = r(100.0, 600.0, 400.0, 300.0);
        // Inteiro: o container é o marcador e a página ocupa todo ele.
        let (c, i) = clip_frames(marker, marker, 1000.0, false);
        assert_eq!(c, r(100.0, 100.0, 400.0, 300.0));
        assert_eq!(i, r(0.0, 0.0, 400.0, 300.0));
        // Cortado embaixo (só 200 à vista): o topo da página fica no topo
        // do container e o pé dela sai por baixo dele.
        let (c, i) = clip_frames(marker, r(100.0, 600.0, 400.0, 200.0), 1000.0, false);
        assert_eq!(c, r(100.0, 200.0, 400.0, 200.0));
        assert_eq!(i, r(0.0, -100.0, 400.0, 300.0));
        // Cortado em cima e à esquerda: a página desce e vai para a esquerda.
        let marker = r(-50.0, -40.0, 400.0, 300.0);
        let clip = r(0.0, 0.0, 350.0, 260.0);
        let (c, i) = clip_frames(marker, clip, 1000.0, false);
        assert_eq!(c, r(0.0, 740.0, 350.0, 260.0));
        assert_eq!(i, r(-50.0, 0.0, 400.0, 300.0));
        // Em coordenadas de janela, a página continua no marcador: topo do
        // container (740 + 260 = 1000) + o que sobra acima (300 - 260).
        assert_eq!(c.y + i.y + i.h, 1000.0 + 40.0);
        // contentView com origem em cima: sem inverter.
        let (c, i) = clip_frames(marker, clip, 1000.0, true);
        assert_eq!(c, r(0.0, 0.0, 350.0, 260.0));
        assert_eq!(i, r(-50.0, -40.0, 400.0, 300.0));
    }

    #[test]
    fn snapshot_rule() {
        use ShotTrigger::*;
        for t in [Loaded, OwnerLeft, AgentAction] {
            assert!(should_shoot(t, false, false, 0, 0));
        }
        let t0 = 1_000_000;
        assert!(should_shoot(Tick, true, true, t0, t0 + SHOT_EVERY_MS));
        assert!(!should_shoot(Tick, true, true, t0, t0 + SHOT_EVERY_MS - 1));
        assert!(!should_shoot(Tick, false, true, t0, t0 + 60_000));
        assert!(!should_shoot(Tick, true, false, t0, t0 + 60_000));
    }
}
