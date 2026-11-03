//! O processo vivo de verdade: o comando roda no PTY (cor, sem buffer de
//! pipe) e o core grava o `log.txt` a partir do fluxo — sem ANSI, redigido
//! linha a linha, com teto e rotação simples. A url que o processo imprime
//! sai daqui também, e a escolha da porta da varredura.
//!
//! Tudo puro e testado com texto escrito à mão; quem liga ao PTY é o
//! [`crate::pty::OutputTap`] que o run monta.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Teto de cada arquivo do log. Passou: o atual vira `log.1.txt` (o de antes
/// sai) e o `log.txt` recomeça — no disco, no máximo duas vezes isto.
pub const LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Linha sem `\n` que passa disto sai assim mesmo (barra de progresso sem
/// fim não segura memória).
const LINE_MAX_BYTES: usize = 64 * 1024;
/// O "Ver log inteiro" lê até isto do fim (os dois arquivos juntos).
pub const LOG_VIEW_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Onde a sequência de controle está, entre um chunk e outro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum Esc {
    None,
    /// Viu `ESC`.
    Start,
    /// `ESC [` … até o byte final (`@`..`~`).
    Csi,
    /// `ESC ]` … até `BEL` ou `ESC \`.
    Osc,
    /// `ESC` dentro do OSC: `\` fecha.
    OscEsc,
}

/// Fluxo do PTY → linhas de texto: tira CSI/OSC, os outros controles (fica o
/// `\t`), e o `\r` sozinho recomeça a linha como no terminal (a barra de
/// progresso fica só com o último estado).
#[derive(Debug)]
pub struct LineSplitter {
    esc: Esc,
    line: Vec<u8>,
    /// Viu `\r`: se vier `\n` é fim de linha; senão a linha recomeça.
    cr: bool,
}

impl Default for LineSplitter {
    fn default() -> Self {
        Self {
            esc: Esc::None,
            line: Vec::new(),
            cr: false,
        }
    }
}

impl LineSplitter {
    /// Devolve as linhas que fecharam neste chunk.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in bytes {
            match self.esc {
                Esc::Start => {
                    self.esc = match b {
                        b'[' => Esc::Csi,
                        b']' => Esc::Osc,
                        // ESC + um byte (ESC 7, ESC =, ESC ( B…): some junto.
                        _ => Esc::None,
                    };
                    continue;
                }
                Esc::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.esc = Esc::None;
                    }
                    continue;
                }
                Esc::Osc => {
                    match b {
                        0x07 => self.esc = Esc::None,
                        0x1b => self.esc = Esc::OscEsc,
                        _ => {}
                    }
                    continue;
                }
                Esc::OscEsc => {
                    self.esc = if b == b'\\' { Esc::None } else { Esc::Osc };
                    continue;
                }
                Esc::None => {}
            }
            if self.cr && b != b'\n' {
                self.cr = false;
                self.line.clear();
            }
            match b {
                0x1b => self.esc = Esc::Start,
                b'\n' => {
                    self.cr = false;
                    out.push(self.take());
                }
                b'\r' => self.cr = true,
                b'\t' => self.line.push(b),
                0x08 => {
                    self.line.pop();
                }
                b if b < 0x20 || b == 0x7f => {}
                b => {
                    self.line.push(b);
                    if self.line.len() >= LINE_MAX_BYTES {
                        out.push(self.take());
                    }
                }
            }
        }
        out
    }

    /// O resto sem `\n` (o fim do processo).
    pub fn finish(&mut self) -> Option<String> {
        (!self.line.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> String {
        let line = String::from_utf8_lossy(&self.line).into_owned();
        self.line.clear();
        line
    }
}

/// Quem recebe cada linha limpa.
type OnLine = Box<dyn FnMut(&str) + Send>;

/// O `log.txt` do nó: cada linha limpa passa pela redação antes do disco.
pub struct LogWriter {
    path: PathBuf,
    file: Option<BufWriter<File>>,
    written: u64,
    max: u64,
    split: LineSplitter,
    /// Cada linha limpa, antes da redação (a url impressa sai dela).
    on_line: Option<OnLine>,
}

impl LogWriter {
    /// Cria (ou zera) o arquivo, 0600.
    pub fn create(path: &Path, max: u64) -> std::io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(BufWriter::new(open_private(path)?)),
            written: 0,
            max,
            split: LineSplitter::default(),
            on_line: None,
        })
    }

    pub fn on_line(mut self, f: impl FnMut(&str) + Send + 'static) -> Self {
        self.on_line = Some(Box::new(f));
        self
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for line in self.split.feed(bytes) {
            self.line(&line);
        }
        // O card lê o arquivo com o processo rodando: o que fechou vai ao disco.
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }

    fn line(&mut self, line: &str) {
        if let Some(f) = self.on_line.as_mut() {
            f(line);
        }
        let clean = crate::session::redact::redact(line);
        let size = clean.len() as u64 + 1;
        if self.written > 0 && self.written + size > self.max {
            self.rotate();
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if writeln!(file, "{clean}").is_ok() {
            self.written += size;
        }
    }

    /// `log.txt` → `log.1.txt` (o de antes sai) e recomeça.
    fn rotate(&mut self) {
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
        }
        let _ = std::fs::rename(&self.path, rotated(&self.path));
        self.file = open_private(&self.path).ok().map(BufWriter::new);
        self.written = 0;
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        if let Some(rest) = self.split.finish() {
            self.line(&rest);
        }
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }
}

