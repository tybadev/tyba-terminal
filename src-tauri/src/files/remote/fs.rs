#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    Disconnected,
    NotFound,
    Denied,
    Sftp { code: u32, message: String },
    Protocol(String),
    Exec(String),
}

pub type RemoteResult<T> = Result<T, RemoteError>;

impl RemoteError {
    pub fn message(&self) -> String {
        match self {
            RemoteError::Disconnected => "a conexão SSH caiu".into(),
            RemoteError::NotFound => "item inexistente no host".into(),
            RemoteError::Denied => "permissão negada no host".into(),
            RemoteError::Sftp { message, .. } => format!("falha SFTP: {message}"),
            RemoteError::Protocol(detail) => format!("protocolo SFTP inesperado: {detail}"),
            RemoteError::Exec(detail) => format!("falha ao executar no host: {detail}"),
        }
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl From<RemoteError> for String {
    fn from(e: RemoteError) -> String {
        e.message()
    }
}

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const S_IFREG: u32 = 0o100000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteStat {
    pub size: u64,
    pub mode: u32,
}

impl RemoteStat {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }

    pub fn is_symlink(&self) -> bool {
        self.mode & S_IFMT == S_IFLNK
    }

    pub fn is_file(&self) -> bool {
        // Trata "tipo desconhecido" (mode sem bits de tipo, ex.: servidor que não
        // envia permissões no atributo) como arquivo comum — o caminho de leitura
        // ainda revalida o conteúdo.
        let fmt = self.mode & S_IFMT;
        fmt == S_IFREG || fmt == 0
    }

    pub fn perm(&self) -> u32 {
        self.mode & 0o7777
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDirEntry {
    pub name: String,
    pub stat: RemoteStat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ExecOutput {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Operações remotas cruas sobre a conexão SSH multiplexada da sessão. As de
/// arquivo falam SFTP (binário-safe, sem shell); `exec` roda um comando no host
/// pela mesma conexão (usado só para git e resolução de cwd/home). Toda a lógica
/// de confinamento, guarda de conflito e parsing vive acima desta trait, contra
/// um mock nos testes — o transporte real é validado só no E2E do dono.
pub trait RemoteFs: Send + Sync {
    fn host(&self) -> &str;
    fn realpath(&self, path: &str) -> RemoteResult<String>;
    fn readdir(&self, path: &str) -> RemoteResult<Vec<RemoteDirEntry>>;
    fn lstat(&self, path: &str) -> RemoteResult<RemoteStat>;
    fn stat(&self, path: &str) -> RemoteResult<RemoteStat>;
    fn read_chunk(&self, path: &str, offset: u64, len: usize) -> RemoteResult<Vec<u8>>;
    fn write_new(&self, path: &str, data: &[u8], mode: u32) -> RemoteResult<()>;
    fn posix_rename(&self, from: &str, to: &str) -> RemoteResult<()>;
    fn rename(&self, from: &str, to: &str) -> RemoteResult<()>;
    fn remove(&self, path: &str) -> RemoteResult<()>;
    fn rmdir(&self, path: &str) -> RemoteResult<()>;
    fn mkdir(&self, path: &str, mode: u32) -> RemoteResult<()>;
    fn exec(&self, argv: &[&str]) -> RemoteResult<ExecOutput>;
}

pub fn mode_dir() -> u32 {
    S_IFDIR | 0o755
}

pub fn mode_file(perm: u32) -> u32 {
    S_IFREG | (perm & 0o7777)
}

pub fn mode_symlink() -> u32 {
    S_IFLNK | 0o777
}
