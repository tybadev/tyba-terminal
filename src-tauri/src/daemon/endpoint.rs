//! Quem pode mexer no nome do socket do daemon — a lição do Orca
//! (`src/main/daemon/AGENTS.md`, 23 defeitos em 7 rodadas, todos a mesma
//! intercalação: alguém vê "morto" em T e mexe no nome em T+1).
//!
//! - Só um daemon se publicando mexe no nome, e só trocando uma entrada que
//!   ele mesmo acabou de provar morta. Ninguém apaga nome que não criou.
//! - "Não sei" nunca vira "morto": só conectar prova ocupado; só recusa ou
//!   ausência provam morte. Timeout, permissão ou tipo estranho = desistir.
//! - `link` antes, nunca um `rename` incondicional (que destruiria um daemon
//!   vivo); `rename`, nunca `unlink` + `link` (que deixa o nome sumido entre
//!   duas chamadas).
//! - Uma trava (`flock`) presa pela vida inteira do daemon decide quem está
//!   vivo; a sonda só confirma. No macOS, fila de `accept` cheia responde
//!   `ECONNREFUSED` igual a um socket morto.
//! - Sem varredor de sobras, e o daemon que sai NÃO remove o nome: o próximo
//!   troca a entrada morta num `rename` só. (O `Drop` dos sockets da CLI e
//!   do canal remove o arquivo; aqui não.)

use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Quanto esperar o `connect` da sonda antes de dizer "não sei".
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// Voltas quando o nome muda de mão no meio da troca.
const ATTEMPTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Alguém aceitou a conexão: há um daemon servindo.
    Serving,
    /// Recusa ou ausência: ninguém escuta ali.
    Dead,
    /// Timeout, permissão, tipo de arquivo estranho: não prova nada.
    Unknown,
}

#[derive(Debug)]
pub enum PublishError {
    /// Outro daemon já serve no nome; quem chamou deve usar aquele.
    AlreadyServing,
    /// Não deu para provar que o dono anterior morreu: não mexe.
    Undecided(String),
    Io(std::io::Error),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishError::AlreadyServing => write!(f, "outro daemon já serve neste socket"),
            PublishError::Undecided(why) => write!(f, "socket do daemon em estado incerto: {why}"),
            PublishError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for PublishError {
    fn from(e: std::io::Error) -> Self {
        PublishError::Io(e)
    }
}

/// Pergunta se algo serve em `path`. Só olha; nunca mexe no nome.
pub fn probe(path: &Path) -> Probe {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Probe::Dead,
        Err(_) => return Probe::Unknown,
        Ok(meta) if !meta.file_type().is_socket() => return Probe::Unknown,
        Ok(_) => {}
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let target = path.to_path_buf();
    // `connect` de socket unix é imediato no caso normal, mas fila cheia
    // bloqueia: a thread deixa a espera ter teto.
    let spawned = std::thread::Builder::new()
        .name("tyba-daemon-probe".into())
        .spawn(move || {
            let _ = tx.send(UnixStream::connect(&target));
        });
    if spawned.is_err() {
        return Probe::Unknown;
    }
    match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(Ok(_)) => Probe::Serving,
        Ok(Err(e)) => match e.raw_os_error() {
            Some(libc::ECONNREFUSED) | Some(libc::ENOENT) => Probe::Dead,
            _ => Probe::Unknown,
        },
        Err(_) => Probe::Unknown,
    }
}

fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

fn private_name(dir: &Path) -> PathBuf {
    // Prefixo `.p` de propósito (o Orca aprendeu que `.b<hex>` cai no
    // varredor de versões antigas dele; aqui não há varredor, mas o nome
    // privado continua fora de qualquer padrão do nome público).
    dir.join(format!(
        ".p{}",
        &uuid::Uuid::new_v4().simple().to_string()[..16]
    ))
}

