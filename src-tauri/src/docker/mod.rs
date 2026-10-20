use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::session::SessionId;

pub mod logs;

const AVAILABILITY_TTL: Duration = Duration::from_secs(10);
/// Vida do nome de container servido à completação.
///
/// Medido nesta máquina: um `docker ps` custa ~45 ms. Sem cache, digitar
/// `docker exec my` pagaria isso por tecla. Cinco segundos é curto o bastante
/// para um container recém-subido aparecer antes de a pessoa terminar de
/// pensar, e longo o bastante para a digitação inteira sair de graça.
const COMPLETION_TTL: Duration = Duration::from_secs(5);
const VERSION_TIMEOUT: Duration = Duration::from_millis(1500);
const PS_TIMEOUT: Duration = Duration::from_secs(3);
const RM_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout local mede docker que responde em milissegundos. Sobre ssh o mesmo
/// comando paga handshake, aprovação do agente de chave e a rede — cabia em 3s
/// nunca, e o painel remoto só entregava timeout (lista vazia). Na primeira
/// conexão o teto é a aprovação humana no 1Password; depois dela o ControlMaster
/// reusa a conexão e as chamadas voltam a ser rápidas.
const REMOTE_FIRST_TIMEOUT: Duration = Duration::from_secs(45);

fn timeout_for(base: Duration, host: Option<&str>) -> Duration {
    match host {
        Some(_) => base.max(REMOTE_FIRST_TIMEOUT),
        None => base,
    }
}

const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";
const COMPOSE_WORKING_DIR_LABEL: &str = "com.docker.compose.project.working_dir";
const COMPOSE_SERVICE_LABEL: &str = "com.docker.compose.service";
const COMPOSE_CONFIG_FILES_LABEL: &str = "com.docker.compose.project.config_files";

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("binário docker não encontrado")]
    NotInstalled,
    #[error("docker não respondeu a tempo")]
    Timeout,
    #[error("docker falhou: {0}")]
    Failed(String),
    #[error("{0}")]
    Ssh(crate::error::AppError),
    #[error("container desconhecido: {0}")]
    UnknownContainer(String),
    #[error("projeto compose desconhecido: {0}")]
    UnknownProject(String),
    #[error("já tem um compose rodando neste projeto: {0}")]
    ComposeBusy(String),
    #[error("caminho inválido: {0}")]
    InvalidPath(String),
    #[error("este caminho não existe nesta máquina: {0} — o container reporta o caminho do ambiente onde subiu, não do host")]
    PathNotOnHost(String),
    #[error("sem permissão de leitura em: {0}")]
    PathDenied(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathStatus {
    Ok,
    Missing,
    Denied,
    Unsupported,
}

impl PathStatus {
    fn rank(self) -> u8 {
        match self {
            PathStatus::Ok => 0,
            PathStatus::Denied => 1,
            PathStatus::Unsupported => 2,
            PathStatus::Missing => 3,
        }
    }

    fn worst(self, other: PathStatus) -> PathStatus {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub compose_project: Option<String>,
    pub compose_working_dir: Option<String>,
    pub service: Option<String>,
    pub config_files: Option<String>,
    pub working_dir_status: Option<PathStatus>,
    pub compose_file_status: Option<PathStatus>,
    /// É do projeto compose do repo pedido (`list` com `repo_root`) — o painel
    /// lateral lista tudo, mas abre esse grupo primeiro. Mesma regra do
    /// `filter_project`, decidida aqui e não no front.
    pub in_repo: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectInfo {
    pub working_dir: String,
    pub config_files: Vec<String>,
}

/// O `--format` do `ps`: só os campos de `PsRow`, nunca `{{json .}}`.
///
/// O `{{json .}}` serializa o contexto inteiro, e isso inclui `.Size`: o CLI
/// vê o campo no template e pede `containers/json?all=1&size=1`, que faz o
/// daemon medir a camada gravável de cada container a cada leitura. Num
/// servidor com containers grandes é o que mais custa na lista remota.
const PS_FORMAT: &str = r#"{"ID":{{json .ID}},"Names":{{json .Names}},"Image":{{json .Image}},"State":{{json .State}},"Status":{{json .Status}},"Ports":{{json .Ports}},"Labels":{{json .Labels}}}"#;

#[derive(Debug, Deserialize)]
struct PsRow {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Names", default)]
    names: String,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Ports", default)]
    ports: String,
    #[serde(rename = "Labels", default)]
    labels: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerTab {
    Logs,
    Shell,
    /// O shell da tela de Docker: mesmo `docker exec`, mas sem aba — mora
    /// dentro da tela e morre quando ela fecha.
    ScreenShell,
    /// Os logs da tela de Docker para container remoto: mesmo `docker logs
    /// -f` da aba, também sem aba e com a vida da tela.
    ScreenLogs,
}

impl ContainerTab {
    /// Mora dentro da tela de Docker (não numa aba).
    pub fn is_screen(self) -> bool {
        matches!(self, ContainerTab::ScreenShell | ContainerTab::ScreenLogs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComposeOp {
    Up,
    Down,
    Restart,
}

impl ComposeOp {
    /// O verbo do compose (depois de `compose -p … -f …`).
    pub fn verb(self) -> &'static [&'static str] {
        match self {
            ComposeOp::Up => &["up", "-d"],
            ComposeOp::Down => &["down"],
            ComposeOp::Restart => &["restart"],
        }
    }
}

/// Arquivo compose maior que isso não abre (não é compose de verdade).
const COMPOSE_FILE_CAP: u64 = 1024 * 1024;

/// Um arquivo compose para a tela de Arquivos (só leitura).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComposeFile {
    pub path: String,
    pub text: String,
}

/// Lê um arquivo regular com teto, sem travar nem estourar: abre sem bloquear
/// (FIFO não prende a thread), confere pelo descritor que é arquivo regular
/// (link para `/dev/zero` não passa) e lê no máximo `cap` bytes.
fn read_regular_capped(path: &Path, cap: u64) -> Result<String, DockerError> {
    use std::io::Read as _;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    let file = options.open(path)?;
    let meta = file.metadata()?;
    let shown = path.to_string_lossy().into_owned();
    if !meta.is_file() || meta.len() > cap {
        return Err(DockerError::InvalidPath(shown));
    }
    let mut buf = Vec::new();
    file.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(DockerError::InvalidPath(shown));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Linhas da saída do compose que a tela guarda por projeto (as últimas).
pub const COMPOSE_OUTPUT_CAP: usize = 2000;

/// Estado de compose de um projeto, do core (a tela some e volta; o estado
/// não): o que está rodando agora, o motivo da última falha e a saída da
/// última execução (já redigida), que fica até a próxima.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ComposeRun {
    /// Identidade da execução (cresce a cada `begin`): a tela não mistura a
    /// saída de uma execução com a da anterior.
    pub run: u64,
    /// Pasta do projeto, para a tela mostrar o projeto mesmo depois que o
    /// `down` tirou os containers dele.
    pub working_dir: Option<String>,
    pub op: Option<ComposeOp>,
    /// A op da última execução (rodando ou terminada), para o título da saída.
    pub last_op: Option<ComposeOp>,
    pub error: Option<String>,
    /// As últimas `COMPOSE_OUTPUT_CAP` linhas.
    pub output: Vec<String>,
    /// Linhas já recebidas nesta execução (inclusive as cortadas do começo):
    /// a tela junta o retrato com os eventos por esta posição.
    pub total: usize,
}

/// Lote de saída que vai para a tela: `start` é a posição (em `total`) da
/// primeira linha.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComposeOutput {
    pub project: String,
    pub run: u64,
    pub op: ComposeOp,
    pub start: usize,
    pub lines: Vec<String>,
}

/// Compose por projeto: um de cada vez.
#[derive(Debug, Default)]
pub struct ComposeRuns {
    runs: HashMap<String, ComposeRun>,
    next: u64,
}

impl ComposeRuns {
    /// Marca `op` rodando; recusa se o projeto já tem um compose em andamento.
    /// Limpa a falha e a saída anteriores.
    pub fn begin(&mut self, project: &str, op: ComposeOp) -> Result<(), DockerError> {
        let run = self.runs.entry(project.to_string()).or_default();
        if run.op.is_some() {
            return Err(DockerError::ComposeBusy(project.to_string()));
        }
        self.next += 1;
        *run = ComposeRun {
            run: self.next,
            working_dir: run.working_dir.take(),
            op: Some(op),
            last_op: Some(op),
            ..Default::default()
        };
        Ok(())
    }

    /// A pasta do projeto, quando o core a resolve.
    pub fn set_dir(&mut self, project: &str, dir: &str) {
        if let Some(run) = self.runs.get_mut(project) {
            run.working_dir = Some(dir.to_string());
        }
    }

    /// Projetos com execução registrada (o retrato de projetos os guarda
    /// mesmo depois do `down`, para subir de novo e abrir o compose).
    pub fn has(&self, project: &str) -> bool {
        self.runs.contains_key(project)
    }

    /// Junta linhas à saída do projeto (cortando o começo além do teto) e
    /// devolve o lote para a tela.
    pub fn append(&mut self, project: &str, lines: Vec<String>) -> Option<ComposeOutput> {
        if lines.is_empty() {
            return None;
        }
        let run = self.runs.get_mut(project)?;
        let op = run.op?;
        let start = run.total;
        run.total += lines.len();
        run.output.extend(lines.iter().cloned());
        let over = run.output.len().saturating_sub(COMPOSE_OUTPUT_CAP);
        run.output.drain(..over);
        Some(ComposeOutput {
            project: project.to_string(),
            run: run.run,
            op,
            start,
            lines,
        })
    }

    /// Desmarca; a saída fica, e a falha fica com o motivo.
    pub fn finish(&mut self, project: &str, result: &Result<(), DockerError>) {
        let run = self.runs.entry(project.to_string()).or_default();
        run.op = None;
        // O motivo vem do stderr: redigido como a saída (princípio #10).
        run.error = result
            .as_ref()
            .err()
            .map(|e| crate::session::redact::redact(&e.to_string()).into_owned());
    }

    pub fn snapshot(&self) -> HashMap<String, ComposeRun> {
        self.runs.clone()
    }
}

/// `docker compose up -d` pode puxar imagem e construir: o teto é longo, mas
/// existe — um compose pendurado não segura a tela para sempre.
const COMPOSE_TIMEOUT: Duration = Duration::from_secs(600);

/// Argumentos de `docker compose` para o projeto: nome e arquivos explícitos,
/// para agir no projeto que subiu (e não no compose padrão da pasta, que pode
/// ser outro). Nome e arquivos já validados por quem chama.
pub fn compose_args(project: &str, files: &[String], op: ComposeOp) -> Vec<String> {
    let mut args = vec![
        "compose".to_string(),
        "--ansi=never".to_string(),
        format!("--project-name={project}"),
    ];
    for file in files {
        args.push(format!("--file={file}"));
    }
    args.extend(op.verb().iter().map(|a| a.to_string()));
    args
}

/// Ação de um container só (o painel lateral Docker). O compose inteiro segue
/// em `ComposeOp`; aqui é o `docker restart|stop <id>` do container clicado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerOp {
    Start,
    Restart,
    Stop,
}

impl ContainerOp {
    pub fn arg(self) -> &'static str {
        match self {
            ContainerOp::Start => "start",
            ContainerOp::Restart => "restart",
            ContainerOp::Stop => "stop",
        }
    }
}

/// `docker stop` espera o container sair (10s de graça por padrão) antes do
/// SIGKILL: o teto tem que cobrir essa espera.
const CONTAINER_OP_TIMEOUT: Duration = Duration::from_secs(20);

fn probe_path(path: &str, want_dir: bool) -> PathStatus {
    if path.contains('\'') {
        return PathStatus::Unsupported;
    }
    if !want_dir && !path.ends_with(".yml") && !path.ends_with(".yaml") {
        return PathStatus::Unsupported;
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() == want_dir => PathStatus::Ok,
        Ok(_) => PathStatus::Missing,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => PathStatus::Denied,
        Err(_) => PathStatus::Missing,
    }
}

pub fn working_dir_status(path: &str) -> PathStatus {
    probe_path(path, true)
}

pub fn compose_file_status(path: &str) -> PathStatus {
    probe_path(path, false)
}

pub fn compose_files_status(paths: &[String]) -> Option<PathStatus> {
    paths
        .iter()
        .map(|p| compose_file_status(p))
        .reduce(PathStatus::worst)
}

fn into_result(path: &str, status: PathStatus) -> Result<(), DockerError> {
    match status {
        PathStatus::Ok => Ok(()),
        PathStatus::Unsupported => Err(DockerError::InvalidPath(path.to_string())),
        PathStatus::Denied => Err(DockerError::PathDenied(path.to_string())),
        PathStatus::Missing => Err(DockerError::PathNotOnHost(path.to_string())),
    }
}

pub fn validate_compose_file(path: &str) -> Result<(), DockerError> {
    into_result(path, compose_file_status(path))
}

pub fn validate_working_dir(path: &str) -> Result<(), DockerError> {
    into_result(path, working_dir_status(path))
}

pub fn resolve_compose_files(working_dir: &str, config_files: &str) -> Vec<String> {
    config_files
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let path = Path::new(entry);
            if path.is_absolute() || working_dir.is_empty() {
                entry.to_string()
            } else {
                Path::new(working_dir)
                    .join(path)
                    .to_string_lossy()
                    .into_owned()
            }
        })
        .collect()
}