/// O arquivo de antes da rotação.
pub fn rotated(path: &Path) -> PathBuf {
    path.with_file_name("log.1.txt")
}

fn open_private(path: &Path) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// O log inteiro do nó (o rotacionado antes do atual), até `max` bytes do
/// fim. `true` quando o começo ficou de fora. O texto já foi gravado limpo
/// e redigido; passa pela redação de novo assim mesmo (o `log.txt` de antes
/// desta versão era cru).
pub fn read_whole(path: &Path, max: u64) -> Option<(String, bool)> {
    let mut bytes = std::fs::read(rotated(path)).unwrap_or_default();
    let current = std::fs::read(path).ok();
    if current.is_none() && bytes.is_empty() {
        return None;
    }
    bytes.extend(current.unwrap_or_default());
    let cut = bytes.len() as u64 > max;
    if cut {
        bytes.drain(..bytes.len() - max as usize);
    }
    let text = super::orchestrator::strip_ansi(&String::from_utf8_lossy(&bytes));
    let mut text = crate::session::redact::redact(&text).into_owned();
    if cut {
        // A primeira linha veio pela metade.
        text = text
            .split_once('\n')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or(text);
    }
    Some((text, cut))
}

/// A primeira url local que a linha traz (`Local: http://localhost:5173/`,
/// `Uvicorn running on http://0.0.0.0:8000`, `(http://[::]:8000/)`). O
/// endereço de todas as interfaces vira `localhost`; o que não é local
/// ([`super::service::is_local_url`]) fica de fora.
pub fn printed_url(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("http").map(|i| i + from) {
        from = at + 4;
        let rest = &lower[at..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            continue;
        }
        let end = line[at..]
            .find(|c: char| c.is_whitespace() || "\"'<>`".contains(c))
            .map_or(line.len(), |i| i + at);
        let mut url = &line[at..end];
        // Pontuação de quem escreveu a frase, não da url.
        loop {
            let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '}']);
            let trimmed = match trimmed.strip_suffix(']') {
                Some(t) if !t.contains('[') => t,
                _ => trimmed,
            };
            if trimmed.len() == url.len() {
                break;
            }
            url = trimmed;
        }
        let url = without_secrets(&any_interface_to_localhost(url));
        if super::service::is_local_url(&url) {
            return Some(url);
        }
    }
    None
}

/// A url impressa vira url do nó (vai ao `url.txt`, ao banco, à ligação e ao
/// `orbit_node_status`): fica só `scheme://host:porta/caminho`, sem query nem
/// fragmento (onde mora o `?token=` do Jupyter e do login), e sem o caminho
/// quando ele carrega um segredo (`/callback/sk-…`).
fn without_secrets(url: &str) -> String {
    let Some(i) = url.find("://") else {
        return url.to_string();
    };
    let rest = &url[i + 3..];
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let path = &rest[authority_end..];
    let keep = if crate::session::redact::contains_secret(path) {
        &rest[..authority_end]
    } else {
        rest
    };
    format!("{}{keep}", &url[..i + 3])
}

