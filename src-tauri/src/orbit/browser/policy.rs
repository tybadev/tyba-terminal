//! Regra de navegação do navegador nativo do nó (Fase 5, `fase5-navegador`).
//!
//! Conferida em **toda** navegação, de qualquer frame: só endereço local (a
//! régua do salvar, [`service::is_local_url`]), menos os hosts internos do
//! Tauri; `about:blank`; e o esquema próprio `orbit-file`, que o core serve só
//! de dentro das raízes do nó. `file://` cru nunca carrega: o core o troca por
//! `orbit-file` quando o arquivo está dentro de uma raiz ([`resolve_file`]).
//!
//! Por que um esquema próprio e não `file://`: só o macOS tem leitura
//! restrita a uma pasta (`loadFileURL:allowingReadAccessToURL:`); o WebView2 e
//! o WebKitGTK não. Com o esquema, a regra é a mesma nos três sistemas.

use std::path::{Component, Path, PathBuf};

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};

use crate::orbit::service;

/// O esquema que serve arquivo local preso às raízes do nó.
pub const FILE_SCHEME: &str = "orbit-file";

/// Hosts que passariam no `*.localhost` mas são do Tauri no Windows: fora da
/// webview do app não servem nada, mas o nome engana.
const TAURI_HOSTS: [&str; 2] = ["tauri.localhost", "ipc.localhost"];

/// O WebView2 só aceita esquema próprio na forma `http://<esquema>.<host>`:
/// o wry intercepta esse endereço e entrega o pedido ao handler já de volta
/// como `orbit-file://<host>/…` (nunca vai à rede).
pub const WINDOWS_FILE_PREFIX: &str = "http://orbit-file.";

/// `orbit-file://r0/x.html` → `http://orbit-file.r0/x.html` (o que o
/// WebView2 carrega). Outra url passa igual.
pub fn to_windows(url: &str) -> String {
    match url.strip_prefix(&format!("{FILE_SCHEME}://")) {
        Some(rest) => format!("{WINDOWS_FILE_PREFIX}{rest}"),
        None => url.to_string(),
    }
}

/// O inverso de [`to_windows`]: o endereço que o WebView2 informa volta a
/// `orbit-file://`, para a política e a barra de endereço.
pub fn from_windows(url: &str) -> String {
    let head = url.get(..WINDOWS_FILE_PREFIX.len());
    match head {
        Some(h) if h.eq_ignore_ascii_case(WINDOWS_FILE_PREFIX) => {
            format!("{FILE_SCHEME}://{}", &url[WINDOWS_FILE_PREFIX.len()..])
        }
        _ => url.to_string(),
    }
}

/// O caminho de um `file://` no disco: no Windows `file:///C:/x` chega como
/// `/C:/x`, e a barra antes da letra do disco sai.
fn file_url_path(path: &str, windows: bool) -> &str {
    let b = path.as_bytes();
    if windows && b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        &path[1..]
    } else {
        path
    }
}

/// A raiz como começo de `file://`: no Windows sem o `\\?\` do caminho
/// canônico e com `/` (`file:///C:/ws`).
fn file_url_root(root: &str, windows: bool) -> String {
    if !windows {
        return root.trim_end_matches('/').to_string();
    }
    let root = root
        .strip_prefix(r"\\?\")
        .unwrap_or(root)
        .replace('\\', "/");
    format!("/{}", root.trim_end_matches('/'))
}

/// O que a navegação pode fazer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// O código do motivo (o card traduz: `orbitBrowserRefused_<código>`).
    Deny(&'static str),
}

/// Recusa de endereço não local.
pub const DENY_NOT_LOCAL: &str = "notLocal";
/// Recusa dos hosts internos do Tauri.
pub const DENY_TAURI_HOST: &str = "tauriHost";
/// Recusa de `file://` cru (só entra pelo `orbit-file`).
pub const DENY_RAW_FILE: &str = "rawFile";

/// A regra de cada navegação, em qualquer frame.
pub fn decide(url: &str) -> Decision {
    let lower = url.trim().to_ascii_lowercase();
    if lower == "about:blank" || lower == "about:srcdoc" {
        return Decision::Allow;
    }
    if service::is_local_url(url) {
        return match host_of(&lower) {
            Some(h) if TAURI_HOSTS.contains(&h) => Decision::Deny(DENY_TAURI_HOST),
            _ => Decision::Allow,
        };
    }
    if lower.starts_with(&format!("{FILE_SCHEME}://")) && !url.chars().any(char::is_control) {
        return Decision::Allow;
    }
    if lower.starts_with("file:") {
        return Decision::Deny(DENY_RAW_FILE);
    }
    Decision::Deny(DENY_NOT_LOCAL)
}

/// O host de uma url `http(s)` já aprovada por `is_local_url` (minúscula).
fn host_of(lower: &str) -> Option<&str> {
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.starts_with('[') {
        return Some(authority.split(']').next().unwrap_or(authority));
    }
    Some(authority.split(':').next().unwrap_or(authority))
}

/// Recusa de um `file://` que o dono ou o agente pediu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRefusal {
    NotFileUrl,
    NotHtml,
    NotFound,
    OutsideRoots,
}

