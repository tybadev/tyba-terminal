use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::session::SessionId;

const AVAILABILITY_TTL: Duration = Duration::from_secs(10);
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
    #[error("container desconhecido: {0}")]
    UnknownContainer(String),
    #[error("projeto compose desconhecido: {0}")]
    UnknownProject(String),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectInfo {
    pub working_dir: String,
    pub config_files: Vec<String>,
}

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComposeOp {
    Up,
    Down,
    Restart,
}

impl ComposeOp {
    pub fn label(self) -> &'static str {
        match self {
            ComposeOp::Up => "up",
            ComposeOp::Down => "down",
            ComposeOp::Restart => "restart",
        }
    }

    pub fn compose_args(self) -> &'static str {
        match self {
            ComposeOp::Up => "compose up -d",
            ComposeOp::Down => "compose down",
            ComposeOp::Restart => "compose restart",
        }
    }
}

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
            })
        })
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
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Limite conhecido do docker-over-ssh: o helper chama o `ssh` com stdin
    // nulo, então host que só aceita senha não conecta — precisa de chave. Não
    // dá pra forçar BatchMode sem mexer no ssh_config do usuário (o que
    // quebraria a sessão interativa dele), então o caso cai no timeout.
    if let Some(target) = docker_host_env(host) {
        cmd.env("DOCKER_HOST", target);
    }
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
                let reason = String::from_utf8_lossy(&stderr);
                let reason = reason.lines().next().unwrap_or("erro desconhecido");
                return Err(DockerError::Failed(reason.to_string()));
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

#[derive(Default)]
pub struct DockerManager {
    /// Por alvo (`None` = local): docker no Mac não diz nada sobre docker na VPS.
    availability: Mutex<HashMap<Option<String>, (bool, Instant)>>,
    known: Mutex<HashMap<String, String>>,
    projects: Mutex<HashMap<String, ProjectInfo>>,
    tabs: Mutex<HashMap<(String, ContainerTab), SessionId>>,
}

pub type SharedDocker = Arc<DockerManager>;

impl DockerManager {
    pub fn new() -> Self {
        Self::default()
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
            &["ps", "-a", "--no-trunc", "--format", "{{json .}}"],
            PS_TIMEOUT,
            host,
        ) {
            Ok(raw) => {
                self.availability.lock().insert(key, (true, Instant::now()));
                raw
            }
            Err(e) => {
                self.availability
                    .lock()
                    .insert(key, (false, Instant::now()));
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
        {
            let mut projects = self.projects.lock();
            projects.clear();
            for c in &containers {
                if let (Some(project), Some(wd)) =
                    (c.compose_project.as_ref(), c.compose_working_dir.as_ref())
                {
                    projects
                        .entry(project.clone())
                        .or_insert_with(|| ProjectInfo {
                            working_dir: wd.clone(),
                            config_files: c
                                .config_files
                                .as_deref()
                                .map(|raw| resolve_compose_files(wd, raw))
                                .unwrap_or_default(),
                        });
                }
            }
        }
        let mut result = match (all, repo_root) {
            (false, Some(root)) => filter_project(&containers, root),
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

    #[test]
    fn compose_op_maps_args_and_labels() {
        assert_eq!(ComposeOp::Up.compose_args(), "compose up -d");
        assert_eq!(ComposeOp::Down.compose_args(), "compose down");
        assert_eq!(ComposeOp::Restart.compose_args(), "compose restart");
        assert_eq!(ComposeOp::Down.label(), "down");
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
}