/// O daemon publicado: o listener e a trava que diz "estou vivo". A trava
/// (`flock` num arquivo ao lado do socket) fica presa enquanto o processo
/// existir — inclusive parado ou travado — e o kernel solta quando ele morre.
/// É ela, e não a sonda, que decide se há outro daemon: no macOS, um socket
/// vivo com a fila de `accept` cheia responde `ECONNREFUSED` como um morto.
pub struct Published {
    pub listener: UnixListener,
    _lock: std::fs::File,
}

/// Liga o socket do daemon em `canonical`, pelo protocolo de posse.
/// A pasta é criada privada (0700) e conferida; o socket fica 0600.
pub fn publish(canonical: &Path) -> Result<Published, PublishError> {
    let dir = canonical
        .parent()
        .ok_or_else(|| PublishError::Undecided("socket sem pasta".into()))?;
    crate::session::create_private_dir(dir)?;
    crate::session::verify_private_dir(dir)?;
    let lock = hold_lock(&canonical.with_extension("lock"))?;

    let scratch = private_name(dir);
    let listener = UnixListener::bind(&scratch)?;
    // Daqui em diante, todo caminho que não publica apaga o nome privado:
    // é nosso, e só o nosso a gente apaga.
    let result = (|| {
        std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o600))?;
        let ours = identity(&std::fs::symlink_metadata(&scratch)?);
        take_name(&scratch, canonical, ours)
    })();
    match result {
        Ok(()) => Ok(Published {
            listener,
            _lock: lock,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&scratch);
            Err(e)
        }
    }
}

/// Trava exclusiva sem esperar. Presa por outro = outro daemon vive.
fn hold_lock(path: &Path) -> Result<std::fs::File, PublishError> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(file);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) => Err(PublishError::AlreadyServing),
        _ => Err(err.into()),
    }
}

fn take_name(scratch: &Path, canonical: &Path, ours: (u64, u64)) -> Result<(), PublishError> {
    for _ in 0..ATTEMPTS {
        match std::fs::hard_link(scratch, canonical) {
            Ok(()) => {
                // O nome público aponta para o nosso socket; o privado sai.
                let _ = std::fs::remove_file(scratch);
                return verify_kept(canonical, ours);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let before = match std::fs::symlink_metadata(canonical) {
            Ok(meta) => identity(&meta),
            // Sumiu entre o `link` e aqui: tenta o `link` de novo.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        match probe(canonical) {
            Probe::Serving => return Err(PublishError::AlreadyServing),
            Probe::Unknown => {
                return Err(PublishError::Undecided(
                    "não deu para provar que o anterior morreu".into(),
                ))
            }
            Probe::Dead => {}
        }
        // A entrada que provamos morta ainda é a mesma?
        match std::fs::symlink_metadata(canonical) {
            Ok(meta) if identity(&meta) == before => {}
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        }
        // Mais uma sonda logo antes da troca: encurta a janela entre a prova
        // e o `rename` (POSIX não tem "rename se o alvo ainda for o inode X").
        match probe(canonical) {
            Probe::Dead => {}
            Probe::Serving => return Err(PublishError::AlreadyServing),
            Probe::Unknown => {
                return Err(PublishError::Undecided(
                    "o anterior mudou de estado na troca".into(),
                ))
            }
        }
        std::fs::rename(scratch, canonical)?;
        return verify_kept(canonical, ours);
    }
    Err(PublishError::Undecided(
        "o socket mudou de mão várias vezes durante a troca".into(),
    ))
}

fn verify_kept(canonical: &Path, ours: (u64, u64)) -> Result<(), PublishError> {
    match std::fs::symlink_metadata(canonical) {
        Ok(meta) if identity(&meta) == ours => Ok(()),
        // Outro publicou por cima: não é nosso, não mexe. Quem chamou larga o
        // listener e não serve ninguém por um nome que não tem.
        Ok(_) => Err(PublishError::AlreadyServing),
        Err(e) => Err(e.into()),
    }
}