impl FileRefusal {
    /// O código que o card traduz (`orbitBrowserRefused_<código>`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFileUrl => "notFileUrl",
            Self::NotHtml => "notHtml",
            Self::NotFound => "notFound",
            Self::OutsideRoots => "outsideRoots",
        }
    }

    /// O motivo em texto, para quem lê sem tradução (o agente).
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotFileUrl => "não é um endereço file://",
            Self::NotHtml => "só arquivo .html ou .htm abre aqui",
            Self::NotFound => "arquivo não encontrado",
            Self::OutsideRoots => "arquivo fora da pasta do workspace ou do nó",
        }
    }
}

/// O que não é codificado no caminho do `orbit-file` (o resto vira `%XX`).
const PATH_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'\\');

/// Raízes canônicas (symlink resolvido); as que não existem saem.
pub fn canonical_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for r in roots {
        if r.as_os_str().is_empty() {
            continue;
        }
        if let Ok(c) = std::fs::canonicalize(r) {
            if c.is_dir() && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// `file:///…/x.html` → `orbit-file://r<i>/<relativo>`: canonicaliza
/// (resolve symlink), exige `.html`/`.htm`, e exige o arquivo **dentro** de
/// uma das `roots` (já canônicas, ver [`canonical_roots`]).
pub fn resolve_file(url: &str, roots: &[PathBuf]) -> Result<String, FileRefusal> {
    let trimmed = url.trim();
    let Some(rest) = trimmed
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("file://"))
        .map(|_| &trimmed[7..])
    else {
        return Err(FileRefusal::NotFileUrl);
    };
    // `file://localhost/x` vale como `file:///x`; outro host não.
    let path = if let Some(p) = rest.strip_prefix("localhost/") {
        format!("/{p}")
    } else if rest.starts_with('/') {
        rest.to_string()
    } else {
        return Err(FileRefusal::NotFileUrl);
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    let path = file_url_path(path, cfg!(windows));
    let decoded = percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| FileRefusal::NotFound)?;
    if decoded.contains('\0') {
        return Err(FileRefusal::NotFound);
    }
    let ext_ok = Path::new(decoded.as_ref())
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
    if !ext_ok {
        return Err(FileRefusal::NotHtml);
    }
    let canon = std::fs::canonicalize(decoded.as_ref()).map_err(|_| FileRefusal::NotFound)?;
    if !canon.is_file() {
        return Err(FileRefusal::NotFound);
    }
    // O symlink pode apontar para um arquivo sem `.html`: confere de novo.
    let canon_ext_ok = canon
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
    if !canon_ext_ok {
        return Err(FileRefusal::NotHtml);
    }
    for (i, root) in roots.iter().enumerate() {
        if let Ok(rel) = canon.strip_prefix(root) {
            let rel = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(s) => s.to_str(),
                    _ => None,
                })
                .map(|s| utf8_percent_encode(s, PATH_SET).to_string())
                .collect::<Vec<_>>()
                .join("/");
            return Ok(format!("{FILE_SCHEME}://r{i}/{rel}"));
        }
    }
    Err(FileRefusal::OutsideRoots)
}

