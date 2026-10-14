//! Telas cheias de Arquivos e Docker: o que cada uma tem aberto e selecionado.
//!
//! Qual tela está aberta é o `side_view` do workspace (`files:<sessão>`,
//! `docker:<sessão>`), como a de Git. Daqui sai o resto do estado da tela —
//! seção do trilho, arquivos abertos nas abas internas, container escolhido —
//! que também é do core (princípio #1). A tela de Arquivos (abas, ativa,
//! recentes) persiste num setting do Store (ver `LayoutManager::persist`); a
//! de Docker é memória de uso e não persiste.

use serde::{Deserialize, Serialize};

/// Quantos arquivos a seção Recentes lembra.
pub const RECENT_CAP: usize = 30;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesSection {
    #[default]
    Files,
    Search,
    Recent,
}

/// Uma aba interna da tela de Arquivos: `view` é `editor:<sessão>:<rel>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTab {
    pub view: String,
    /// Pré-visualização (itálico): o próximo arquivo aberto do mesmo jeito
    /// toma o lugar dela. Editar ou abrir de propósito fixa.
    pub preview: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesScreen {
    pub section: FilesSection,
    pub tabs: Vec<FileTab>,
    pub active: Option<String>,
    /// Mais recente primeiro, sem repetição.
    pub recent: Vec<String>,
}

impl FilesScreen {
    /// Abre (ou foca) o arquivo. `preview` só vale para arquivo que ainda não
    /// tem aba; abrir de propósito um que está em pré-visualização o fixa.
    pub fn open(&mut self, view: &str, preview: bool) {
        match self.tabs.iter().position(|t| t.view == view) {
            Some(i) => {
                if !preview {
                    self.tabs[i].preview = false;
                }
            }
            None => {
                let tab = FileTab {
                    view: view.to_string(),
                    preview,
                };
                // Só a pré-visualização da mesma sessão: a de outra sessão
                // (outro worktree) não some por causa desta.
                let session = view_session(view);
                let replace = if preview {
                    self.tabs
                        .iter()
                        .position(|t| t.preview && view_session(&t.view) == session)
                } else {
                    None
                };
                match replace {
                    Some(i) => self.tabs[i] = tab,
                    None => {
                        // Ao lado da ativa, como link aberto no navegador.
                        let at = self
                            .active
                            .as_deref()
                            .and_then(|a| self.tabs.iter().position(|t| t.view == a))
                            .map(|p| p + 1)
                            .unwrap_or(self.tabs.len());
                        self.tabs.insert(at, tab);
                    }
                }
            }
        }
        self.active = Some(view.to_string());
        // Recentes são arquivos da sessão: o compose só leitura não entra.
        if !view.starts_with(super::VIEW_COMPOSE_FILE_PREFIX) {
            self.recent.retain(|v| v != view);
            self.recent.insert(0, view.to_string());
            self.recent.truncate(RECENT_CAP);
        }
    }

    /// Foca uma aba que já existe. `false` se não existe.
    pub fn activate(&mut self, view: &str) -> bool {
        if !self.tabs.iter().any(|t| t.view == view) {
            return false;
        }
        self.active = Some(view.to_string());
        true
    }

    /// Tira a aba da pré-visualização (o buffer ficou sujo, duplo clique).
    pub fn pin(&mut self, view: &str) -> bool {
        match self.tabs.iter_mut().find(|t| t.view == view) {
            Some(tab) if tab.preview => {
                tab.preview = false;
                true
            }
            _ => false,
        }
    }

    /// Fecha a aba; sendo a ativa, o foco vai para a vizinha (a da direita, ou
    /// a da esquerda se era a última). `false` se não existia.
    pub fn close(&mut self, view: &str) -> bool {
        let Some(i) = self.tabs.iter().position(|t| t.view == view) else {
            return false;
        };
        self.tabs.remove(i);
        if self.active.as_deref() == Some(view) {
            self.active = self
                .tabs
                .get(i)
                .or_else(|| i.checked_sub(1).and_then(|p| self.tabs.get(p)))
                .map(|t| t.view.clone());
        }
        true
    }

    /// Esquece o que `keep` recusa (a sessão dona morreu).
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = (self.tabs.len(), self.recent.len());
        self.tabs.retain(|t| keep(&t.view));
        self.recent.retain(|v| keep(v));
        if self.active.as_deref().is_some_and(|a| !keep(a)) {
            self.active = self.tabs.first().map(|t| t.view.clone());
        }
        before != (self.tabs.len(), self.recent.len())
    }

    pub fn has(&self, keep: impl Fn(&str) -> bool) -> bool {
        self.tabs.iter().any(|t| keep(&t.view))
    }

    /// Reescreve cada view (sessão reaberta com outro id).
    pub fn map_views(&mut self, f: impl Fn(&str) -> String) {
        for tab in self.tabs.iter_mut() {
            tab.view = f(&tab.view);
        }
        if let Some(active) = self.active.as_mut() {
            *active = f(active);
        }
        for view in self.recent.iter_mut() {
            *view = f(view);
        }
    }

    /// Junta `other` depois do que já está aqui: abas e recentes que faltam
    /// vão para o fim; a ativa daqui prevalece.
    pub fn absorb(&mut self, other: FilesScreen) {
        for tab in other.tabs {
            if !self.tabs.iter().any(|t| t.view == tab.view) {
                self.tabs.push(tab);
            }
        }
        if self.active.is_none() {
            self.active = other.active;
        }
        for view in other.recent {
            if !self.recent.contains(&view) {
                self.recent.push(view);
            }
        }
        self.recent.truncate(RECENT_CAP);
    }
}

