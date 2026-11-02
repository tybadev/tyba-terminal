//! O que as três views nativas (`macos.rs`, `windows.rs`, `linux.rs`)
//! dividem: os scripts do mundo isolado, o tamanho da view escondida e o
//! `orbit-file`, servido só de dentro das raízes do nó.

use std::borrow::Cow;
use std::path::PathBuf;

use wry::http::{header::CONTENT_TYPE, Request, Response, StatusCode};

use super::policy;

/// Teto do arquivo servido pelo `orbit-file`.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Tamanho da view antes de o card mandar o retângulo (pontos).
pub(super) const HIDDEN_W: f64 = 1280.0;
pub(super) const HIDDEN_H: f64 = 800.0;

/// Largura máxima da prévia do card, em pontos.
pub(super) const PREVIEW_MAX_W: f64 = 640.0;

/// O mundo isolado das ferramentas do agente.
pub(super) const WORLD: &str = "tyba-orbit";
/// Refs, snapshot, ações e o buffer do console (mundo isolado).
pub(super) const ISOLATED_JS: &str = include_str!("scripts/isolated.js");
/// Reemite `console.error/warn` para o buffer (mundo da página).
pub(super) const PAGE_SHIM_JS: &str = include_str!("scripts/page_shim.js");
/// O corpo de cada ação: garante o `__orbit` (página que carregou antes do
/// user script) e chama a ação com os argumentos — `action` e `args` são
/// argumentos da chamada, nunca interpolados.
pub(super) const RUN_JS: &str =
    "return JSON.stringify(await globalThis.__orbit.run(action, JSON.parse(args)));";

/// `orbit-file://r<i>/<caminho>`: só arquivo de dentro da raiz `i`.
pub(super) fn serve_file(
    roots: &[PathBuf],
    request: &Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
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

/// Largura e altura de um PNG, lidas do cabeçalho `IHDR`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || &png[..8] != SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_size_reads_the_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&1600u32.to_be_bytes());
        png.extend_from_slice(&900u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((1600, 900)));
        assert_eq!(png_size(&png[..20]), None);
        assert_eq!(png_size(b"GIF89a................................"), None);
    }
}
