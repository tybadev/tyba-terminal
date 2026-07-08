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
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectInfo {
    pub working_dir: String,
    pub config_file: Option<String>,
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

pub fn validate_compose_file(path: &str) -> Result<(), DockerError> {
    if path.contains('\'') {
        return Err(DockerError::InvalidPath(path.to_string()));
    }
    if !(path.ends_with(".yml") || path.ends_with(".yaml")) {
        return Err(DockerError::InvalidPath(path.to_string()));
    }
    if !Path::new(path).is_file() {
        return Err(DockerError::InvalidPath(path.to_string()));
    }
    Ok(())
}

pub fn validate_working_dir(path: &str) -> Result<(), DockerError> {
    if path.contains('\'') {
        return Err(DockerError::InvalidPath(path.to_string()));
    }
    if !Path::new(path).is_dir() {
        return Err(DockerError::InvalidPath(path.to_string()));
    }
    Ok(())
}

pub fn is_valid_container_id(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_labels(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.to_string()))
        })
        .collect()
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
            })
        })
        .collect()
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
            Command::new(c)
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    })
    .as_ref()
}

fn drain(child: &mut Child) -> (std::thread::JoinHandle<Vec<u8>>, std::thread::JoinHandle<Vec<u8>>) {
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

fn run_docker(args: &[&str], timeout: Duration) -> Result<String, DockerError> {
    let bin = docker_bin().ok_or(DockerError::NotInstalled)?;
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
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
    availability: Mutex<Option<(bool, Instant)>>,
    known: Mutex<HashMap<String, String>>,
    projects: Mutex<HashMap<String, ProjectInfo>>,
    tabs: Mutex<HashMap<(String, ContainerTab), SessionId>>,
}

pub type SharedDocker = Arc<DockerManager>;

impl DockerManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn available(&self) -> bool {
        {
            let cache = self.availability.lock();
            if let Some((ok, at)) = *cache {
                if at.elapsed() < AVAILABILITY_TTL {
                    return ok;
                }
            }
        }
        let ok = run_docker(&["version", "--format", "{{.Server.Version}}"], VERSION_TIMEOUT)
            .is_ok();
        *self.availability.lock() = Some((ok, Instant::now()));
        ok
    }

    pub fn list(
        &self,
        repo_root: Option<&str>,
        all: bool,
    ) -> Result<Vec<ContainerInfo>, DockerError> {
        let raw = match run_docker(&["ps", "-a", "--no-trunc", "--format", "{{json .}}"], PS_TIMEOUT)
        {
            Ok(raw) => {
                *self.availability.lock() = Some((true, Instant::now()));
                raw
            }
            Err(e) => {
                *self.availability.lock() = Some((false, Instant::now()));
                return Err(e);
            }
        };
        let containers = parse_ps_output(&raw);
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
                            config_file: c.config_files.clone(),
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

    pub fn remove(&self, id: &str) -> Result<(), DockerError> {
        self.container_name(id)?;
        run_docker(&["rm", "-f", id], RM_TIMEOUT)?;
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

    const FIXTURE: &str = r#"
{"Command":"\"docker-entrypoint.s…\"","CreatedAt":"2026-07-08 10:00:00 -0300 -03","ID":"a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2","Image":"postgres:16","Labels":"com.docker.compose.project=tyba-terminal,com.docker.compose.project.working_dir=/Users/dev/tyba-terminal,com.docker.compose.project.config_files=/Users/dev/tyba-terminal/docker-compose.yml,com.docker.compose.service=db","LocalVolumes":"1","Mounts":"pgdata","Names":"tyba-db-1","Networks":"tyba_default","Ports":"0.0.0.0:5432->5432/tcp","RunningFor":"2 hours ago","Size":"0B","State":"running","Status":"Up 2 hours"}
{"Command":"\"redis-server\"","CreatedAt":"2026-07-08 09:00:00 -0300 -03","ID":"ffffffffffff000000000000000000000000000000000000000000000000ffff","Image":"redis:7","Labels":"","Names":"loose-redis,alias","Ports":"","State":"exited","Status":"Exited (0) 3 hours ago"}
not-json-line
{"ID":"short","Image":"broken","Names":"x","State":"running","Status":"Up"}
"#;

    #[test]
    fn parse_ps_output_reads_valid_lines_and_skips_garbage() {
        let list = parse_ps_output(FIXTURE);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "tyba-db-1");
        assert_eq!(list[0].image, "postgres:16");
        assert_eq!(list[0].state, "running");
        assert_eq!(list[0].ports, "0.0.0.0:5432->5432/tcp");
        assert_eq!(
            list[0].compose_project.as_deref(),
            Some("tyba-terminal")
        );
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
    fn validate_compose_file_rejects_bad_paths() {
        assert!(matches!(
            validate_compose_file("/tmp/it's.yml"),
            Err(DockerError::InvalidPath(_))
        ));
        assert!(matches!(
            validate_compose_file("/tmp/compose.json"),
            Err(DockerError::InvalidPath(_))
        ));
        assert!(matches!(
            validate_compose_file("/nonexistent/compose.yml"),
            Err(DockerError::InvalidPath(_))
        ));
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
                working_dir: "/Users/dev/tyba-terminal".into(),
                config_file: Some("/Users/dev/tyba-terminal/docker-compose.yml".into()),
            },
        );
        let info = mgr.project_info("tyba-terminal").unwrap();
        assert_eq!(info.working_dir, "/Users/dev/tyba-terminal");
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
        let mut list = vec![
            ContainerInfo {
                id: "b".repeat(12),
                name: "zeta".into(),
                image: String::new(),
                state: "exited".into(),
                status: String::new(),
                ports: String::new(),
                compose_project: None,
                compose_working_dir: None,
                service: None,
                config_files: None,
            },
            ContainerInfo {
                id: "c".repeat(12),
                name: "beta".into(),
                image: String::new(),
                state: "running".into(),
                status: String::new(),
                ports: String::new(),
                compose_project: None,
                compose_working_dir: None,
                service: None,
                config_files: None,
            },
            ContainerInfo {
                id: "d".repeat(12),
                name: "alfa".into(),
                image: String::new(),
                state: "running".into(),
                status: String::new(),
                ports: String::new(),
                compose_project: None,
                compose_working_dir: None,
                service: None,
                config_files: None,
            },
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