/// As extensões que o `orbit-file` serve: só arquivo estático de site. O
/// resto da pasta (`.env`, chave, banco, código) não sai, nem para um HTML
/// da própria pasta que o leia por `fetch`.
const WEB_EXTENSIONS: &[&str] = &[
    "html", "htm", "css", "js", "mjs", "json", "map", "png", "jpg", "jpeg", "gif", "svg", "webp",
    "ico", "woff", "woff2", "ttf", "txt", "md",
];

/// O arquivo tem extensão de site estático ([`WEB_EXTENSIONS`])?
fn web_static(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| WEB_EXTENSIONS.iter().any(|w| e.eq_ignore_ascii_case(w)))
}

/// O arquivo que o `orbit-file` serve: `host` = `r<i>` (a raiz), `path` o
/// caminho da url. Sem NUL, sem segmento que comece com `.` (`..`,
/// dotfile e dotdir: `.env`, `.git/`), só extensão de site estático
/// ([`WEB_EXTENSIONS`]), e o caminho canônico (symlink resolvido) tem de
/// continuar dentro da raiz e obedecer às mesmas regras. `None` = 404.
pub fn serve_path(host: &str, path: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let index: usize = host.strip_prefix('r')?.parse().ok()?;
    let root = roots.get(index)?;
    let decoded = percent_decode_str(path).decode_utf8().ok()?;
    if decoded.contains('\0') || decoded.contains('\\') {
        return None;
    }
    let mut full = root.clone();
    for part in decoded.split('/') {
        match part {
            "" => continue,
            p if p.starts_with('.') => return None,
            p => full.push(p),
        }
    }
    if !web_static(&full) {
        return None;
    }
    let canon = std::fs::canonicalize(&full).ok()?;
    let rel = canon.strip_prefix(root).ok()?;
    let hidden = rel.components().any(|c| match c {
        Component::Normal(s) => s.to_string_lossy().starts_with('.'),
        _ => true,
    });
    (!hidden && web_static(&canon) && canon.is_file()).then_some(canon)
}

