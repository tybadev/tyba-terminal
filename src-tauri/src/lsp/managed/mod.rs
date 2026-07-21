use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

pub mod consent;
pub mod download;
pub mod pins;
pub mod registry;
pub mod storage;

use crate::lsp::registry::ServerEntry;
use crate::session::store::Store;
use consent::{Consent, Decision};
use registry::{Managed, Platform, Source};

const PROGRESS_FLUSH: Duration = Duration::from_millis(12);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Progress {
    Pending,
    Downloading { downloaded: u64, total: u64 },
    Verifying,
    Extracting,
    Ready,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub server_id: String,
    pub label: String,
    pub version: String,
    pub size: u64,
    pub source: String,
    pub platform: String,
    pub verified: bool,
}

pub enum Outcome {
    NotManaged,
    Installed,
    Installing(Progress),
    Offer(Card),
}

pub struct SpawnPlan {
    pub program: PathBuf,
    pub pre_args: Vec<PathBuf>,
    pub exec_dirs: Vec<PathBuf>,
    pub reads: Vec<PathBuf>,
}

pub type ProgressEmit = dyn Fn(&str, &Progress) + Send + Sync;

pub struct ManagedManager {
    data_dir: PathBuf,
    consent: Consent,
    inflight: Mutex<HashMap<String, Progress>>,
}

pub type SharedManaged = Arc<ManagedManager>;