/// Nome de projeto compose como o compose aceita (letras, dígitos, `-`, `_`;
/// `.` de versões antigas), começando por letra ou dígito — nunca parece flag.
pub fn is_valid_project_name(name: &str) -> bool {
    name.len() <= 128
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn is_valid_container_id(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_label_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('/')
}

fn parse_labels(raw: &str) -> HashMap<String, String> {
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut last_key: Option<String> = None;
    for entry in raw.split(',') {
        if entry.is_empty() {
            continue;
        }
        match entry.split_once('=') {
            Some((k, v)) if is_label_key(k.trim()) => {
                let key = k.trim().to_string();
                labels.insert(key.clone(), v.to_string());
                last_key = Some(key);
            }
            _ => {
                let continues_list = last_key
                    .as_deref()
                    .is_some_and(|key| key == COMPOSE_CONFIG_FILES_LABEL);
                if !continues_list {
                    continue;
                }
                if let Some(value) = labels.get_mut(COMPOSE_CONFIG_FILES_LABEL) {
                    value.push(',');
                    value.push_str(entry);
                }
            }
        }
    }
    labels
}

pub fn parse_ps_output(raw: &str) -> Vec<ContainerInfo> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let row: PsRow = serde_json::from_str(line).ok()?;
            if !is_valid_container_id(&row.id) {
                return None;
            }
            let labels = parse_labels(&row.labels);
            let name = row
                .names
                .split(',')
                .next()
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string();
            Some(ContainerInfo {
                id: row.id,
                name,
                image: row.image,
                state: row.state.to_lowercase(),
                status: row.status,
                ports: row.ports,
                compose_project: labels.get(COMPOSE_PROJECT_LABEL).cloned(),
                compose_working_dir: labels.get(COMPOSE_WORKING_DIR_LABEL).cloned(),
                service: labels.get(COMPOSE_SERVICE_LABEL).cloned(),
                config_files: labels.get(COMPOSE_CONFIG_FILES_LABEL).cloned(),
                working_dir_status: None,
                compose_file_status: None,
                in_repo: false,
            })
        })
        .collect()
}

/// Imagem do alvo, para a seção Imagens da tela de Docker (só leitura).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    #[serde(rename(deserialize = "ID"), default)]
    pub id: String,
    #[serde(rename(deserialize = "Repository"), default)]
    pub repository: String,
    #[serde(rename(deserialize = "Tag"), default)]
    pub tag: String,
    #[serde(rename(deserialize = "Size"), default)]
    pub size: String,
    #[serde(rename(deserialize = "CreatedSince"), default)]
    pub created_since: String,
}

/// Volume do alvo, para a seção Volumes da tela de Docker (só leitura).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeInfo {
    #[serde(rename(deserialize = "Name"), default)]
    pub name: String,
    #[serde(rename(deserialize = "Driver"), default)]
    pub driver: String,
    #[serde(rename(deserialize = "Mountpoint"), default)]
    pub mountpoint: String,
}

/// Uma linha JSON por item (`--format '{{json .}}'`); linha ilegível some.
fn parse_json_lines<T: serde::de::DeserializeOwned>(raw: &str) -> Vec<T> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn parse_images(raw: &str) -> Vec<ImageInfo> {
    parse_json_lines::<ImageInfo>(raw)
        .into_iter()
        .filter(|i| !i.id.is_empty())
        .collect()
}

pub fn parse_volumes(raw: &str) -> Vec<VolumeInfo> {
    parse_json_lines::<VolumeInfo>(raw)
        .into_iter()
        .filter(|v| !v.name.is_empty())
        .collect()
}

pub fn annotate_host_paths(containers: &mut [ContainerInfo]) {
    let mut dirs: HashMap<String, PathStatus> = HashMap::new();
    let mut files: HashMap<String, Option<PathStatus>> = HashMap::new();
    for container in containers.iter_mut() {
        let Some(working_dir) = container.compose_working_dir.clone() else {
            continue;
        };
        let dir_status = *dirs
            .entry(working_dir.clone())
            .or_insert_with(|| working_dir_status(&working_dir));
        container.working_dir_status = Some(dir_status);

        let Some(raw) = container.config_files.clone() else {
            continue;
        };
        let key = format!("{working_dir}\u{0}{raw}");
        container.compose_file_status = *files
            .entry(key)
            .or_insert_with(|| compose_files_status(&resolve_compose_files(&working_dir, &raw)));
    }
}

/// As raízes que contam como "este repo": a pedida e, se ela for um worktree
/// (sessão de agente), o checkout principal — é de lá que o compose do repo
/// costuma ter subido.
pub fn repo_roots(repo_root: &str) -> Vec<String> {
    let mut roots = vec![repo_root.to_string()];
    let main = crate::repo::git_path(Path::new(repo_root), "--git-common-dir")
        .map(|common| crate::repo::canonicalize_or(&common))
        .filter(|common| common.file_name().is_some_and(|n| n == ".git"))
        .and_then(|common| common.parent().map(Path::to_path_buf));
    if let Some(main) = main {
        let own = crate::repo::canonicalize_or(Path::new(repo_root));
        if main != own {
            roots.push(main.to_string_lossy().into_owned());
        }
    }
    roots
}

/// Marca `in_repo` nos containers do projeto do repo (regra do `filter_project`,
/// em qualquer uma das `roots`).
pub fn mark_repo_project(containers: &mut [ContainerInfo], roots: &[String]) {
    let ours: std::collections::HashSet<String> = roots
        .iter()
        .flat_map(|root| filter_project(containers, root))
        .map(|c| c.id)
        .collect();
    for c in containers.iter_mut() {
        c.in_repo = ours.contains(&c.id);
    }
}

pub fn filter_project(containers: &[ContainerInfo], repo_root: &str) -> Vec<ContainerInfo> {
    let root = repo_root.trim_end_matches('/');
    if root.is_empty() {
        return Vec::new();
    }
    let prefix = format!("{root}/");
    let by_dir: Vec<ContainerInfo> = containers
        .iter()
        .filter(|c| {
            c.compose_working_dir.as_deref().is_some_and(|wd| {
                let wd = wd.trim_end_matches('/');
                wd == root || wd.starts_with(&prefix)
            })
        })
        .cloned()
        .collect();
    if !by_dir.is_empty() {
        return by_dir;
    }
    let Some(base) = Path::new(root)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
    else {
        return Vec::new();
    };
    containers
        .iter()
        .filter(|c| {
            c.compose_project
                .as_deref()
                .is_some_and(|p| p.to_lowercase() == base)
        })
        .cloned()
        .collect()
}

pub fn sort_containers(containers: &mut [ContainerInfo]) {
    containers.sort_by(|a, b| {
        let a_stopped = a.state != "running";
        let b_stopped = b.state != "running";
        a_stopped.cmp(&b_stopped).then_with(|| a.name.cmp(&b.name))
    });
}

pub fn docker_bin() -> Option<&'static PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let candidates = [
            "docker".to_string(),
            "/usr/local/bin/docker".to_string(),
            "/opt/homebrew/bin/docker".to_string(),
            "/usr/bin/docker".to_string(),
            format!("{home}/.docker/bin/docker"),
        ];
        candidates.into_iter().map(PathBuf::from).find(|c| {
            let mut cmd = Command::new(c);
            cmd.arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            crate::repo::no_console_window(&mut cmd);
            cmd.status().is_ok_and(|s| s.success())
        })
    })
    .as_ref()
}

fn drain(
    child: &mut Child,
) -> (
    std::thread::JoinHandle<Vec<u8>>,
    std::thread::JoinHandle<Vec<u8>>,
) {
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    (out, err)
}

/// Alvo dos comandos: a máquina local ou um Host SSH (pelo alias já
/// materializado no ssh_config). O docker fala com máquina remota nativamente
/// por `DOCKER_HOST=ssh://`; o alias resolve porque o TYBA escreve o Include.
pub fn docker_host_env(host: Option<&str>) -> Option<String> {
    host.map(|alias| format!("ssh://{alias}"))
}

fn run_docker(args: &[&str], timeout: Duration, host: Option<&str>) -> Result<String, DockerError> {
    let timeout = timeout_for(timeout, host);
    let bin = docker_bin().ok_or(DockerError::NotInstalled)?;
    let mut cmd = Command::new(bin);
    cmd.args(args);
    // O helper do docker chama o `ssh` com stdin nulo: host de senha só
    // conecta pelo master de uma sessão aberta. Sem master, falha na hora
    // (`ssh.password_needs_session`) em vez de esperar o timeout.
    if let Some(target) = docker_host_env(host) {
        if let Some(alias) = host {
            crate::ssh::command::require_session_if_password(alias).map_err(DockerError::Ssh)?;
        }
        crate::ssh::command::apply_env(&mut cmd);
        cmd.env("DOCKER_HOST", target);
    }
    wait_output(cmd, timeout, |stderr| {
        stderr
            .lines()
            .next()
            .unwrap_or("erro desconhecido")
            .to_string()
    })
}