fn any_interface_to_localhost(url: &str) -> String {
    let (scheme, rest) = match url.find("://") {
        Some(i) => (&url[..i + 3], &url[i + 3..]),
        None => return url.to_string(),
    };
    for any in ["0.0.0.0", "[::]", "[::0]"] {
        if let Some(after) = rest.strip_prefix(any) {
            if after.is_empty() || after.starts_with([':', '/']) {
                return format!("{scheme}localhost{after}");
            }
        }
    }
    url.to_string()
}

/// A porta da varredura: só a que ficou aberta em duas varreduras seguidas
/// (dá ~1 s para a url impressa chegar antes), e entre elas a que apareceu
/// primeiro — não a menor (o Caddy abre a de admin junto). Empate no mesmo
/// retrato: a menor.
#[derive(Debug, Default)]
pub struct PortPick {
    /// Na ordem em que apareceram.
    order: Vec<u16>,
    previous: Vec<u16>,
}

impl PortPick {
    pub fn observe(&mut self, now: &[u16]) -> Option<u16> {
        let mut fresh: Vec<u16> = now
            .iter()
            .copied()
            .filter(|p| *p > 0 && !self.order.contains(p))
            .collect();
        fresh.sort_unstable();
        fresh.dedup();
        self.order.extend(fresh);
        let pick = self
            .order
            .iter()
            .copied()
            .find(|p| now.contains(p) && self.previous.contains(p));
        self.previous = now.to_vec();
        pick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(chunks: &[&[u8]]) -> Vec<String> {
        let mut s = LineSplitter::default();
        let mut out: Vec<String> = chunks.iter().flat_map(|c| s.feed(c)).collect();
        out.extend(s.finish());
        out
    }

    #[test]
    fn splitter_drops_color_and_osc_even_across_chunks() {
        assert_eq!(
            lines(&[b"\x1b[32mok\x1b[0", b"m feito\r\n", b"\x1b]0;titulo\x07fim"]),
            vec!["ok feito", "fim"]
        );
        // OSC fechado por ESC \ partido entre chunks.
        assert_eq!(lines(&[b"a\x1b]133;A\x1b", b"\\b\n"]), vec!["ab"]);
    }

    #[test]
    fn carriage_return_keeps_the_last_progress_state() {
        assert_eq!(
            lines(&[b"baixando 10%\rbaixando 50%\r", b"baixando 100%\n"]),
            vec!["baixando 100%"]
        );
        // \r\n partido entre chunks é fim de linha, não recomeço.
        assert_eq!(lines(&[b"um\r", b"\ndois\n"]), vec!["um", "dois"]);
    }

    #[test]
    fn writer_redacts_and_strips_before_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        {
            let mut w = LogWriter::create(&path, LOG_MAX_BYTES).unwrap();
            w.feed(format!("\x1b[1mchave {key}\x1b[0m\r\nsem fim").as_bytes());
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&key), "{text}");
        assert!(!text.contains('\x1b'), "{text}");
        assert!(text.starts_with("chave "), "{text}");
        assert!(text.ends_with("sem fim\n"), "o resto sai no fim: {text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn writer_rotates_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        {
            let mut w = LogWriter::create(&path, 100).unwrap();
            for i in 0..30 {
                w.feed(format!("linha {i:02}\n").as_bytes());
            }
        }
        let now = std::fs::metadata(&path).unwrap().len();
        let old = std::fs::metadata(rotated(&path)).unwrap().len();
        assert!(now <= 100 && old <= 100, "{now} {old}");
        let current = std::fs::read_to_string(&path).unwrap();
        assert!(current.ends_with("linha 29\n"), "{current}");
        // O inteiro junta os dois, na ordem.
        let (whole, cut) = read_whole(&path, LOG_VIEW_MAX_BYTES).unwrap();
        assert!(!cut);
        let first = whole.lines().next().unwrap();
        let last = whole.lines().last().unwrap();
        assert!(first < last && last == "linha 29", "{whole}");
        // Com teto menor: só o fim, sem a linha pela metade.
        let (tail, cut) = read_whole(&path, 25).unwrap();
        assert!(cut);
        assert_eq!(tail, "linha 28\nlinha 29\n");
    }

    #[test]
    fn writer_hands_clean_lines_before_redaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        {
            let sink = std::sync::Arc::clone(&seen);
            let mut w = LogWriter::create(&path, LOG_MAX_BYTES)
                .unwrap()
                .on_line(move |l| sink.lock().push(l.to_string()));
            w.feed(b"  \x1b[32m\xe2\x9e\x9c\x1b[39m  Local:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/\x1b[39m\n");
        }
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            printed_url(&seen[0]).as_deref(),
            Some("http://localhost:5173/")
        );
    }

    #[test]
    fn printed_url_finds_local_addresses() {
        let cases = [
            (
                "  Local:   http://localhost:5173/",
                Some("http://localhost:5173/"),
            ),
            (
                "Uvicorn running on http://0.0.0.0:8000 (Press CTRL+C to quit)",
                Some("http://localhost:8000"),
            ),
            (
                "Serving HTTP on :: port 8000 (http://[::]:8000/) ...",
                Some("http://localhost:8000/"),
            ),
            (
                "Listening on http://127.0.0.1:3000.",
                Some("http://127.0.0.1:3000"),
            ),
            (
                "site em https://code.muriki.test",
                Some("https://code.muriki.test"),
            ),
            (
                "abra http://app.localhost:8080/login",
                Some("http://app.localhost:8080/login"),
            ),
            ("ipv6 em http://[::1]:5432", Some("http://[::1]:5432")),
            // Não local primeiro: segue para a próxima da linha.
            (
                "Network: http://192.168.0.5:5173/ Local: http://localhost:5173/",
                Some("http://localhost:5173/"),
            ),
            ("docs em https://vitejs.dev/guide", None),
            ("Network: http://192.168.0.5:5173/", None),
            ("sem url nenhuma, só httpbin", None),
            ("http://user@localhost:3000", None),
        ];
        for (line, want) in cases {
            assert_eq!(printed_url(line).as_deref(), want, "{line}");
        }
    }

    /// A url impressa com token (Jupyter, callback de login) não vira url
    /// do nó com o segredo: sai sem query nem fragmento, e sem o caminho
    /// quando ele é o segredo.
    #[test]
    fn printed_url_drops_query_fragment_and_secret_path() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJlX2Zha2U";
        let key = "sk-abcdefghij0123456789ABCD";
        let cases = [
            (
                format!("Jupyter em http://localhost:8888/lab?token={jwt}"),
                "http://localhost:8888/lab".to_string(),
            ),
            (
                format!("abra http://127.0.0.1:3000/?token={jwt}#x"),
                "http://127.0.0.1:3000/".to_string(),
            ),
            (
                format!("callback: http://localhost:5173/callback/{key}"),
                "http://localhost:5173".to_string(),
            ),
            (
                "http://0.0.0.0:8000/docs#/rotas".to_string(),
                "http://localhost:8000/docs".to_string(),
            ),
        ];
        for (line, want) in cases {
            let got = printed_url(&line).unwrap();
            assert_eq!(got, want, "{line}");
            assert!(!crate::session::redact::contains_secret(&got), "{got}");
        }
    }

    #[test]
    fn port_pick_waits_one_scan_and_keeps_the_first_seen() {
        let mut p = PortPick::default();
        assert_eq!(p.observe(&[]), None);
        assert_eq!(p.observe(&[5173]), None, "uma varredura só ainda espera");
        // A do HMR abre depois: a primeira continua sendo a escolhida.
        assert_eq!(p.observe(&[5173, 24678]), Some(5173));

        // Primeira que aparece ganha, mesmo maior que a de depois.
        let mut p = PortPick::default();
        p.observe(&[8080]);
        assert_eq!(p.observe(&[3000, 8080]), Some(8080));

        // Fechou antes da segunda varredura: não vale.
        let mut p = PortPick::default();
        p.observe(&[4000]);
        assert_eq!(p.observe(&[]), None);
        assert_eq!(p.observe(&[4000]), None);
        assert_eq!(p.observe(&[4000]), Some(4000));

        // Empate no mesmo retrato: a menor.
        let mut p = PortPick::default();
        p.observe(&[2019, 443, 80]);
        assert_eq!(p.observe(&[80, 443, 2019]), Some(80));
    }
}