impl ManagedManager {
    pub fn new(store: Arc<Store>, data_dir: PathBuf) -> Self {
        Self {
            data_dir: data_dir.clone(),
            consent: Consent::new(store),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub fn progress(&self, server_id: &str) -> Option<Progress> {
        self.inflight.lock().get(server_id).cloned()
    }

    pub fn is_consented(&self, server_id: &str) -> bool {
        self.consent.is_granted(server_id)
    }

    fn card(&self, m: &Managed, platform: Platform) -> Card {
        let label = crate::lsp::registry::entry_by_id(m.id)
            .map(|e| e.label.to_string())
            .unwrap_or_else(|| m.id.to_string());
        Card {
            server_id: m.id.to_string(),
            label,
            version: m.version().to_string(),
            size: m.download_size(platform),
            source: m.source_host().to_string(),
            platform: platform.id().to_string(),
            verified: true,
        }
    }

    pub fn card_for(&self, server_id: &str) -> Option<Card> {
        let m = registry::managed_for(server_id)?;
        let plat = Platform::current()?;
        if !m.available_for(plat) {
            return None;
        }
        Some(self.card(m, plat))
    }

    pub fn registry_cards(&self) -> Vec<Card> {
        let Some(plat) = Platform::current() else {
            return Vec::new();
        };
        registry::MANAGED
            .iter()
            .filter(|m| m.available_for(plat))
            .map(|m| self.card(m, plat))
            .collect()
    }

    pub fn outcome(&self, entry: &ServerEntry) -> Outcome {
        let Some(m) = registry::managed_for(entry.id) else {
            return Outcome::NotManaged;
        };
        let Some(plat) = Platform::current() else {
            return Outcome::NotManaged;
        };
        if !m.available_for(plat) {
            return Outcome::NotManaged;
        }
        if storage::is_installed(&self.data_dir, m) {
            return Outcome::Installed;
        }
        if let Some(p) = self.progress(entry.id) {
            if !matches!(p, Progress::Ready) {
                return Outcome::Installing(p);
            }
        }
        if self.consent.is_granted(entry.id) {
            return Outcome::Installing(Progress::Pending);
        }
        Outcome::Offer(self.card(m, plat))
    }

    pub fn plan(&self, entry: &ServerEntry) -> Option<SpawnPlan> {
        let m = registry::managed_for(entry.id)?;
        let plat = Platform::current()?;
        if !storage::is_installed(&self.data_dir, m) {
            return None;
        }
        match &m.source {
            Source::Native { .. } => {
                let pin = m.native_pin(plat)?;
                let bin = storage::native_binary(&self.data_dir, m.id, pin);
                let exec_dirs = bin
                    .parent()
                    .map(|p| vec![p.to_path_buf()])
                    .unwrap_or_default();
                Some(SpawnPlan {
                    program: bin,
                    pre_args: vec![],
                    exec_dirs,
                    reads: vec![storage::version_dir(&self.data_dir, m.id, pin.version)],
                })
            }
            Source::Node { .. } => {
                let node_ver = registry::node_runtime_version();
                let node = storage::node_binary(&self.data_dir, node_ver);
                let entry_js = storage::server_entry_js(&self.data_dir, m)?;
                let node_dir = storage::node_runtime_dir(&self.data_dir, node_ver);
                let install_dir = storage::version_dir(&self.data_dir, m.id, m.version());
                let exec_dirs = node
                    .parent()
                    .map(|p| vec![p.to_path_buf()])
                    .unwrap_or_default();
                Some(SpawnPlan {
                    program: node,
                    pre_args: vec![entry_js],
                    exec_dirs,
                    reads: vec![install_dir, node_dir],
                })
            }
        }
    }

    pub fn record_decision(
        &self,
        server_id: &str,
        decision: Decision,
    ) -> Result<(), String> {
        let version = registry::managed_for(server_id)
            .map(|m| m.version())
            .unwrap_or("");
        self.consent.record(server_id, version, decision)
    }

    fn is_running(&self, server_id: &str) -> bool {
        matches!(
            self.inflight.lock().get(server_id),
            Some(Progress::Pending | Progress::Downloading { .. } | Progress::Verifying | Progress::Extracting)
        )
    }

    pub fn start_download(
        self: &Arc<Self>,
        server_id: &str,
        emit: Arc<ProgressEmit>,
    ) -> Result<(), String> {
        let m = registry::managed_for(server_id).ok_or("server não é gerenciado")?;
        let plat = Platform::current().ok_or("plataforma sem pins gerenciados")?;
        if !m.available_for(plat) {
            return Err("sem pin gerenciado para esta plataforma".into());
        }
        if storage::is_installed(&self.data_dir, m) || self.is_running(server_id) {
            return Ok(());
        }
        self.inflight
            .lock()
            .insert(server_id.to_string(), Progress::Pending);

        let this = Arc::clone(self);
        let id = server_id.to_string();
        std::thread::spawn(move || {
            let outcome = this.run_install(m, plat, &id, &emit);
            if let Err(message) = outcome {
                let p = Progress::Error { message };
                this.inflight.lock().insert(id.clone(), p.clone());
                emit(&id, &p);
            }
        });
        Ok(())
    }

    fn run_install(
        &self,
        m: &Managed,
        plat: Platform,
        id: &str,
        emit: &Arc<ProgressEmit>,
    ) -> Result<(), String> {
        let tmp_root = storage::lsp_dir(&self.data_dir)
            .join(".tmp")
            .join(format!("{id}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_root).map_err(|e| format!("storage: {e}"))?;
        let result = self.download_and_install(m, plat, id, emit, &tmp_root);
        let _ = std::fs::remove_dir_all(&tmp_root);
        result?;
        let p = Progress::Ready;
        self.inflight.lock().insert(id.to_string(), p.clone());
        emit(id, &p);
        Ok(())
    }

    fn download_and_install(
        &self,
        m: &Managed,
        plat: Platform,
        id: &str,
        emit: &Arc<ProgressEmit>,
        tmp_root: &std::path::Path,
    ) -> Result<(), String> {
        let mut jobs: Vec<(String, &registry::Pin)> = Vec::new();
        if m.is_node() {
            let runtime = registry::node_runtime_pin(plat).ok_or("node runtime sem pin")?;
            jobs.push(("__node__".to_string(), runtime));
            for npm in m.npm_packages() {
                jobs.push((npm.package.to_string(), &npm.pin));
            }
        } else {
            let pin = m.native_pin(plat).ok_or("pin nativo ausente")?;
            jobs.push((m.id.to_string(), pin));
        }

        let total: u64 = jobs.iter().map(|(_, p)| p.size).sum();
        let base = Arc::new(Mutex::new(0u64));
        let last = Arc::new(Mutex::new(Instant::now() - PROGRESS_FLUSH));
        let mut verified: Vec<(String, PathBuf)> = Vec::new();

        for (name, pin) in &jobs {
            let dest = tmp_root.join(format!("{name}.artifact"));
            let base_before = *base.lock();
            let base_c = Arc::clone(&base);
            let last_c = Arc::clone(&last);
            let emit_c = Arc::clone(emit);
            let id_c = id.to_string();
            let progress = move |done: u64| {
                let current = base_before + done;
                *base_c.lock() = current;
                let mut last = last_c.lock();
                if last.elapsed() >= PROGRESS_FLUSH || current == total {
                    *last = Instant::now();
                    drop(last);
                    emit_c(
                        &id_c,
                        &Progress::Downloading {
                            downloaded: current,
                            total,
                        },
                    );
                }
            };
            download::fetch_to_file(pin.url, &dest, pin.sha256, pin.size, progress)
                .map_err(|e| e.message())?;
            *base.lock() = base_before + pin.size;
            verified.push((name.clone(), dest));
        }

        let p = Progress::Verifying;
        self.inflight.lock().insert(id.to_string(), p.clone());
        emit(id, &p);

        let p = Progress::Extracting;
        self.inflight.lock().insert(id.to_string(), p.clone());
        emit(id, &p);

        match &m.source {
            Source::Native { .. } => {
                let pin = m.native_pin(plat).ok_or("pin nativo ausente")?;
                let tmp = &verified[0].1;
                storage::install_native(&self.data_dir, m.id, pin, tmp)?;
            }
            Source::Node { .. } => {
                let runtime = registry::node_runtime_pin(plat).ok_or("node runtime sem pin")?;
                let node_tmp = verified
                    .iter()
                    .find(|(n, _)| n == "__node__")
                    .map(|(_, p)| p.clone())
                    .ok_or("runtime node não baixado")?;
                storage::install_node_runtime(&self.data_dir, runtime, &node_tmp)?;
                let pkgs: Vec<(String, PathBuf)> = verified
                    .iter()
                    .filter(|(n, _)| n != "__node__")
                    .cloned()
                    .collect();
                storage::install_node_server(&self.data_dir, m, &pkgs)?;
            }
        }
        Ok(())
    }

    pub fn gc_after_green(&self, server_id: &str) {
        let Some(m) = registry::managed_for(server_id) else {
            return;
        };
        storage::gc_previous_versions(&self.data_dir, m.id, m.version());
        if m.is_node() {
            storage::gc_node_runtime(&self.data_dir, registry::node_runtime_version());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> Arc<ManagedManager> {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        Arc::new(ManagedManager::new(store, tmp.path().to_path_buf()))
    }

    #[test]
    fn path_first_never_manages_a_server_present_on_path() {
        // A resolução gerenciada só é consultada quando registry::discover (PATH +
        // locais canônicos) falha — provado no LspManager. Aqui: um server sem pin
        // gerenciado nunca vira Outcome::Offer.
        let mgr = manager();
        let gopls = crate::lsp::registry::entry_by_id("gopls").unwrap();
        assert!(matches!(mgr.outcome(gopls), Outcome::NotManaged));
    }

    #[test]
    fn a_managed_server_without_consent_offers_a_card() {
        let mgr = manager();
        let ra = crate::lsp::registry::entry_by_id("rust-analyzer").unwrap();
        if Platform::current().is_none() {
            return;
        }
        match mgr.outcome(ra) {
            Outcome::Offer(card) => {
                assert_eq!(card.server_id, "rust-analyzer");
                assert!(card.verified);
                assert!(card.size > 0);
                assert_eq!(card.source, "github.com");
            }
            _ => panic!("sem consent, um server gerenciado tem que oferecer o card"),
        }
    }

    #[test]
    fn consent_flips_the_offer_into_installing() {
        let mgr = manager();
        if Platform::current().is_none() {
            return;
        }
        mgr.record_decision("rust-analyzer", Decision::Accept).unwrap();
        let ra = crate::lsp::registry::entry_by_id("rust-analyzer").unwrap();
        assert!(matches!(mgr.outcome(ra), Outcome::Installing(_)));
    }

    #[test]
    fn refuse_keeps_offering_the_card() {
        let mgr = manager();
        if Platform::current().is_none() {
            return;
        }
        mgr.record_decision("rust-analyzer", Decision::Refuse).unwrap();
        let ra = crate::lsp::registry::entry_by_id("rust-analyzer").unwrap();
        assert!(
            matches!(mgr.outcome(ra), Outcome::Offer(_)),
            "recusa não persiste: o card volta"
        );
    }
}