/// O `orbit-file://r<i>/<rel>` de volta ao `file://` que o dono reconhece
/// (a barra de endereço mostra este). Outra url passa igual.
pub fn display_url(url: &str, roots: &[PathBuf]) -> String {
    let url = if cfg!(windows) {
        from_windows(url)
    } else {
        url.to_string()
    };
    let Some(rest) = url.strip_prefix(&format!("{FILE_SCHEME}://")) else {
        return url.to_string();
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split(['?', '#']).next().unwrap_or("");
    let Some(root) = host
        .strip_prefix('r')
        .and_then(|i| i.parse::<usize>().ok())
        .and_then(|i| roots.get(i))
    else {
        return url.to_string();
    };
    let root = file_url_root(&root.to_string_lossy(), cfg!(windows));
    format!("file://{root}/{path}")
}

/// Content-Type do arquivo servido pelo `orbit-file`, pela extensão.
pub fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_urls_are_allowed() {
        for ok in [
            "about:blank",
            "http://localhost:5173/",
            "https://code.muriki.test/login?x=1",
            "http://127.0.0.1:8080",
            "http://[::1]:3000/a",
            "http://app.localhost/",
            "orbit-file://r0/index.html",
        ] {
            assert_eq!(decide(ok), Decision::Allow, "{ok}");
        }
    }

    #[test]
    fn tauri_hosts_public_and_odd_schemes_are_refused() {
        for (bad, why) in [
            ("http://tauri.localhost/", DENY_TAURI_HOST),
            ("https://ipc.localhost/cmd", DENY_TAURI_HOST),
            ("http://TAURI.localhost:80/", DENY_TAURI_HOST),
            ("https://example.com/", DENY_NOT_LOCAL),
            ("http://localhost@evil.com/", DENY_NOT_LOCAL),
            ("data:text/html,<b>x</b>", DENY_NOT_LOCAL),
            ("blob:http://localhost:5173/abc", DENY_NOT_LOCAL),
            ("javascript:alert(1)", DENY_NOT_LOCAL),
            ("tauri://localhost", DENY_NOT_LOCAL),
            ("ipc://localhost/x", DENY_NOT_LOCAL),
            ("file:///etc/passwd", DENY_RAW_FILE),
            ("FILE:///tmp/x.html", DENY_RAW_FILE),
            ("ftp://localhost/", DENY_NOT_LOCAL),
        ] {
            assert_eq!(decide(bad), Decision::Deny(why), "{bad}");
        }
    }

    fn tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(root.join("site/css")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(root.join("site/index.html"), "<h1>oi</h1>").unwrap();
        std::fs::write(root.join("site/a b.HTM"), "x").unwrap();
        std::fs::write(root.join("site/css/app.css"), "b{}").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        std::fs::write(other.join("secret.html"), "x").unwrap();
        (tmp, root, other)
    }

    fn file_url(p: &Path) -> String {
        format!("file://{}", p.display())
    }

    #[test]
    fn file_inside_root_with_html_resolves_to_orbit_file() {
        let (_tmp, root, _other) = tree();
        let roots = canonical_roots(std::slice::from_ref(&root));
        assert_eq!(
            resolve_file(&file_url(&root.join("site/index.html")), &roots),
            Ok("orbit-file://r0/site/index.html".into())
        );
        // Espaço codificado na url e extensão maiúscula.
        let url = format!("file://{}/site/a%20b.HTM", root.display());
        assert_eq!(
            resolve_file(&url, &roots),
            Ok("orbit-file://r0/site/a%20b.HTM".into())
        );
    }

    #[test]
    fn file_outside_dotdot_symlink_and_txt_are_refused() {
        let (_tmp, root, other) = tree();
        let roots = canonical_roots(std::slice::from_ref(&root));
        assert_eq!(
            resolve_file(&file_url(&root.join("notes.txt")), &roots),
            Err(FileRefusal::NotHtml)
        );
        // Pasta de outro nó (fora das raízes).
        assert_eq!(
            resolve_file(&file_url(&other.join("secret.html")), &roots),
            Err(FileRefusal::OutsideRoots)
        );
        // `..` que sai da raiz: canonicaliza e cai fora.
        let dotdot = format!("file://{}/../other/secret.html", root.display());
        assert_eq!(
            resolve_file(&dotdot, &roots),
            Err(FileRefusal::OutsideRoots)
        );
        // Symlink dentro da raiz apontando para fora.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(other.join("secret.html"), root.join("link.html")).unwrap();
            assert_eq!(
                resolve_file(&file_url(&root.join("link.html")), &roots),
                Err(FileRefusal::OutsideRoots)
            );
            // Symlink `.html` apontando para um `.txt` dentro da raiz.
            std::os::unix::fs::symlink(root.join("notes.txt"), root.join("t.html")).unwrap();
            assert_eq!(
                resolve_file(&file_url(&root.join("t.html")), &roots),
                Err(FileRefusal::NotHtml)
            );
        }
        assert_eq!(
            resolve_file("file://outro-host/x.html", &roots),
            Err(FileRefusal::NotFileUrl)
        );
        assert_eq!(
            resolve_file("http://localhost/x.html", &roots),
            Err(FileRefusal::NotFileUrl)
        );
        assert_eq!(
            resolve_file(&file_url(&root.join("nada.html")), &roots),
            Err(FileRefusal::NotFound)
        );
    }

    #[test]
    fn serve_path_stays_inside_the_root() {
        let (_tmp, root, other) = tree();
        let roots = canonical_roots(std::slice::from_ref(&root));
        let canon_root = &roots[0];
        assert_eq!(
            serve_path("r0", "/site/css/app.css", &roots),
            Some(canon_root.join("site/css/app.css"))
        );
        assert_eq!(
            serve_path("r0", "/site/a%20b.HTM", &roots),
            Some(canon_root.join("site/a b.HTM"))
        );
        assert_eq!(
            serve_path("r0", "/site/../../other/secret.html", &roots),
            None
        );
        assert_eq!(
            serve_path("r0", "/site/%2e%2e/%2e%2e/other/secret.html", &roots),
            None
        );
        assert_eq!(serve_path("r1", "/site/index.html", &roots), None);
        assert_eq!(serve_path("x0", "/site/index.html", &roots), None);
        assert_eq!(serve_path("r0", "/site", &roots), None);
        assert_eq!(serve_path("r0", "/site/a%00.html", &roots), None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&other, root.join("fora")).unwrap();
            assert_eq!(serve_path("r0", "/fora/secret.html", &roots), None);
        }
    }

    /// Um HTML da pasta lê por `fetch` o que o `orbit-file` servir: só sai
    /// arquivo de site, nunca dotfile, dotdir nem outro tipo.
    #[test]
    fn serve_path_serves_only_static_web_files() {
        let (_tmp, root, _other) = tree();
        let roots = canonical_roots(std::slice::from_ref(&root));
        let canon_root = &roots[0];
        std::fs::write(root.join(".env"), "SEGREDO=1").unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/config.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("site/.cache")).unwrap();
        std::fs::write(root.join("site/.cache/app.js"), "x").unwrap();
        std::fs::write(root.join("site/.hidden.html"), "x").unwrap();
        std::fs::write(root.join("dados.db"), "x").unwrap();
        std::fs::write(root.join("id_ed25519"), "x").unwrap();
        std::fs::write(root.join("app.wasm"), "x").unwrap();
        for ext in ["png", "woff2", "md", "json", "SVG"] {
            std::fs::write(root.join(format!("site/f.{ext}")), "x").unwrap();
            assert_eq!(
                serve_path("r0", &format!("/site/f.{ext}"), &roots),
                Some(canon_root.join(format!("site/f.{ext}"))),
                "{ext}"
            );
        }
        for path in [
            "/.env",
            "/%2eenv",
            "/.git/config.json",
            "/site/.cache/app.js",
            "/site/.hidden.html",
            "/site/./index.html",
            "/dados.db",
            "/id_ed25519",
            "/app.wasm",
        ] {
            assert_eq!(serve_path("r0", path, &roots), None, "{path}");
        }
        #[cfg(unix)]
        {
            // Nome de site apontando para o que não é site: o alvo conta.
            std::os::unix::fs::symlink(root.join(".env"), root.join("site/env.txt")).unwrap();
            std::os::unix::fs::symlink(root.join("dados.db"), root.join("site/db.css")).unwrap();
            assert_eq!(serve_path("r0", "/site/env.txt", &roots), None);
            assert_eq!(serve_path("r0", "/site/db.css", &roots), None);
        }
    }

    #[test]
    fn display_url_maps_back_to_file() {
        let roots = vec![PathBuf::from("/Users/eu/ws")];
        assert_eq!(
            display_url("orbit-file://r0/site/index.html?x#y", &roots),
            "file:///Users/eu/ws/site/index.html"
        );
        assert_eq!(
            display_url("orbit-file://r9/x.html", &roots),
            "orbit-file://r9/x.html"
        );
        assert_eq!(
            display_url("http://localhost:1/", &roots),
            "http://localhost:1/"
        );
    }

    #[test]
    fn windows_form_of_orbit_file_round_trips() {
        assert_eq!(
            to_windows("orbit-file://r0/site/a%20b.html?x"),
            "http://orbit-file.r0/site/a%20b.html?x"
        );
        assert_eq!(to_windows("http://localhost:1/"), "http://localhost:1/");
        assert_eq!(
            from_windows("HTTP://orbit-file.r2/x.html"),
            "orbit-file://r2/x.html"
        );
        assert_eq!(from_windows("http://localhost:1/"), "http://localhost:1/");
        // O host do wry não é local: só passa na política depois de voltar.
        assert_eq!(
            decide("http://orbit-file.r0/x.html"),
            Decision::Deny(DENY_NOT_LOCAL)
        );
        assert_eq!(
            decide(&from_windows("http://orbit-file.r0/x.html")),
            Decision::Allow
        );
    }

    #[test]
    fn windows_file_paths_and_roots() {
        assert_eq!(file_url_path("/C:/ws/x.html", true), "C:/ws/x.html");
        assert_eq!(file_url_path("/C:/ws/x.html", false), "/C:/ws/x.html");
        assert_eq!(file_url_path("/ws/x.html", true), "/ws/x.html");
        assert_eq!(
            file_url_root(r"\\?\C:\Users\eu\ws", true),
            "/C:/Users/eu/ws"
        );
        assert_eq!(file_url_root("/Users/eu/ws/", false), "/Users/eu/ws");
    }

    #[test]
    fn content_type_by_extension() {
        assert_eq!(
            content_type(Path::new("a/b.HTML")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("x.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("x")), "application/octet-stream");
    }
}