/// Roda até o fim (ou o teto) com stdout/stderr em pipe. Falha vira
/// `Failed(reason(stderr))`.
fn wait_output(
    mut cmd: Command,
    timeout: Duration,
    reason: impl Fn(&str) -> String,
) -> Result<String, DockerError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::repo::no_console_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let (out, err) = drain(&mut child);
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                let stdout = out.join().unwrap_or_default();
                let stderr = err.join().unwrap_or_default();
                if status.success() {
                    return Ok(String::from_utf8_lossy(&stdout).into_owned());
                }
                return Err(DockerError::Failed(reason(&String::from_utf8_lossy(
                    &stderr,
                ))));
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(DockerError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        }
    }
}

/// Intervalo do lote de saída do compose para a tela (princípio #3: nunca um
/// evento por pedaço).
const COMPOSE_FLUSH: Duration = Duration::from_millis(16);

/// Linhas de um fluxo (stdout ou stderr) em `pending`; `\r` também quebra
/// (barra de progresso reescrita na mesma linha).
fn pump_lines(
    stream: impl Read + Send + 'static,
    pending: Arc<Mutex<Vec<String>>>,
    last: Option<Arc<Mutex<String>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match std::io::BufRead::read_until(&mut reader, b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf);
                    for piece in text.split(['\n', '\r']) {
                        let line = piece.trim_end();
                        if line.is_empty() {
                            continue;
                        }
                        if let Some(last) = &last {
                            *last.lock() = line.to_string();
                        }
                        pending.lock().push(line.to_string());
                    }
                }
            }
        }
    })
}

/// Roda o compose até o fim (ou o teto) mandando a saída em lotes para
/// `on_lines` a cada `COMPOSE_FLUSH`. Falha vira `Failed(última linha do
/// stderr)`; o teto mata o process group inteiro.
/// Um lote da saída do compose, redigido pelo redator da execução.
fn redact_compose_lines(
    redactor: &mut crate::session::redact::LineRedactor,
    lines: &[String],
) -> Vec<String> {
    lines
        .iter()
        .map(|l| redactor.line(l).into_owned())
        .collect()
}

fn stream_compose(
    mut cmd: Command,
    timeout: Duration,
    mut on_lines: impl FnMut(Vec<String>),
) -> Result<(), DockerError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    crate::repo::no_console_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let pending = Arc::new(Mutex::new(Vec::new()));
    let last_err = Arc::new(Mutex::new(String::new()));
    let out = pump_lines(
        child.stdout.take().expect("stdout piped"),
        Arc::clone(&pending),
        None,
    );
    let err = pump_lines(
        child.stderr.take().expect("stderr piped"),
        Arc::clone(&pending),
        Some(Arc::clone(&last_err)),
    );
    let flush = |on_lines: &mut dyn FnMut(Vec<String>)| {
        let batch = std::mem::take(&mut *pending.lock());
        if !batch.is_empty() {
            on_lines(batch);
        }
    };
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            #[cfg(unix)]
            // pgid == pid do líder (`process_group(0)`).
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGKILL)
            };
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            flush(&mut on_lines);
            return Err(DockerError::Timeout);
        }
        flush(&mut on_lines);
        std::thread::sleep(COMPOSE_FLUSH);
    };
    let _ = out.join();
    let _ = err.join();
    flush(&mut on_lines);
    if status.success() {
        return Ok(());
    }
    let reason = crate::session::redact::redact(last_err.lock().trim()).into_owned();
    Err(DockerError::Failed(if reason.is_empty() {
        "erro desconhecido".into()
    } else {
        reason
    }))
}

/// Prepara um `docker …` com o alvo (local ou host SSH) — a mesma montagem do
/// `run_docker`, para os comandos de longa duração que não cabem num timeout.
fn docker_command(args: &[&str], host: Option<&str>) -> Result<Command, DockerError> {
    let bin = docker_bin().ok_or(DockerError::NotInstalled)?;
    let mut cmd = Command::new(bin);
    cmd.args(args);
    if let Some(target) = docker_host_env(host) {
        if let Some(alias) = host {
            crate::ssh::command::require_session_if_password(alias).map_err(DockerError::Ssh)?;
        }
        crate::ssh::command::apply_env(&mut cmd);
        cmd.env("DOCKER_HOST", target);
    }
    crate::repo::no_console_window(&mut cmd);
    Ok(cmd)
}

/// `docker logs -f --timestamps` do container, stdout/stderr em pipe. `since`
/// retoma de onde o seguidor anterior parou (container reiniciado).
pub fn spawn_logs_follow(
    id: &str,
    since: Option<&str>,
    host: Option<&str>,
) -> Result<Child, DockerError> {
    if !is_valid_container_id(id) {
        return Err(DockerError::UnknownContainer(id.chars().take(12).collect()));
    }
    let tail = logs::INITIAL_TAIL.to_string();
    let mut args = vec!["logs", "-f", "--timestamps"];
    match since {
        Some(ts) => args.extend(["--since", ts]),
        None => args.extend(["--tail", tail.as_str()]),
    }
    args.push(id);
    let mut cmd = docker_command(&args, host)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd.spawn()?)
}

const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const INSPECT_TIMEOUT: Duration = Duration::from_secs(3);
const STATS_TIMEOUT: Duration = Duration::from_secs(6);
const FS_TIMEOUT: Duration = Duration::from_secs(5);
/// Teto do leitor de arquivo do container: é uma espiada, não um download.
pub const FS_READ_MAX: usize = 512 * 1024;

/// O que a faixa de contexto da aba de container mostra.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContainerDetail {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub started_at: Option<String>,
    pub workdir: Option<String>,
    pub mounts: Vec<logs::Mount>,
    /// `localhost:3000 → 3000` (fora → dentro), já formatado.
    pub ports: Vec<String>,
}

fn json_str(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(str::to_string)
}

fn format_ports(c: &serde_json::Value) -> Vec<String> {
    let Some(map) = c
        .get("NetworkSettings")
        .and_then(|n| n.get("Ports"))
        .and_then(|p| p.as_object())
    else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for (inner, bindings) in map {
        let port = inner.split('/').next().unwrap_or(inner);
        for bind in bindings.as_array().into_iter().flatten() {
            let Some(host_port) = bind.get("HostPort").and_then(|p| p.as_str()) else {
                continue;
            };
            // Lido como se usa: o endereço de fora primeiro, a porta do
            // container depois ("localhost:15432 → 5432").
            let ip = match bind.get("HostIp").and_then(|i| i.as_str()).unwrap_or("") {
                "" | "0.0.0.0" | "::" | "127.0.0.1" | "::1" => "localhost",
                other => other,
            };
            ports.push(format!("{ip}:{host_port} → {port}"));
        }
    }
    ports.sort();
    ports.dedup();
    ports
}

