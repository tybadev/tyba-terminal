use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

const CHUNK: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_REDIRECTS: usize = 5;

const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
    "releases.hashicorp.com",
    "registry.npmjs.org",
    "nodejs.org",
];

pub fn host_allowed(url: &str) -> bool {
    match reqwest::Url::parse(url) {
        Ok(u) => {
            u.scheme() == "https"
                && u.host_str()
                    .map(|h| ALLOWED_HOSTS.contains(&h))
                    .unwrap_or(false)
        }
        Err(_) => false,
    }
}

#[derive(Debug)]
pub enum DownloadError {
    Network(String),
    Checksum { expected: String, got: String },
    Size { expected: u64, got: u64 },
    Io(String),
}

impl DownloadError {
    pub fn message(&self) -> String {
        match self {
            DownloadError::Network(e) => {
                format!("falha de rede ao baixar o language server: {e} — tente de novo")
            }
            DownloadError::Checksum { expected, got } => format!(
                "checksum divergente: o artefato baixado não bate com o pin (esperado {expected}, veio {got}) — download descartado"
            ),
            DownloadError::Size { expected, got } => format!(
                "tamanho divergente: esperado {expected} bytes, veio {got} — download descartado"
            ),
            DownloadError::Io(e) => format!("erro de disco ao gravar o artefato: {e}"),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    out
}

pub fn verify_stream<R: Read, W: Write, P: FnMut(u64)>(
    mut reader: R,
    mut sink: W,
    expected_sha256: &str,
    expected_size: u64,
    mut progress: P,
) -> Result<(), DownloadError> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        sink.write_all(&buf[..n])
            .map_err(|e| DownloadError::Io(e.to_string()))?;
        done += n as u64;
        if done > expected_size {
            return Err(DownloadError::Size {
                expected: expected_size,
                got: done,
            });
        }
        progress(done);
    }
    sink.flush().map_err(|e| DownloadError::Io(e.to_string()))?;
    if done != expected_size {
        return Err(DownloadError::Size {
            expected: expected_size,
            got: done,
        });
    }
    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(expected_sha256) {
        return Err(DownloadError::Checksum {
            expected: expected_sha256.to_string(),
            got,
        });
    }
    Ok(())
}

pub fn fetch_to_file<P: FnMut(u64)>(
    url: &str,
    dest_tmp: &Path,
    expected_sha256: &str,
    expected_size: u64,
    progress: P,
) -> Result<(), DownloadError> {
    if !host_allowed(url) {
        return Err(DownloadError::Network(format!(
            "URL do pin fora da allowlist de hosts oficiais: {url}"
        )));
    }
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error(std::io::Error::other("redirects demais"));
        }
        let ok = attempt.url().scheme() == "https"
            && attempt
                .url()
                .host_str()
                .map(|h| ALLOWED_HOSTS.contains(&h))
                .unwrap_or(false);
        if ok {
            attempt.follow()
        } else {
            attempt.error(std::io::Error::other(
                "redirect para host fora da allowlist",
            ))
        }
    });
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TRANSFER_TIMEOUT)
        .redirect(redirect)
        .user_agent(concat!("tyba/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| DownloadError::Network(e.to_string()))?;
    if !response.status().is_success() {
        return Err(DownloadError::Network(format!(
            "status HTTP {}",
            response.status()
        )));
    }
    let file = std::fs::File::create(dest_tmp).map_err(|e| DownloadError::Io(e.to_string()))?;
    let writer = std::io::BufWriter::new(file);
    let result = verify_stream(response, writer, expected_sha256, expected_size, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(dest_tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sha256_hex(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    #[test]
    fn accepts_a_stream_whose_hash_and_size_match() {
        let data = b"conteudo binario sintetico do server";
        let sha = sha256_hex(data);
        let mut sink = Vec::new();
        let mut seen = 0u64;
        verify_stream(
            Cursor::new(data.to_vec()),
            &mut sink,
            &sha,
            data.len() as u64,
            |done| seen = done,
        )
        .unwrap();
        assert_eq!(sink, data);
        assert_eq!(seen, data.len() as u64);
    }

    #[test]
    fn rejects_a_stream_whose_hash_diverges() {
        let data = b"artefato adulterado";
        let wrong = "0".repeat(64);
        let mut sink = Vec::new();
        let err = verify_stream(
            Cursor::new(data.to_vec()),
            &mut sink,
            &wrong,
            data.len() as u64,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Checksum { .. }));
    }

    #[test]
    fn rejects_a_stream_that_is_shorter_than_the_pinned_size() {
        let data = b"curto demais";
        let sha = sha256_hex(data);
        let mut sink = Vec::new();
        let err = verify_stream(
            Cursor::new(data.to_vec()),
            &mut sink,
            &sha,
            (data.len() as u64) + 10,
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Size { .. }));
    }

    #[test]
    fn rejects_a_stream_that_overshoots_the_pinned_size() {
        let data = b"conteudo maior que o pin declarava";
        let sha = sha256_hex(data);
        let mut sink = Vec::new();
        let err =
            verify_stream(Cursor::new(data.to_vec()), &mut sink, &sha, 5, |_| {}).unwrap_err();
        assert!(matches!(err, DownloadError::Size { .. }));
    }

    #[test]
    fn hex_is_lowercase_and_zero_padded() {
        assert_eq!(hex(&[0x0a, 0xff, 0x00]), "0aff00");
    }

    #[test]
    fn host_allowlist_accepts_official_hosts_and_rejects_the_rest() {
        assert!(host_allowed(
            "https://github.com/rust-lang/rust-analyzer/releases/download/x/y.gz"
        ));
        assert!(host_allowed(
            "https://registry.npmjs.org/pyright/-/pyright-1.tgz"
        ));
        assert!(host_allowed(
            "https://release-assets.githubusercontent.com/abc"
        ));
        assert!(!host_allowed("http://github.com/x"), "HTTP nunca");
        assert!(
            !host_allowed("https://evil.example.com/rust-analyzer.gz"),
            "host arbitrário com sha casando ainda é recusado"
        );
        assert!(
            !host_allowed("https://github.com.evil.com/x"),
            "sufixo enganoso não é o host oficial"
        );
    }
}