/// A sessão de um view `editor:<sessão>:<rel>`: o trecho até o segundo `:`.
fn view_session(view: &str) -> Option<&str> {
    let mut parts = view.splitn(3, ':');
    parts.next()?;
    parts.next()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockerSection {
    #[default]
    Containers,
    Compose,
    Images,
    Volumes,
}

/// O que se vê do container escolhido.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerPane {
    #[default]
    Logs,
    Shell,
    Files,
    Details,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DockerScreen {
    pub section: DockerSection,
    pub container: Option<String>,
    pub pane: ContainerPane,
    /// Projeto compose escolhido na seção Compose.
    pub project: Option<String>,
    /// "Ocultar parados" da seção Compose: some o projeto sem nada rodando.
    pub hide_stopped: bool,
}

impl DockerScreen {
    /// Logs ou Shell de um serviço na seção Compose: pula para o container na
    /// seção Containers (sem abas, a tela é uma só).
    pub fn show_container(&mut self, container: String, pane: ContainerPane) {
        self.section = DockerSection::Containers;
        self.container = Some(container);
        self.pane = pane;
    }

    /// Container com `docker logs -f` justificado por esta tela: escolhido e
    /// na seção Logs. Em qualquer outra, o seguidor pode morrer.
    pub fn following(&self) -> Option<&str> {
        match (self.section, self.pane) {
            (DockerSection::Containers, ContainerPane::Logs) => self.container.as_deref(),
            _ => None,
        }
    }
}

/// O estado das duas telas, por workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Screens {
    pub files: FilesScreen,
    pub docker: DockerScreen,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn views(s: &FilesScreen) -> Vec<(&str, bool)> {
        s.tabs
            .iter()
            .map(|t| (t.view.as_str(), t.preview))
            .collect()
    }

    #[test]
    fn preview_replaces_previous_preview() {
        let mut s = FilesScreen::default();
        s.open("a", true);
        s.open("b", true);
        assert_eq!(views(&s), vec![("b", true)]);
        assert_eq!(s.active.as_deref(), Some("b"));
    }

    #[test]
    fn preview_replaces_only_same_session_preview() {
        let mut s = FilesScreen::default();
        s.open("editor:s1:a.rs", true);
        s.open("editor:s2:b.rs", true);
        assert_eq!(
            views(&s),
            vec![("editor:s1:a.rs", true), ("editor:s2:b.rs", true)]
        );
        s.open("editor:s1:c.rs", true);
        assert_eq!(
            views(&s),
            vec![("editor:s1:c.rs", true), ("editor:s2:b.rs", true)]
        );
    }

    #[test]
    fn pinned_tab_survives_next_preview() {
        let mut s = FilesScreen::default();
        s.open("a", true);
        assert!(s.pin("a"));
        s.open("b", true);
        assert_eq!(views(&s), vec![("a", false), ("b", true)]);
    }

    #[test]
    fn deliberate_open_pins_existing_preview() {
        let mut s = FilesScreen::default();
        s.open("a", true);
        s.open("a", false);
        assert_eq!(views(&s), vec![("a", false)]);
        // E pré-visualizar de novo um fixo não o rebaixa.
        s.open("a", true);
        assert_eq!(views(&s), vec![("a", false)]);
    }

    #[test]
    fn new_tab_opens_next_to_active() {
        let mut s = FilesScreen::default();
        s.open("a", false);
        s.open("b", false);
        s.activate("a");
        s.open("c", false);
        assert_eq!(views(&s), vec![("a", false), ("c", false), ("b", false)]);
    }

    #[test]
    fn closing_active_focuses_neighbor() {
        let mut s = FilesScreen::default();
        for v in ["a", "b", "c"] {
            s.open(v, false);
        }
        s.activate("b");
        assert!(s.close("b"));
        assert_eq!(s.active.as_deref(), Some("c"));
        assert!(s.close("c"));
        assert_eq!(s.active.as_deref(), Some("a"));
        assert!(s.close("a"));
        assert_eq!(s.active, None);
        assert!(!s.close("a"));
    }

    #[test]
    fn closing_inactive_keeps_focus() {
        let mut s = FilesScreen::default();
        s.open("a", false);
        s.open("b", false);
        s.close("a");
        assert_eq!(s.active.as_deref(), Some("b"));
    }

    #[test]
    fn activate_unknown_is_refused() {
        let mut s = FilesScreen::default();
        assert!(!s.activate("x"));
        assert_eq!(s.active, None);
    }

    #[test]
    fn recent_is_most_recent_first_without_duplicates_and_capped() {
        let mut s = FilesScreen::default();
        s.open("a", false);
        s.open("b", false);
        s.open("a", false);
        assert_eq!(s.recent, vec!["a", "b"]);
        for i in 0..(RECENT_CAP + 5) {
            s.open(&format!("f{i}"), true);
        }
        assert_eq!(s.recent.len(), RECENT_CAP);
    }

    #[test]
    fn compose_file_tab_does_not_take_a_recent_slot() {
        let mut s = FilesScreen::default();
        s.open("editor:s1:a.rs", false);
        s.open("compose-file:s1:poc:0:compose.yml", false);
        assert_eq!(s.recent, vec!["editor:s1:a.rs"]);
        assert_eq!(
            s.active.as_deref(),
            Some("compose-file:s1:poc:0:compose.yml")
        );
    }

    #[test]
    fn retain_drops_dead_and_refocuses() {
        let mut s = FilesScreen::default();
        s.open("keep", false);
        s.open("dead", false);
        assert!(s.retain(|v| v != "dead"));
        assert_eq!(views(&s), vec![("keep", false)]);
        assert_eq!(s.active.as_deref(), Some("keep"));
        assert_eq!(s.recent, vec!["keep"]);
        assert!(!s.retain(|_| true));
    }

    #[test]
    fn map_views_rewrites_tabs_active_and_recent() {
        let mut s = FilesScreen::default();
        s.open("old:a", false);
        s.map_views(|v| v.replace("old", "new"));
        assert_eq!(views(&s), vec![("new:a", false)]);
        assert_eq!(s.active.as_deref(), Some("new:a"));
        assert_eq!(s.recent, vec!["new:a"]);
    }

    #[test]
    fn absorb_appends_missing_and_keeps_active() {
        let mut s = FilesScreen::default();
        s.open("a", true);
        let mut other = FilesScreen::default();
        other.open("a", false);
        other.open("b", false);
        s.absorb(other);
        assert_eq!(views(&s), vec![("a", true), ("b", false)]);
        assert_eq!(s.active.as_deref(), Some("a"));
        assert_eq!(s.recent, vec!["a", "b"]);

        let mut empty = FilesScreen::default();
        let mut other = FilesScreen::default();
        other.open("c", false);
        empty.absorb(other);
        assert_eq!(empty.active.as_deref(), Some("c"));
    }

    #[test]
    fn files_screen_round_trips_through_json() {
        let mut s = FilesScreen::default();
        s.open("a", true);
        let back: FilesScreen = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Campo que falta não derruba a carga.
        let partial: FilesScreen = serde_json::from_str("{\"active\":\"x\"}").unwrap();
        assert_eq!(partial.active.as_deref(), Some("x"));
    }

    #[test]
    fn docker_follows_only_selected_logs() {
        let mut d = DockerScreen::default();
        assert_eq!(d.following(), None);
        d.container = Some("c1".into());
        assert_eq!(d.following(), Some("c1"));
        d.pane = ContainerPane::Shell;
        assert_eq!(d.following(), None);
        d.pane = ContainerPane::Logs;
        d.section = DockerSection::Images;
        assert_eq!(d.following(), None);
    }

    #[test]
    fn service_link_jumps_to_container_section() {
        let mut d = DockerScreen {
            section: DockerSection::Compose,
            project: Some("poc".into()),
            ..Default::default()
        };
        d.show_container("c1".into(), ContainerPane::Shell);
        assert_eq!(d.section, DockerSection::Containers);
        assert_eq!(d.container.as_deref(), Some("c1"));
        assert_eq!(d.pane, ContainerPane::Shell);
        // O projeto fica: voltar a Compose cai no mesmo.
        assert_eq!(d.project.as_deref(), Some("poc"));
        d.pane = ContainerPane::Details;
        d.show_container("c2".into(), ContainerPane::Logs);
        assert_eq!(d.following(), Some("c2"));
    }

    #[test]
    fn sections_serialize_lowercase() {
        assert_eq!(
            serde_json::to_string(&FilesSection::Recent).unwrap(),
            "\"recent\""
        );
        assert_eq!(
            serde_json::to_string(&ContainerPane::Details).unwrap(),
            "\"details\""
        );
        assert_eq!(
            serde_json::from_str::<DockerSection>("\"volumes\"").unwrap(),
            DockerSection::Volumes
        );
        assert_eq!(
            serde_json::from_str::<DockerSection>("\"compose\"").unwrap(),
            DockerSection::Compose
        );
    }
}