/// `docker inspect <id>` → o que a faixa e o mapeamento de caminho precisam.
pub fn parse_inspect(raw: &str) -> Option<ContainerDetail> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let c = value.as_array().and_then(|a| a.first()).unwrap_or(&value);
    let id = json_str(c.get("Id"))?;
    let state = c.get("State");
    let config = c.get("Config");
    let mounts = c
        .get("Mounts")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter(|m| m.get("Type").and_then(|t| t.as_str()) == Some("bind"))
        .filter_map(|m| {
            Some(logs::Mount {
                source: json_str(m.get("Source"))?,
                destination: json_str(m.get("Destination"))?,
            })
        })
        .collect();
    Some(ContainerDetail {
        id,
        name: json_str(c.get("Name"))
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_string(),
        image: json_str(config.and_then(|cfg| cfg.get("Image"))).unwrap_or_default(),
        state: json_str(state.and_then(|st| st.get("Status"))).unwrap_or_default(),
        // Container que nunca subiu reporta o "zero" do Go.
        started_at: json_str(state.and_then(|st| st.get("StartedAt")))
            .filter(|t| !t.starts_with("0001-")),
        workdir: json_str(config.and_then(|cfg| cfg.get("WorkingDir"))).filter(|w| !w.is_empty()),
        mounts,
        ports: format_ports(c),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContainerStats {
    pub cpu: String,
    pub mem: String,
}

pub fn parse_stats(raw: &str) -> Option<ContainerStats> {
    let line = raw.lines().find(|l| !l.trim().is_empty())?;
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let cpu = value.get("CPUPerc")?.as_str()?.to_string();
    let mem = value
        .get("MemUsage")?
        .as_str()?
        .split('/')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    Some(ContainerStats { cpu, mem })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerAction {
    Restart,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    /// Bytes, para arquivo comum. `None` em pasta, link e dispositivo.
    pub size: Option<u64>,
    /// Alvo do link simbólico (`ls` não segue o link: `is_dir` fica falso).
    pub link_target: Option<String>,
}

/// Uma linha de `ls -lAn` (GNU ou busybox). `None` para `total N` e para o
/// que não tiver a forma esperada.
fn parse_ls_line(line: &str) -> Option<FsEntry> {
    // Campos separados por um ou mais espaços; o nome é o resto da linha,
    // então se anda campo a campo guardando onde cada um termina.
    let mut rest = line;
    let mut field = || -> Option<&str> {
        let trimmed = rest.trim_start_matches(' ');
        if trimmed.is_empty() {
            return None;
        }
        let end = trimmed.find(' ').unwrap_or(trimmed.len());
        let (head, tail) = trimmed.split_at(end);
        rest = tail;
        Some(head)
    };
    let mode = field()?;
    let kind = mode.chars().next()?;
    if mode.len() < 10 || !"-dlcbps".contains(kind) {
        return None;
    }
    let _links = field()?;
    let _owner = field()?;
    let _group = field()?;
    let mut size_field = field()?;
    if kind == 'c' || kind == 'b' {
        // Dispositivo: `maior, menor` no lugar do tamanho.
        if size_field.ends_with(',') {
            size_field = field()?;
        }
    }
    // Data: `Oct 12 23:50` / `Oct 12  2025` (3 campos) ou `2026-10-12 23:50`
    // quando o ls usa estilo ISO (2 campos).
    let first = field()?;
    let is_iso = first.len() == 10 && first.as_bytes().get(4) == Some(&b'-');
    field()?;
    if !is_iso {
        field()?;
    }
    let name = rest.strip_prefix(' ')?;
    if name.is_empty() {
        return None;
    }
    let (name, link_target) = match (kind, name.split_once(" -> ")) {
        ('l', Some((n, target))) => (n, Some(target.to_string())),
        _ => (name, None),
    };
    Some(FsEntry {
        name: name.to_string(),
        is_dir: kind == 'd',
        size: if kind == '-' {
            size_field.parse().ok()
        } else {
            None
        },
        link_target,
    })
}

/// Saída de `ls -lAn`: pastas primeiro, depois por nome.
pub fn parse_ls(raw: &str) -> Vec<FsEntry> {
    let mut out: Vec<FsEntry> = raw.lines().filter_map(parse_ls_line).collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    out
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FsFile {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}

/// Caminho do container aceito pelo leitor: absoluto, sem NUL nem quebra de
/// linha. Vai como argv (nunca por shell), então aspas não importam.
pub fn validate_container_path(path: &str) -> Result<(), DockerError> {
    if !path.starts_with('/') || path.contains('\0') || path.contains('\n') {
        return Err(DockerError::InvalidPath(path.to_string()));
    }
    Ok(())
}

#[derive(Default)]
pub struct DockerManager {
    /// Por alvo (`None` = local): docker no Mac não diz nada sobre docker na VPS.
    availability: Mutex<HashMap<Option<String>, (bool, Instant)>>,
    known: Mutex<HashMap<String, String>>,
    /// Só para completação. Separado do `known` porque aquele só se atualiza
    /// quando alguém abre o painel de containers, e a completação não pode
    /// depender disso para não servir nome de container que já morreu.
    completion_names: Mutex<Option<(Instant, Vec<String>)>>,
    projects: Mutex<HashMap<String, ProjectInfo>>,
    tabs: Mutex<HashMap<(String, ContainerTab), SessionId>>,
    /// Última leitura de CPU/RAM e motivo de saída, por alvo — o painel pede a
    /// cada refresh, mas o `docker stats` só roda de novo depois do intervalo.
    usage: Mutex<HashMap<Option<String>, (Instant, PanelUsage)>>,
    compose_runs: Mutex<ComposeRuns>,
}

/// CPU e memória de um container rodando (`docker stats --no-stream`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContainerUsage {
    /// Id completo (o `stats` devolve o curto; casado com o snapshot do `list`).
    pub id: String,
    pub cpu_percent: Option<f64>,
    pub mem_bytes: Option<u64>,
}

/// Por que um container parado saiu (`docker inspect`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExitReason {
    pub id: String,
    pub oom_killed: bool,
    pub exit_code: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PanelUsage {
    pub usage: Vec<ContainerUsage>,
    pub exits: Vec<ExitReason>,
}

/// Intervalo mínimo entre duas rodadas de `docker stats` no mesmo alvo.
const USAGE_MIN_INTERVAL: Duration = Duration::from_secs(5);
/// `stats --no-stream` amostra por ~1-2s antes de responder.
const PANEL_STATS_TIMEOUT: Duration = Duration::from_secs(8);

/// "182.3MiB" / "1.2GB" / "512kB" → bytes. Binário (KiB) e decimal (kB).
pub fn parse_size(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    let split = raw
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    let value: f64 = number.parse().ok()?;
    let factor: f64 = match unit.trim() {
        "B" | "" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some((value * factor).round() as u64)
}

/// Uma linha JSON por container de `docker stats --format '{{json .}}'`.
/// Linha que não parseia é ignorada (o docker às vezes emite `--` em
/// container que acabou de parar).
pub fn parse_stats_output(raw: &str) -> Vec<ContainerUsage> {
    #[derive(Deserialize)]
    struct Line {
        // Só `ID`: o `{{json .}}` traz também `Container`, e um alias para ele
        // faria o serde recusar a linha como campo duplicado.
        #[serde(rename = "ID")]
        id: String,
        #[serde(rename = "CPUPerc", default)]
        cpu: String,
        #[serde(rename = "MemUsage", default)]
        mem: String,
    }
    raw.lines()
        .filter_map(|line| serde_json::from_str::<Line>(line.trim()).ok())
        .filter(|l| is_valid_container_id(&l.id))
        .map(|l| ContainerUsage {
            id: l.id,
            cpu_percent: l.cpu.trim().trim_end_matches('%').trim().parse().ok(),
            mem_bytes: l.mem.split('/').next().and_then(parse_size),
        })
        .collect()
}

/// `docker inspect --format '{{.Id}} {{.State.OOMKilled}} {{.State.ExitCode}}'`.
pub fn parse_exit_reasons(raw: &str) -> Vec<ExitReason> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let id = parts.next()?;
            let oom = parts.next()?;
            let code = parts.next().and_then(|c| c.parse().ok());
            is_valid_container_id(id).then(|| ExitReason {
                id: id.to_string(),
                oom_killed: oom == "true",
                exit_code: code,
            })
        })
        .collect()
}

pub type SharedDocker = Arc<DockerManager>;

impl DockerManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nomes de containers **em execução**, para completar argumento.
    ///
    /// Comando mais barato que o `list`: sem `-a`, sem `--no-trunc` e sem JSON —
    /// aqui só o nome importa. Container parado fica de fora de propósito:
    /// `docker exec` num container parado falha, e oferecer o nome seria
    /// sugerir o erro.
    ///
    /// Devolve vazio, nunca erro: a completação não é lugar de reclamar de
    /// daemon parado.
    pub fn completion_names(&self) -> Vec<String> {
        {
            let cache = self.completion_names.lock();
            if let Some((at, names)) = cache.as_ref() {
                if at.elapsed() < COMPLETION_TTL {
                    return names.clone();
                }
            }
        }
        // A disponibilidade tem cache próprio (10 s), então esta guarda quase
        // nunca custa processo — e evita o timeout de 3 s do `ps` com o daemon
        // parado.
        if !self.available(None) {
            return Vec::new();
        }
        let names = run_docker(&["ps", "--format", "{{.Names}}"], PS_TIMEOUT, None)
            .map(|raw| {
                raw.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        *self.completion_names.lock() = Some((Instant::now(), names.clone()));
        names
    }

    pub fn available(&self, host: Option<&str>) -> bool {
        let key = host.map(str::to_string);
        {
            let cache = self.availability.lock();
            if let Some((ok, at)) = cache.get(&key) {
                if at.elapsed() < AVAILABILITY_TTL {
                    return *ok;
                }
            }
        }
        let ok = run_docker(
            &["version", "--format", "{{.Server.Version}}"],
            VERSION_TIMEOUT,
            host,
        )
        .is_ok();
        self.availability.lock().insert(key, (ok, Instant::now()));
        ok
    }

    pub fn list(
        &self,
        repo_root: Option<&str>,
        all: bool,
        host: Option<&str>,
    ) -> Result<Vec<ContainerInfo>, DockerError> {
        let key = host.map(str::to_string);
        let raw = match run_docker(
            &["ps", "-a", "--no-trunc", "--format", PS_FORMAT],
            PS_TIMEOUT,
            host,
        ) {
            Ok(raw) => {
                self.availability.lock().insert(key, (true, Instant::now()));
                raw
            }
            // Falha do `ps` não decide disponibilidade: a tela pergunta o
            // `version` em seguida para separar "sem docker" de erro, e um
            // `false` em cache responderia por ele sem perguntar.
            Err(e) => {
                self.availability.lock().remove(&key);
                return Err(e);
            }
        };
        let mut containers = parse_ps_output(&raw);
        // Caminho do host remoto não existe no Mac: anotar viraria link quebrado.
        if host.is_none() {
            annotate_host_paths(&mut containers);
        }
        {
            let mut known = self.known.lock();
            known.clear();
            for c in &containers {
                known.insert(c.id.clone(), c.name.clone());
            }
        }
        self.prune_tabs();
        // O retrato dos projetos é o que `compose_op` executa: só da máquina
        // local. A lista de um host SSH traz caminhos de lá, que aqui podem
        // existir por acaso e virar `compose down` na pasta errada.
        if host.is_none() {
            let mut projects = self.projects.lock();
            // Projeto que já rodou compose fica mesmo sem container (depois
            // do `down`): dá para subir de novo e o compose.yml continua abrindo.
            let runs = self.compose_runs.lock();
            projects.retain(|name, _| runs.has(name));
            drop(runs);
            for c in &containers {
                if let (Some(project), Some(wd)) =
                    (c.compose_project.as_ref(), c.compose_working_dir.as_ref())
                {
                    projects.insert(
                        project.clone(),
                        ProjectInfo {
                            working_dir: wd.clone(),
                            config_files: c
                                .config_files
                                .as_deref()
                                .map(|raw| resolve_compose_files(wd, raw))
                                .unwrap_or_default(),
                        },
                    );
                }
            }
        }
        if let Some(root) = repo_root {
            mark_repo_project(&mut containers, &repo_roots(root));
        }
        let mut result = match (all, repo_root) {
            (false, Some(_)) => containers.into_iter().filter(|c| c.in_repo).collect(),
            _ => containers,
        };
        sort_containers(&mut result);
        Ok(result)
    }

    pub fn container_name(&self, id: &str) -> Result<String, DockerError> {
        if !is_valid_container_id(id) {
            return Err(DockerError::UnknownContainer(id.chars().take(12).collect()));
        }
        self.known
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| DockerError::UnknownContainer(id.chars().take(12).collect()))
    }

    pub fn remove(&self, id: &str, host: Option<&str>) -> Result<(), DockerError> {
        self.container_name(id)?;
        run_docker(&["rm", "-f", id], RM_TIMEOUT, host)?;
        self.known.lock().remove(id);
        self.prune_tabs();
        Ok(())
    }

    /// CPU/RAM dos que rodam e o motivo de saída dos parados (`exited` = ids
    /// do snapshot). Só o painel Docker aberto chama; o intervalo mínimo por
    /// alvo segura o custo (`docker stats` remoto é ssh + amostragem).
    pub fn panel_usage(
        &self,
        exited: &[String],
        host: Option<&str>,
    ) -> Result<PanelUsage, DockerError> {
        let key = host.map(str::to_string);
        if let Some((at, cached)) = self.usage.lock().get(&key) {
            if at.elapsed() < USAGE_MIN_INTERVAL {
                return Ok(cached.clone());
            }
        }
        let raw = run_docker(
            &["stats", "--no-stream", "--format", "{{json .}}"],
            PANEL_STATS_TIMEOUT,
            host,
        )?;
        let known: Vec<String> = self.known.lock().keys().cloned().collect();
        let usage = parse_stats_output(&raw)
            .into_iter()
            .filter_map(|mut u| {
                let full = known.iter().find(|id| id.starts_with(&u.id))?;
                u.id = full.clone();
                Some(u)
            })
            .collect();
        let ids: Vec<&str> = exited
            .iter()
            .map(String::as_str)
            .filter(|id| self.container_name(id).is_ok())
            .collect();
        let exits = if ids.is_empty() {
            Vec::new()
        } else {
            let mut args = vec![
                "inspect",
                "--format",
                "{{.Id}} {{.State.OOMKilled}} {{.State.ExitCode}}",
            ];
            args.extend(ids);
            // Container removido entre o list e aqui faz o inspect falhar
            // inteiro; o motivo é extra, não vale derrubar CPU/RAM.
            run_docker(&args, PS_TIMEOUT, host)
                .map(|raw| parse_exit_reasons(&raw))
                .unwrap_or_default()
        };
        let result = PanelUsage { usage, exits };
        self.usage
            .lock()
            .insert(key, (Instant::now(), result.clone()));
        Ok(result)
    }

    /// Reinicia ou para um container que o último `list` viu — id fora do
    /// snapshot é recusado antes de virar argumento do `docker`.
    pub fn container_op(
        &self,
        id: &str,
        op: ContainerOp,
        host: Option<&str>,
    ) -> Result<(), DockerError> {
        self.container_name(id)?;
        run_docker(&[op.arg(), id], CONTAINER_OP_TIMEOUT, host)?;
        Ok(())
    }

    fn prune_tabs(&self) {
        let known = self.known.lock();
        self.tabs
            .lock()
            .retain(|(container_id, _), _| known.contains_key(container_id));
    }

    pub fn project_info(&self, name: &str) -> Result<ProjectInfo, DockerError> {
        self.projects
            .lock()
            .get(name)
            .cloned()
            .ok_or_else(|| DockerError::UnknownProject(name.to_string()))
    }

    /// `docker compose up -d|restart|down` de um projeto que o último `list`
    /// local viu, na pasta dele e sem terminal: o resultado volta para a tela.
    /// Só local — o caminho do compose é desta máquina.
    /// Um compose por projeto: o segundo é recusado enquanto o primeiro roda.
    pub fn compose_op(
        &self,
        project: &str,
        op: ComposeOp,
        emit: impl Fn(ComposeOutput),
    ) -> Result<(), DockerError> {
        if !is_valid_project_name(project) {
            return Err(DockerError::UnknownProject(
                project.chars().take(64).collect(),
            ));
        }
        self.compose_runs.lock().begin(project, op)?;
        let result = self.run_compose(project, op, &emit);
        self.compose_runs.lock().finish(project, &result);
        result
    }

    /// Compose rodando, última falha e saída, por projeto (só os que já
    /// rodaram nesta execução do app).
    pub fn compose_runs(&self) -> HashMap<String, ComposeRun> {
        self.compose_runs.lock().snapshot()
    }

    fn run_compose(
        &self,
        project: &str,
        op: ComposeOp,
        emit: &impl Fn(ComposeOutput),
    ) -> Result<(), DockerError> {
        let info = self.project_info(project)?;
        self.compose_runs.lock().set_dir(project, &info.working_dir);
        validate_working_dir(&info.working_dir)?;
        for file in &info.config_files {
            validate_compose_file(file)?;
        }
        let bin = docker_bin().ok_or(DockerError::NotInstalled)?;
        let mut cmd = Command::new(bin);
        cmd.args(compose_args(project, &info.config_files, op))
            .current_dir(&info.working_dir)
            // Credential helper (`docker-credential-desktop`) e plugins moram no
            // PATH do login: sem ele, o pull do `up` falha no app aberto pelo Dock.
            .env("PATH", crate::shell_path::agent_path());
        // Um redator por execução: o corpo de uma chave chega em linhas (e
        // lotes) separados do BEGIN.
        let mut redactor = crate::session::redact::LineRedactor::default();
        stream_compose(cmd, COMPOSE_TIMEOUT, |lines| {
            // Segredo nunca sai do core em claro (princípio #10), nem para a
            // memória da tela.
            let lines = redact_compose_lines(&mut redactor, &lines);
            if let Some(batch) = self.compose_runs.lock().append(project, lines) {
                emit(batch);
            }
        })
    }

    /// Conteúdo de um arquivo compose de um projeto local, só leitura: o
    /// `index` é a posição em `config_files`. Mesmo crivo do compose (arquivo
    /// `.yml/.yaml` que existe) e teto de tamanho.
    pub fn compose_file(&self, project: &str, index: usize) -> Result<ComposeFile, DockerError> {
        if !is_valid_project_name(project) {
            return Err(DockerError::UnknownProject(
                project.chars().take(64).collect(),
            ));
        }
        let info = self.project_info(project)?;
        let path = info
            .config_files
            .get(index)
            .ok_or_else(|| DockerError::UnknownProject(project.to_string()))?
            .clone();
        validate_compose_file(&path)?;
        let text = read_regular_capped(Path::new(&path), COMPOSE_FILE_CAP)?;
        Ok(ComposeFile { path, text })
    }

    /// Nome do container, relistando uma vez se ele ainda não está no retrato
    /// (a aba foi aberta antes do primeiro `ps` desta execução, ou por um
    /// painel que listou outro alvo).
    pub fn ensure_known(&self, id: &str, host: Option<&str>) -> Result<String, DockerError> {
        if let Ok(name) = self.container_name(id) {
            return Ok(name);
        }
        if !is_valid_container_id(id) {
            return Err(DockerError::UnknownContainer(id.chars().take(12).collect()));
        }
        let raw = run_docker(
            &["ps", "-a", "--no-trunc", "--format", PS_FORMAT],
            PS_TIMEOUT,
            host,
        )?;
        {
            let mut known = self.known.lock();
            for c in parse_ps_output(&raw) {
                known.insert(c.id, c.name);
            }
        }
        self.container_name(id)
    }

    pub fn detail(&self, id: &str, host: Option<&str>) -> Result<ContainerDetail, DockerError> {
        self.ensure_known(id, host)?;
        let raw = run_docker(&["inspect", id], INSPECT_TIMEOUT, host)?;
        parse_inspect(&raw).ok_or_else(|| DockerError::Failed("inspect ilegível".into()))
    }

    pub fn stats(&self, id: &str, host: Option<&str>) -> Result<ContainerStats, DockerError> {
        self.ensure_known(id, host)?;
        let raw = run_docker(
            &["stats", "--no-stream", "--format", "{{json .}}", id],
            STATS_TIMEOUT,
            host,
        )?;
        parse_stats(&raw).ok_or_else(|| DockerError::Failed("stats ilegível".into()))
    }

    pub fn action(
        &self,
        id: &str,
        action: ContainerAction,
        host: Option<&str>,
    ) -> Result<(), DockerError> {
        self.ensure_known(id, host)?;
        let verb = match action {
            ContainerAction::Restart => "restart",
            ContainerAction::Stop => "stop",
        };
        run_docker(&[verb, id], ACTION_TIMEOUT, host)?;
        Ok(())
    }

    pub fn fs_list(
        &self,
        id: &str,
        path: &str,
        host: Option<&str>,
    ) -> Result<Vec<FsEntry>, DockerError> {
        self.ensure_known(id, host)?;
        validate_container_path(path)?;
        // Barra no fim: com `-l`, o ls não segue um link simbólico passado na
        // linha de comando — sem ela, uma pasta que é link listaria o link.
        let dir = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };
        let raw = run_docker(
            &["exec", id, "ls", "-lAn", "--", dir.as_str()],
            FS_TIMEOUT,
            host,
        )?;
        Ok(parse_ls(&raw))
    }

    pub fn fs_read(&self, id: &str, path: &str, host: Option<&str>) -> Result<FsFile, DockerError> {
        self.ensure_known(id, host)?;
        validate_container_path(path)?;
        let limit = (FS_READ_MAX + 1).to_string();
        let raw = run_docker(
            &["exec", id, "head", "-c", limit.as_str(), "--", path],
            FS_TIMEOUT,
            host,
        )?;
        let binary = raw.contains('\0');
        let truncated = raw.len() > FS_READ_MAX;
        let mut text = if binary { String::new() } else { raw };
        if truncated && !binary {
            let mut cut = FS_READ_MAX;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        Ok(FsFile {
            text,
            truncated,
            binary,
        })
    }

    /// `docker image ls` do alvo — leitura, nenhuma ação.
    pub fn images(&self, host: Option<&str>) -> Result<Vec<ImageInfo>, DockerError> {
        let raw = run_docker(&["image", "ls", "--format", "{{json .}}"], PS_TIMEOUT, host)?;
        Ok(parse_images(&raw))
    }

    /// `docker volume ls` do alvo — leitura, nenhuma ação.
    pub fn volumes(&self, host: Option<&str>) -> Result<Vec<VolumeInfo>, DockerError> {
        let raw = run_docker(
            &["volume", "ls", "--format", "{{json .}}"],
            PS_TIMEOUT,
            host,
        )?;
        Ok(parse_volumes(&raw))
    }

    /// Sessões (shell e logs) que moram na tela de Docker, fora de qualquer aba.
    pub fn screen_sessions(&self) -> Vec<SessionId> {
        self.tabs
            .lock()
            .iter()
            .filter(|((_, tab), _)| tab.is_screen())
            .map(|(_, s)| *s)
            .collect()
    }

    pub fn forget_screen_sessions(&self) {
        self.tabs.lock().retain(|(_, tab), _| !tab.is_screen());
    }

    pub fn tab_session(&self, id: &str, tab: ContainerTab) -> Option<SessionId> {
        self.tabs.lock().get(&(id.to_string(), tab)).copied()
    }

    pub fn remember_tab(&self, id: &str, tab: ContainerTab, session: SessionId) {
        self.tabs.lock().insert((id.to_string(), tab), session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_runs_refuse_second_op_and_keep_last_error_and_output() {
        let mut runs = ComposeRuns::default();
        assert!(runs.snapshot().is_empty());

        runs.begin("poc", ComposeOp::Up).unwrap();
        assert_eq!(runs.snapshot()["poc"].op, Some(ComposeOp::Up));
        assert!(matches!(
            runs.begin("poc", ComposeOp::Down),
            Err(DockerError::ComposeBusy(_))
        ));
        // Outro projeto não espera.
        runs.begin("api", ComposeOp::Restart).unwrap();

        let first_run = runs.snapshot()["poc"].run;
        let batch = runs
            .append("poc", vec!["Pulling".into(), "Pulled".into()])
            .unwrap();
        assert_eq!(
            (batch.start, batch.run, batch.op),
            (0, first_run, ComposeOp::Up)
        );
        let batch = runs.append("poc", vec!["Started".into()]).unwrap();
        assert_eq!((batch.start, batch.lines.len()), (2, 1));
        assert!(runs.append("poc", Vec::new()).is_none());
        assert!(runs.append("ninguem", vec!["x".into()]).is_none());

        runs.finish("poc", &Err(DockerError::Failed("pull falhou".into())));
        let poc = &runs.snapshot()["poc"];
        assert_eq!(poc.op, None);
        assert_eq!(poc.last_op, Some(ComposeOp::Up));
        assert_eq!(poc.error.as_deref(), Some("docker falhou: pull falhou"));
        // A saída fica depois do fim.
        assert_eq!(poc.output, vec!["Pulling", "Pulled", "Started"]);
        assert_eq!(poc.total, 3);

        // Nova op limpa falha e saída; sucesso mantém a saída.
        runs.set_dir("poc", "/u/poc");
        runs.begin("poc", ComposeOp::Down).unwrap();
        assert_eq!(runs.snapshot()["poc"].error, None);
        assert!(runs.snapshot()["poc"].run > first_run);
        assert_eq!(
            runs.snapshot()["poc"].working_dir.as_deref(),
            Some("/u/poc")
        );
        assert!(runs.has("poc") && !runs.has("ninguem"));
        assert!(runs.snapshot()["poc"].output.is_empty());
        runs.append("poc", vec!["Removed".into()]);
        runs.finish("poc", &Ok(()));
        let poc = &runs.snapshot()["poc"];
        assert_eq!((poc.op, poc.error.as_ref()), (None, None));
        assert_eq!(poc.output, vec!["Removed"]);
    }

    #[test]
    fn compose_failure_reason_is_redacted() {
        let mut runs = ComposeRuns::default();
        runs.begin("poc", ComposeOp::Up).unwrap();
        // Token sintético.
        let secret = "sk-abcdefghijklmnopqrstuvwxyz0123456789";
        runs.finish(
            "poc",
            &Err(DockerError::Failed(format!("auth failed with {secret}"))),
        );
        let error = runs.snapshot()["poc"].error.clone().unwrap();
        assert!(!error.contains(secret), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn compose_file_read_refuses_devices_fifos_and_big_files() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("compose.yml");
        std::fs::write(&ok, "services: {}\n").unwrap();
        assert_eq!(read_regular_capped(&ok, 1024).unwrap(), "services: {}\n");
        assert!(read_regular_capped(&ok, 4).is_err());

        let zero = dir.path().join("zero.yml");
        std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
        assert!(read_regular_capped(&zero, 1024).is_err());

        let fifo = dir.path().join("fifo.yml");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        // Não pode prender: abre sem bloquear e recusa por não ser arquivo.
        assert!(read_regular_capped(&fifo, 1024).is_err());
    }

    #[test]
    fn compose_output_keeps_only_the_last_lines() {
        let mut runs = ComposeRuns::default();
        runs.begin("poc", ComposeOp::Up).unwrap();
        let lines: Vec<String> = (0..COMPOSE_OUTPUT_CAP + 5).map(|i| i.to_string()).collect();
        runs.append("poc", lines);
        let poc = &runs.snapshot()["poc"];
        assert_eq!(poc.output.len(), COMPOSE_OUTPUT_CAP);
        assert_eq!(poc.output[0], "5");
        assert_eq!(poc.total, COMPOSE_OUTPUT_CAP + 5);
    }

    #[cfg(unix)]
    #[test]
    fn compose_output_never_carries_a_pem_body() {
        // Sintética: só tem a forma de uma chave.
        let body = "Zm9vYmFy".repeat(8);
        let script = format!(
            "printf '%s\\n' '-----BEGIN {k}-----' {body} {body} '-----END {k}-----' fim",
            k = "RSA PRIVATE KEY"
        );
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", &script]);
        let mut redactor = crate::session::redact::LineRedactor::default();
        let mut got = Vec::new();
        stream_compose(cmd, Duration::from_secs(10), |lines| {
            got.extend(redact_compose_lines(&mut redactor, &lines))
        })
        .unwrap();
        assert!(got.iter().all(|l| !l.contains(&body)), "{got:?}");
        assert_eq!(got.last().map(String::as_str), Some("fim"));
    }

    #[cfg(unix)]
    #[test]
    fn stream_compose_batches_lines_and_reports_last_stderr() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args([
            "-c",
            "printf 'a\\nb\\r' ; printf 'warn\\nboom\\n' >&2 ; exit 3",
        ]);
        let mut got = Vec::new();
        let result = stream_compose(cmd, Duration::from_secs(10), |lines| got.extend(lines));
        assert!(matches!(result, Err(DockerError::Failed(ref r)) if r == "boom"));
        got.sort();
        assert_eq!(got, vec!["a", "b", "boom", "warn"]);

        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo ok"]);
        assert!(stream_compose(cmd, Duration::from_secs(10), |_| {}).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn stream_compose_timeout_kills_the_group() {
        let mut cmd = Command::new("/bin/sh");
        // O filho em segundo plano mantém o pipe aberto: sem killpg o
        // join dos leitores esperaria os 30s dele.
        cmd.args(["-c", "sleep 30 & sleep 30"]);
        let t = Instant::now();
        let result = stream_compose(cmd, Duration::from_millis(200), |_| {});
        assert!(matches!(result, Err(DockerError::Timeout)));
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn project_names_are_validated() {
        assert!(is_valid_project_name("poc-git"));
        assert!(is_valid_project_name("muriki_api.v2"));
        assert!(!is_valid_project_name(""));
        assert!(!is_valid_project_name("a b"));
        assert!(!is_valid_project_name("../x"));
        assert!(!is_valid_project_name("x;rm"));
        assert!(!is_valid_project_name("-p"));
        assert!(!is_valid_project_name(&"a".repeat(129)));
    }

    #[test]
    fn images_parse_json_lines_and_skip_garbage() {
        let raw = r#"{"ID":"sha256:abc","Repository":"node","Tag":"22-alpine","Size":"180MB","CreatedSince":"2 weeks ago"}
não é json
{"ID":"","Repository":"<none>"}
"#;
        let images = parse_images(raw);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].repository, "node");
        assert_eq!(images[0].tag, "22-alpine");
        assert_eq!(images[0].size, "180MB");
        assert_eq!(images[0].created_since, "2 weeks ago");
    }

    #[test]
    fn volumes_parse_json_lines() {
        let raw = r#"{"Driver":"local","Mountpoint":"/var/lib/docker/volumes/pg/_data","Name":"pg","Scope":"local"}
{"Driver":"local","Name":""}
"#;
        let volumes = parse_volumes(raw);
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].name, "pg");
        assert_eq!(volumes[0].driver, "local");
    }

    #[test]
    fn image_serializes_snake_case_for_webview() {
        let img = ImageInfo {
            id: "x".into(),
            repository: "r".into(),
            tag: "t".into(),
            size: "1MB".into(),
            created_since: "now".into(),
        };
        let json = serde_json::to_string(&img).unwrap();
        assert!(json.contains("\"created_since\":\"now\""), "{json}");
    }

    #[test]
    fn screen_sessions_are_tracked_apart_from_tabs() {
        let mgr = DockerManager::new();
        let shell_tab = SessionId::new_v4();
        let logs_tab = SessionId::new_v4();
        let screen_shell = SessionId::new_v4();
        let screen_logs = SessionId::new_v4();
        mgr.remember_tab("c1", ContainerTab::Shell, shell_tab);
        mgr.remember_tab("c1", ContainerTab::Logs, logs_tab);
        mgr.remember_tab("c1", ContainerTab::ScreenShell, screen_shell);
        mgr.remember_tab("c1", ContainerTab::ScreenLogs, screen_logs);
        let mut screen = mgr.screen_sessions();
        screen.sort();
        let mut expected = vec![screen_shell, screen_logs];
        expected.sort();
        assert_eq!(screen, expected);
        mgr.forget_screen_sessions();
        assert!(mgr.screen_sessions().is_empty());
        assert_eq!(mgr.tab_session("c1", ContainerTab::Shell), Some(shell_tab));
        assert_eq!(mgr.tab_session("c1", ContainerTab::Logs), Some(logs_tab));
    }

    #[test]
    fn local_target_has_no_docker_host() {
        assert_eq!(docker_host_env(None), None);
    }

    #[test]
    fn remoto_ganha_folga_pro_handshake_e_pra_aprovacao_da_chave() {
        assert_eq!(timeout_for(PS_TIMEOUT, None), PS_TIMEOUT);
        assert_eq!(
            timeout_for(PS_TIMEOUT, Some("vps")),
            REMOTE_FIRST_TIMEOUT,
            "3s não cobre handshake + 1Password: o painel remoto só dava timeout"
        );
        let longo = Duration::from_secs(60);
        assert_eq!(timeout_for(longo, Some("vps")), longo, "nunca encurta");
    }

    #[test]
    fn ssh_target_uses_the_alias_materializado() {
        assert_eq!(
            docker_host_env(Some("Hostinger-vps")).as_deref(),
            Some("ssh://Hostinger-vps")
        );
    }

    const FIXTURE: &str = r#"
{"Command":"\"docker-entrypoint.s…\"","CreatedAt":"2026-07-08 10:00:00 -0300 -03","ID":"a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2","Image":"postgres:16","Labels":"com.docker.compose.project=tyba-terminal,com.docker.compose.project.working_dir=/Users/dev/tyba-terminal,com.docker.compose.project.config_files=/Users/dev/tyba-terminal/docker-compose.yml,com.docker.compose.service=db","LocalVolumes":"1","Mounts":"pgdata","Names":"tyba-db-1","Networks":"tyba_default","Ports":"0.0.0.0:5432->5432/tcp","RunningFor":"2 hours ago","Size":"0B","State":"running","Status":"Up 2 hours"}
{"Command":"\"redis-server\"","CreatedAt":"2026-07-08 09:00:00 -0300 -03","ID":"ffffffffffff000000000000000000000000000000000000000000000000ffff","Image":"redis:7","Labels":"","Names":"loose-redis,alias","Ports":"","State":"exited","Status":"Exited (0) 3 hours ago"}
not-json-line
{"ID":"short","Image":"broken","Names":"x","State":"running","Status":"Up"}
"#;

    const LINUX_FIXTURE: &str = r#"
{"CreatedAt":"2026-07-08 10:00:00 -0300 -03","ID":"b1b2c3d4e5f6b1b2c3d4e5f6b1b2c3d4e5f6b1b2c3d4e5f6b1b2c3d4e5f6b1b2","Image":"nginx:1.27","Labels":"com.docker.compose.project=api,com.docker.compose.project.config_files=/home/dev/api/compose.yaml,/home/dev/api/compose.override.yaml,com.docker.compose.project.working_dir=/home/dev/api,com.docker.compose.service=web","Names":"api-web-1","Ports":"","State":"running","Status":"Up 1 hour"}
{"CreatedAt":"2026-07-08 10:00:00 -0300 -03","ID":"c1c2c3d4e5f6c1c2c3d4e5f6c1c2c3d4e5f6c1c2c3d4e5f6c1c2c3d4e5f6c1c2","Image":"redis:7","Labels":"com.docker.compose.project=cache,com.docker.compose.project.config_files=compose.yaml,com.docker.compose.project.working_dir=/home/dev/cache,com.docker.compose.service=redis","Names":"cache-redis-1","Ports":"","State":"running","Status":"Up 1 hour"}
"#;

    fn container(working_dir: Option<&str>, config_files: Option<&str>) -> ContainerInfo {
        ContainerInfo {
            id: "a".repeat(64),
            name: "svc".into(),
            image: String::new(),
            state: "running".into(),
            status: String::new(),
            ports: String::new(),
            compose_project: Some("proj".into()),
            compose_working_dir: working_dir.map(str::to_string),
            service: None,
            config_files: config_files.map(str::to_string),
            working_dir_status: None,
            compose_file_status: None,
            in_repo: false,
        }
    }

    #[test]
    fn parse_ps_output_reads_valid_lines_and_skips_garbage() {
        let list = parse_ps_output(FIXTURE);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "tyba-db-1");
        assert_eq!(list[0].image, "postgres:16");
        assert_eq!(list[0].state, "running");
        assert_eq!(list[0].ports, "0.0.0.0:5432->5432/tcp");
        assert_eq!(list[0].compose_project.as_deref(), Some("tyba-terminal"));
        assert_eq!(
            list[0].compose_working_dir.as_deref(),
            Some("/Users/dev/tyba-terminal")
        );
        assert_eq!(list[0].service.as_deref(), Some("db"));
        assert_eq!(
            list[0].config_files.as_deref(),
            Some("/Users/dev/tyba-terminal/docker-compose.yml")
        );
        assert_eq!(list[1].name, "loose-redis");
        assert_eq!(list[1].compose_project, None);
        assert_eq!(list[1].service, None);
    }

    /// `.Size` no template faz o CLI pedir `size=1`, e o daemon mede o disco de
    /// cada container a cada leitura da lista. Verificado com o docker 29: o
    /// `{{json .}}` pede `containers/json?all=1&size=1`; este formato, só
    /// `all=1`.
    #[test]
    fn ps_format_never_asks_the_daemon_for_sizes() {
        assert!(!PS_FORMAT.contains("Size"), "o template pediria size=1");
        assert!(!PS_FORMAT.contains("{{json .}}"));
        for field in ["ID", "Names", "Image", "State", "Status", "Ports", "Labels"] {
            assert!(
                PS_FORMAT.contains(&format!("\"{field}\":{{{{json .{field}}}}}")),
                "o campo {field} de PsRow tem de vir no formato"
            );
        }
    }

    #[test]
    fn parse_ps_output_reads_the_explicit_format() {
        let line = r#"{"ID":"a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2","Names":"tyba-db-1","Image":"postgres:16","State":"running","Status":"Up 2 hours","Ports":"0.0.0.0:5432->5432/tcp","Labels":"com.docker.compose.project=tyba-terminal,com.docker.compose.service=db"}"#;
        let list = parse_ps_output(line);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "tyba-db-1");
        assert_eq!(list[0].state, "running");
        assert_eq!(list[0].ports, "0.0.0.0:5432->5432/tcp");
        assert_eq!(list[0].compose_project.as_deref(), Some("tyba-terminal"));
        assert_eq!(list[0].service.as_deref(), Some("db"));
    }

    #[test]
    fn compose_op_maps_args_and_labels() {
        assert_eq!(
            compose_args("poc", &[], ComposeOp::Up),
            vec!["compose", "--ansi=never", "--project-name=poc", "up", "-d"]
        );
        assert_eq!(
            compose_args(
                "poc",
                &["/r/compose.yml".into(), "/r/compose.override.yml".into()],
                ComposeOp::Down
            ),
            vec![
                "compose",
                "--ansi=never",
                "--project-name=poc",
                "--file=/r/compose.yml",
                "--file=/r/compose.override.yml",
                "down"
            ]
        );
        assert_eq!(
            compose_args("poc", &[], ComposeOp::Restart)
                .last()
                .map(String::as_str),
            Some("restart")
        );
        assert!(matches!(
            serde_json::from_str::<ComposeOp>("\"down\"").unwrap(),
            ComposeOp::Down
        ));
    }

    #[test]
    fn validate_compose_file_rejects_quotes_and_extensions() {
        assert!(matches!(
            validate_compose_file("/home/dev/it's.yml"),
            Err(DockerError::InvalidPath(_))
        ));
        assert!(matches!(
            validate_compose_file("/home/dev/compose.json"),
            Err(DockerError::InvalidPath(_))
        ));
        assert_eq!(
            compose_file_status("/home/dev/it's/compose.yaml"),
            PathStatus::Unsupported
        );
    }

    #[test]
    fn validate_separates_missing_from_invalid() {
        assert!(matches!(
            validate_compose_file("/home/dev/nowhere/compose.yml"),
            Err(DockerError::PathNotOnHost(_))
        ));
        assert!(matches!(
            validate_working_dir("/home/dev/nowhere"),
            Err(DockerError::PathNotOnHost(_))
        ));
        assert!(matches!(
            validate_compose_file("compose.yaml"),
            Err(DockerError::PathNotOnHost(_))
        ));
    }

    #[test]
    fn validate_accepts_real_paths_and_reports_denied() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        let dir_path = dir.path().to_string_lossy().into_owned();
        let file_path = file.to_string_lossy().into_owned();

        assert_eq!(working_dir_status(&dir_path), PathStatus::Ok);
        assert_eq!(compose_file_status(&file_path), PathStatus::Ok);
        assert!(validate_working_dir(&dir_path).is_ok());
        assert!(validate_compose_file(&file_path).is_ok());
        assert_eq!(working_dir_status(&file_path), PathStatus::Missing);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.path().join("locked");
            std::fs::create_dir(&locked).unwrap();
            let inner = locked.join("compose.yaml");
            std::fs::write(&inner, "services: {}\n").unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            if std::fs::metadata(&inner).is_err() {
                let inner_path = inner.to_string_lossy().into_owned();
                assert_eq!(compose_file_status(&inner_path), PathStatus::Denied);
                assert!(matches!(
                    validate_compose_file(&inner_path),
                    Err(DockerError::PathDenied(_))
                ));
            }
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn resolve_compose_files_expands_relative_and_multi_file_lists() {
        assert_eq!(
            resolve_compose_files("/home/dev/api", "compose.yaml"),
            vec!["/home/dev/api/compose.yaml".to_string()]
        );
        assert_eq!(
            resolve_compose_files(
                "/home/dev/api",
                "/home/dev/api/compose.yaml,compose.override.yaml"
            ),
            vec![
                "/home/dev/api/compose.yaml".to_string(),
                "/home/dev/api/compose.override.yaml".to_string(),
            ]
        );
        assert!(resolve_compose_files("/home/dev/api", "").is_empty());
    }

    #[test]
    fn compose_files_status_reports_the_worst_of_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        let present = file.to_string_lossy().into_owned();
        let absent = dir
            .path()
            .join("compose.override.yaml")
            .to_string_lossy()
            .into_owned();

        assert_eq!(compose_files_status(&[]), None);
        assert_eq!(
            compose_files_status(std::slice::from_ref(&present)),
            Some(PathStatus::Ok)
        );
        assert_eq!(
            compose_files_status(&[present, absent]),
            Some(PathStatus::Missing)
        );
    }

    #[test]
    fn annotate_host_paths_flags_labels_that_are_not_host_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compose.yaml");
        std::fs::write(&file, "services: {}\n").unwrap();
        let real = dir.path().to_string_lossy().into_owned();

        let mut list = vec![
            container(Some(&real), Some("compose.yaml")),
            container(
                Some("/home/dev/only-in-the-container"),
                Some("compose.yaml"),
            ),
            container(None, None),
        ];
        annotate_host_paths(&mut list);

        assert_eq!(list[0].working_dir_status, Some(PathStatus::Ok));
        assert_eq!(list[0].compose_file_status, Some(PathStatus::Ok));
        assert_eq!(list[1].working_dir_status, Some(PathStatus::Missing));
        assert_eq!(list[1].compose_file_status, Some(PathStatus::Missing));
        assert_eq!(list[2].working_dir_status, None);
        assert_eq!(list[2].compose_file_status, None);
    }

    #[test]
    fn parse_ps_output_keeps_every_compose_file_of_the_list() {
        let list = parse_ps_output(LINUX_FIXTURE);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0].config_files.as_deref(),
            Some("/home/dev/api/compose.yaml,/home/dev/api/compose.override.yaml")
        );
        assert_eq!(
            list[0].compose_working_dir.as_deref(),
            Some("/home/dev/api")
        );
        assert_eq!(list[0].service.as_deref(), Some("web"));
        assert_eq!(list[1].config_files.as_deref(), Some("compose.yaml"));
        assert_eq!(
            resolve_compose_files("/home/dev/cache", list[1].config_files.as_deref().unwrap()),
            vec!["/home/dev/cache/compose.yaml".to_string()]
        );
    }

    #[test]
    fn parse_labels_keeps_config_files_list_but_not_stray_fragments() {
        let labels = parse_labels(
            "com.docker.compose.project.config_files=/home/dev/api/compose.yaml,/home/dev/api/compose.override.yaml,com.docker.compose.service=web",
        );
        assert_eq!(
            labels.get(COMPOSE_CONFIG_FILES_LABEL).map(String::as_str),
            Some("/home/dev/api/compose.yaml,/home/dev/api/compose.override.yaml")
        );
        assert_eq!(
            labels.get(COMPOSE_SERVICE_LABEL).map(String::as_str),
            Some("web")
        );

        let with_equals = parse_labels(
            "com.docker.compose.project.config_files=/home/dev/a=b/compose.yaml,/home/dev/a=b/compose.override.yaml",
        );
        assert_eq!(
            with_equals
                .get(COMPOSE_CONFIG_FILES_LABEL)
                .map(String::as_str),
            Some("/home/dev/a=b/compose.yaml,/home/dev/a=b/compose.override.yaml")
        );
    }

    #[test]
    fn project_info_requires_snapshot() {
        let mgr = DockerManager::new();
        assert!(matches!(
            mgr.project_info("tyba-terminal"),
            Err(DockerError::UnknownProject(_))
        ));
        mgr.projects.lock().insert(
            "tyba-terminal".into(),
            ProjectInfo {
                working_dir: "/home/dev/tyba-terminal".into(),
                config_files: vec!["/home/dev/tyba-terminal/docker-compose.yml".into()],
            },
        );
        let info = mgr.project_info("tyba-terminal").unwrap();
        assert_eq!(info.working_dir, "/home/dev/tyba-terminal");
        assert_eq!(
            info.config_files,
            vec!["/home/dev/tyba-terminal/docker-compose.yml".to_string()]
        );
    }

    #[test]
    fn parse_labels_tolerates_empty_and_valueless_entries() {
        let labels = parse_labels("a=1,,b=,=x,c=k=v");
        assert_eq!(labels.get("a").map(String::as_str), Some("1"));
        assert_eq!(labels.get("b").map(String::as_str), Some(""));
        assert_eq!(labels.get("c").map(String::as_str), Some("k=v"));
        assert!(!labels.contains_key(""));
    }

    #[test]
    fn filter_project_matches_working_dir_exact_and_subdir() {
        let list = parse_ps_output(FIXTURE);
        let hit = filter_project(&list, "/Users/dev/tyba-terminal/");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].name, "tyba-db-1");
        let parent = filter_project(&list, "/Users/dev");
        assert_eq!(parent.len(), 1);
        let miss = filter_project(&list, "/Users/dev/tyba-term");
        assert!(miss.is_empty());
    }

    #[test]
    fn filter_project_falls_back_to_project_name() {
        let mut list = parse_ps_output(FIXTURE);
        for c in &mut list {
            c.compose_working_dir = None;
        }
        let hit = filter_project(&list, "/somewhere/else/Tyba-Terminal");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].name, "tyba-db-1");
    }

    #[test]
    fn mark_repo_project_flags_only_the_repo_compose() {
        let mut list = vec![
            ContainerInfo {
                id: "1".repeat(12),
                ..container(Some("/u/repo"), None)
            },
            ContainerInfo {
                id: "2".repeat(12),
                compose_project: Some("outro".into()),
                ..container(Some("/u/outro"), None)
            },
        ];
        mark_repo_project(&mut list, &["/u/repo/".to_string()]);
        assert!(list[0].in_repo);
        assert!(!list[1].in_repo);
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git")
            .status
            .success();
        assert!(ok, "git {args:?} falhou");
    }

    #[test]
    fn worktree_session_marks_the_main_checkout_compose_as_this_repo() {
        let base = tempfile::tempdir().unwrap();
        let main = base.path().join("poc-git");
        std::fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["config", "user.email", "t@t.com"]);
        git(&main, &["config", "user.name", "t"]);
        git(&main, &["config", "commit.gpgsign", "false"]);
        std::fs::write(main.join("compose.yml"), "services: {}\n").unwrap();
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-qm", "init"]);
        let wt = base.path().join("wt-agente-x");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "agente",
                wt.to_str().unwrap(),
            ],
        );
        let main = crate::repo::canonicalize_or(&main);
        let wt = crate::repo::canonicalize_or(&wt);

        // O compose subiu do checkout principal; a sessão está no worktree.
        let main_dir = main.to_string_lossy().into_owned();
        let mut list = vec![
            ContainerInfo {
                id: "1".repeat(12),
                compose_project: Some("poc-git".into()),
                ..container(Some(&main_dir), None)
            },
            ContainerInfo {
                id: "2".repeat(12),
                compose_project: Some("outro".into()),
                ..container(Some("/u/outro"), None)
            },
        ];
        let roots = repo_roots(&wt.to_string_lossy());
        assert_eq!(roots.len(), 2, "{roots:?}");
        mark_repo_project(&mut list, &roots);
        assert!(list[0].in_repo);
        assert!(!list[1].in_repo);

        // Fora de worktree, a raiz é só ela.
        assert_eq!(repo_roots(&main_dir), vec![main_dir.clone()]);
    }

    #[test]
    fn sort_puts_running_first_then_by_name() {
        let named = |name: &str, state: &str| ContainerInfo {
            name: name.into(),
            state: state.into(),
            ..container(None, None)
        };
        let mut list = vec![
            named("zeta", "exited"),
            named("beta", "running"),
            named("alfa", "running"),
        ];
        sort_containers(&mut list);
        let names: Vec<&str> = list.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["alfa", "beta", "zeta"]);
    }

    #[test]
    fn container_id_validation() {
        assert!(is_valid_container_id(&"a".repeat(12)));
        assert!(is_valid_container_id(&"f".repeat(64)));
        assert!(!is_valid_container_id("abc"));
        assert!(!is_valid_container_id(&"g".repeat(12)));
        assert!(!is_valid_container_id(&"a".repeat(65)));
        assert!(!is_valid_container_id("../etc/passwd"));
        assert!(!is_valid_container_id("tyba-db-1; rm -rf /"));
    }

    #[test]
    fn prune_tabs_drops_sessions_of_unknown_containers() {
        let mgr = DockerManager::new();
        let alive = "a".repeat(64);
        let dead = "b".repeat(64);
        mgr.known.lock().insert(alive.clone(), "db".into());
        mgr.remember_tab(&alive, ContainerTab::Logs, SessionId::new_v4());
        mgr.remember_tab(&dead, ContainerTab::Shell, SessionId::new_v4());
        mgr.prune_tabs();
        assert!(mgr.tab_session(&alive, ContainerTab::Logs).is_some());
        assert!(mgr.tab_session(&dead, ContainerTab::Shell).is_none());
    }

    #[test]
    fn manager_rejects_ids_outside_snapshot() {
        let mgr = DockerManager::new();
        let id = "a1b2c3d4e5f6".to_string() + &"0".repeat(52);
        assert!(matches!(
            mgr.container_name(&id),
            Err(DockerError::UnknownContainer(_))
        ));
        mgr.known.lock().insert(id.clone(), "db".into());
        assert_eq!(mgr.container_name(&id).unwrap(), "db");
        assert!(matches!(
            mgr.container_name("èçho"),
            Err(DockerError::UnknownContainer(_))
        ));
    }

    #[test]
    fn parse_size_handles_binary_and_decimal_units() {
        assert_eq!(parse_size("182MiB"), Some(182 * 1024 * 1024));
        assert_eq!(parse_size("1.5GiB"), Some(1_610_612_736));
        assert_eq!(parse_size("512kB"), Some(512_000));
        assert_eq!(parse_size(" 12B "), Some(12));
        assert_eq!(parse_size("2GB"), Some(2_000_000_000));
        assert_eq!(parse_size("--"), None);
        assert_eq!(parse_size("3 parsecs"), None);
    }

    #[test]
    fn parse_stats_output_reads_cpu_and_memory() {
        let raw = concat!(
            r#"{"BlockIO":"0B / 0B","CPUPerc":"4.12%","Container":"a1b2c3d4e5f6","ID":"a1b2c3d4e5f6","MemPerc":"2.31%","MemUsage":"182.3MiB / 7.66GiB","Name":"api","NetIO":"1kB / 0B","PIDs":"11"}"#,
            "\n",
            r#"{"CPUPerc":"--","ID":"bbbbbbbbbbbb","MemUsage":"-- / --","Name":"worker"}"#,
            "\n",
            "lixo\n",
            r#"{"CPUPerc":"1%","ID":"--rm","MemUsage":"1MiB / 1GiB"}"#,
        );
        let out = parse_stats_output(raw);
        assert_eq!(out.len(), 2, "linha inválida e id inválido saem");
        assert_eq!(out[0].id, "a1b2c3d4e5f6");
        assert_eq!(out[0].cpu_percent, Some(4.12));
        assert_eq!(
            out[0].mem_bytes,
            Some((182.3f64 * 1024.0 * 1024.0).round() as u64)
        );
        assert_eq!(out[1].cpu_percent, None, "`--` não vira zero");
        assert_eq!(out[1].mem_bytes, None);
    }

    #[test]
    fn parse_exit_reasons_reads_oom_and_code() {
        let id = "a".repeat(64);
        let other = "b".repeat(64);
        let raw = format!("{id} true 137\n{other} false 1\nnope\n");
        let out = parse_exit_reasons(&raw);
        assert_eq!(
            out,
            vec![
                ExitReason {
                    id,
                    oom_killed: true,
                    exit_code: Some(137)
                },
                ExitReason {
                    id: other,
                    oom_killed: false,
                    exit_code: Some(1)
                },
            ]
        );
    }

    const INSPECT: &str = r#"[{"Id":"a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2","Name":"/api","State":{"Status":"running","StartedAt":"2026-10-12T15:21:00.5Z"},"Config":{"Image":"node:22-alpine","WorkingDir":"/app"},"Mounts":[{"Type":"bind","Source":"/Users/dev/poc-git","Destination":"/app"},{"Type":"volume","Name":"pg","Source":"/var/lib/docker/volumes/pg/_data","Destination":"/data"}],"NetworkSettings":{"Ports":{"3000/tcp":[{"HostIp":"0.0.0.0","HostPort":"3000"},{"HostIp":"::","HostPort":"3000"}],"9229/tcp":null}}}]"#;

    #[test]
    fn parse_inspect_reads_strip_fields_and_bind_mounts_only() {
        let d = parse_inspect(INSPECT).unwrap();
        assert_eq!(d.name, "api");
        assert_eq!(d.image, "node:22-alpine");
        assert_eq!(d.state, "running");
        assert_eq!(d.started_at.as_deref(), Some("2026-10-12T15:21:00.5Z"));
        assert_eq!(d.workdir.as_deref(), Some("/app"));
        assert_eq!(
            d.mounts,
            vec![logs::Mount {
                source: "/Users/dev/poc-git".into(),
                destination: "/app".into()
            }],
            "volume nomeado não é caminho do projeto"
        );
        assert_eq!(d.ports, vec!["localhost:3000 → 3000".to_string()]);
        let never = INSPECT.replace("2026-10-12T15:21:00.5Z", "0001-01-01T00:00:00Z");
        assert_eq!(parse_inspect(&never).unwrap().started_at, None);
        assert!(parse_inspect("lixo").is_none());
    }

    #[test]
    fn parse_stats_keeps_cpu_and_used_memory() {
        let raw = r#"{"BlockIO":"0B / 0B","CPUPerc":"4.12%","MemPerc":"2.3%","MemUsage":"182MiB / 7.66GiB","Name":"api"}"#;
        assert_eq!(
            parse_stats(raw),
            Some(ContainerStats {
                cpu: "4.12%".into(),
                mem: "182MiB".into()
            })
        );
        assert_eq!(parse_stats(""), None);
    }

    #[test]
    fn parse_ls_lists_dirs_first() {
        let raw = "total 12\n\
drwxr-xr-x    2 0        0             4096 Oct 12 23:50 src\n\
-rw-r--r--    1 0        0              812 Oct 12 23:50 package.json\n\
drwxr-xr-x  140 0        0             4096 Oct 12  2025 node_modules\n\
-rw-------    1 1000     1000            33 Oct 12 23:50 .env\n";
        let entries = parse_ls(raw);
        let names: Vec<(&str, bool, Option<u64>)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir, e.size))
            .collect();
        assert_eq!(
            names,
            vec![
                ("node_modules", true, None),
                ("src", true, None),
                (".env", false, Some(33)),
                ("package.json", false, Some(812))
            ]
        );
    }

    #[test]
    fn parse_ls_marks_symlinks_and_keeps_spaces_in_names() {
        let raw = "lrwxrwxrwx 1 0 0 11 Oct 12 23:50 app -> /srv/app v2\n\
-rw-r--r-- 1 0 0 5 Oct 12 23:50 meu arquivo.txt\n";
        let entries = parse_ls(raw);
        assert_eq!(entries[0].name, "app");
        assert_eq!(entries[0].link_target.as_deref(), Some("/srv/app v2"));
        assert_eq!(entries[0].size, None);
        assert!(!entries[0].is_dir);
        assert_eq!(entries[1].name, "meu arquivo.txt");
        assert_eq!(entries[1].size, Some(5));
    }

    #[test]
    fn parse_ls_reads_iso_dates_devices_and_skips_noise() {
        let raw = "total 0\n\
crw-rw-rw- 1 0 0 1, 3 2026-10-12 23:50 null\n\
-rw-r--r--+ 1 0 0 2048 2026-10-12 23:50 notas.md\n\
ls: cannot access 'x': Permission denied\n";
        let entries = parse_ls(raw);
        let names: Vec<(&str, Option<u64>)> =
            entries.iter().map(|e| (e.name.as_str(), e.size)).collect();
        assert_eq!(names, vec![("notas.md", Some(2048)), ("null", None)]);
    }

    #[test]
    fn panel_usage_serves_cache_within_interval() {
        let mgr = DockerManager::new();
        let cached = PanelUsage {
            usage: vec![ContainerUsage {
                id: "x".into(),
                cpu_percent: Some(1.0),
                mem_bytes: None,
            }],
            exits: vec![],
        };
        mgr.usage
            .lock()
            .insert(None, (Instant::now(), cached.clone()));
        // Dentro do intervalo não roda docker nenhum: devolve o que já tinha.
        assert_eq!(mgr.panel_usage(&[], None).unwrap(), cached);
    }

    #[test]
    fn container_op_maps_args_and_rejects_unknown_ids() {
        assert_eq!(ContainerOp::Restart.arg(), "restart");
        assert_eq!(ContainerOp::Stop.arg(), "stop");
        assert_eq!(ContainerOp::Start.arg(), "start");
        let op: ContainerOp = serde_json::from_str("\"stop\"").unwrap();
        assert_eq!(op, ContainerOp::Stop);
        assert!(serde_json::from_str::<ContainerOp>("\"rm\"").is_err());

        let mgr = DockerManager::new();
        let id = "a1b2c3d4e5f6".to_string() + &"0".repeat(52);
        assert!(matches!(
            mgr.container_op(&id, ContainerOp::Stop, None),
            Err(DockerError::UnknownContainer(_))
        ));
        assert!(matches!(
            mgr.container_op("--help", ContainerOp::Restart, None),
            Err(DockerError::UnknownContainer(_))
        ));
    }

    #[test]
    fn container_path_must_be_absolute_and_single_line() {
        assert!(validate_container_path("/app/src").is_ok());
        assert!(validate_container_path("/app/it's ok").is_ok());
        assert!(validate_container_path("app/src").is_err());
        assert!(validate_container_path("/app\n/etc").is_err());
        assert!(validate_container_path("/app\0").is_err());
    }
}
