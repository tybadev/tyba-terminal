//! O produtor do palpite de tela: quem avalia manifesto e chama `set_observed`.
//!
//! Um observador por sessão, vivo na thread do PTY daquela sessão. Recebe o
//! recorte de tela que o [`crate::pty`] tirou **sob** o lock, e faz todo o resto
//! **fora** dele — a justificativa de onde isso roda está no `pty/mod.rs`, junto
//! do laço.
//!
//! Três portões, nesta ordem, do mais barato para o mais caro:
//!
//! 1. **Escopo**: sessão de agente lançada pelo TYBA nem ganha observador. Onde
//!    há hook, a tela não opina — e `set_observed` recusaria de novo, mas a
//!    recusa dele é a segunda linha, não a primeira.
//! 2. **Sequência**: o `fingerprint` do recorte não mudou desde a última
//!    avaliação, então nada que o manifesto olha mudou, e a avaliação inteira é
//!    pulada. É o que mantém regra de terceiro fora da esmagadora maioria dos
//!    flushes — output rolando embaixo de um título estável não reavalia nada.
//! 3. **Identidade**: só o manifesto que reconhece a sessão avalia estado.

use std::sync::Arc;

use crate::session::{ObservedAgent, SessionKind};
use crate::status::manifest::{Scope, Verdict};
use crate::status::registry::ManifestRegistry;
use crate::status::screen::ScreenSnapshot;

/// Nome do binário do agente rodando na sessão, quando há árvore de processos
/// para perguntar. Consultado **depois** do portão de sequência: é uma leitura
/// de estado do prober, não a varredura em si, mas nem essa se paga a cada
/// chunk.
pub type ProcessProbe = Box<dyn Fn() -> Option<String> + Send>;

/// Para onde o palpite vai. Fechado sobre a sessão e o `AppHandle` porque o
/// observador não conhece nenhum dos dois — ele produz um `ObservedAgent`, não
/// um evento.
pub type ObservedSink = Box<dyn Fn(Option<ObservedAgent>) + Send>;

pub struct ScreenObserver {
    registry: Arc<ManifestRegistry>,
    scope: Scope,
    /// `None` fora do shell local: em SSH e em container não há árvore de
    /// processos que o TYBA alcance, e ali identidade sai só do título.
    process: Option<ProcessProbe>,
    sink: ObservedSink,
    /// O portão de sequência. `None` antes da primeira avaliação.
    last_fingerprint: Option<u64>,
    /// O último palpite publicado. Guardado para dois motivos: não repetir IPC
    /// idêntico, e sobreviver a `Hold`/`NoMatch` — que mandam **não mexer** no
    /// estado, não limpá-lo.
    last: Option<ObservedAgent>,
}

impl ScreenObserver {
    /// `None` quando esta sessão não pode receber palpite de tela.
    ///
    /// Hoje isso é só a sessão de agente lançada pelo TYBA: ali existe hook, e
    /// hook é autoridade. As duas fontes não corromperiam nada (são campos
    /// diferentes), mas dariam à UI duas respostas para a mesma pergunta — e a
    /// de tela é a pior das duas.
    pub fn for_session(
        kind: &SessionKind,
        registry: Arc<ManifestRegistry>,
        process: ProcessProbe,
        sink: ObservedSink,
    ) -> Option<Self> {
        let scope = scope_of(kind)?;
        Some(Self {
            registry,
            scope,
            process: matches!(scope, Scope::Shell).then_some(process),
            sink,
            last_fingerprint: None,
            last: None,
        })
    }

    /// Vale a pena recortar a tela desta sessão? Consultado **dentro** do lock,
    /// então é só um booleano.
    pub fn wants_snapshot(&self) -> bool {
        !self.registry.is_empty()
    }

    /// Um recorte de tela. Fora do lock.
    pub fn observe(&mut self, snapshot: &ScreenSnapshot) {
        let fingerprint = snapshot.fingerprint();
        if self.last_fingerprint == Some(fingerprint) {
            return;
        }
        self.last_fingerprint = Some(fingerprint);

        // Clone do `Arc` para o empréstimo do manifesto não brigar com o
        // `publish`, que é `&mut self`.
        let registry = Arc::clone(&self.registry);
        let process = self.process.as_ref().and_then(|probe| probe());
        let Some(manifest) = registry.identify(self.scope, process.as_deref(), &snapshot.title)
        else {
            // Ninguém reconhece o que está na tela. Identidade é o que sustenta
            // a presença: sem ela não há agente a apontar, e manter o último
            // palpite deixaria um agente morto no quadro para sempre.
            self.publish(None);
            return;
        };
        let agent = manifest.id.clone();
        let state = match manifest.evaluate(snapshot) {
            Verdict::State(state) => Some(state),
            // `Hold` é "casou, e por isso NÃO mexa no estado"; `NoMatch` é
            // "nenhuma regra falou". Os dois preservam o que já havia — limpar
            // aqui faria o estado piscar a cada tela que o manifesto não
            // descreve. O estado só é herdado do MESMO agente: trocar de agente
            // e levar o estado junto seria afirmar o que ninguém observou.
            Verdict::Hold | Verdict::NoMatch => self
                .last
                .as_ref()
                .filter(|observed| observed.agent == agent)
                .and_then(|observed| observed.state),
        };
        self.publish(Some(ObservedAgent { agent, state }));
    }

    /// O PTY morreu: o que estava na tela deixou de ser notícia do presente.
    pub fn clear(&mut self) {
        self.last_fingerprint = None;
        self.publish(None);
    }

    fn publish(&mut self, observed: Option<ObservedAgent>) {
        if self.last == observed {
            return;
        }
        self.last.clone_from(&observed);
        (self.sink)(observed);
    }
}

/// Onde a sessão está, na linguagem do manifesto.
///
/// `None` é a recusa: sessão de agente do TYBA não recebe palpite.
fn scope_of(kind: &SessionKind) -> Option<Scope> {
    match kind {
        SessionKind::Shell => Some(Scope::Shell),
        SessionKind::Ssh { .. } => Some(Scope::Ssh),
        SessionKind::Container { .. } => Some(Scope::Container),
        SessionKind::Agent { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::session::{AgentRunnerKind, ObservedState};

    const CODEX: &str = r#"
id = "codex"
match = { process = ["codex"], title = ["Codex"] }
applies_to = ["shell", "ssh"]

[[rules]]
id = "title_blocked"
state = "awaiting_input"
priority = 1100
region = "osc_title"
contains = ["Action Required"]

[[rules]]
id = "transcript_viewer"
priority = 1000
region = { bottom_lines = 3 }
contains = ["q to quit"]
skip_state_update = true

[[rules]]
id = "screen_working"
state = "running"
priority = 500
region = { bottom_lines = 3 }
contains = ["esc to interrupt"]
"#;

    #[derive(Default)]
    struct Espiao {
        probes: AtomicUsize,
        publicados: parking_lot::Mutex<Vec<Option<ObservedAgent>>>,
    }

    impl Espiao {
        fn publicados(&self) -> Vec<Option<ObservedAgent>> {
            self.publicados.lock().clone()
        }

        fn ultimo(&self) -> Option<ObservedAgent> {
            self.publicados.lock().last().cloned().flatten()
        }
    }

    fn registry() -> Arc<ManifestRegistry> {
        Arc::new(ManifestRegistry::from_sources(&[CODEX]))
    }

    fn observer(kind: &SessionKind, espiao: &Arc<Espiao>) -> Option<ScreenObserver> {
        let probe_spy = Arc::clone(espiao);
        let sink_spy = Arc::clone(espiao);
        ScreenObserver::for_session(
            kind,
            registry(),
            Box::new(move || {
                probe_spy.probes.fetch_add(1, Ordering::Relaxed);
                Some("codex".to_string())
            }),
            Box::new(move |observed| sink_spy.publicados.lock().push(observed)),
        )
    }

    fn snap(title: &str, lines: &[&str]) -> ScreenSnapshot {
        ScreenSnapshot {
            title: title.into(),
            alt_screen: false,
            bottom_lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    /// O portão de sequência não é decoração: com a tela parada, a regra de
    /// terceiro não roda.
    ///
    /// O contador está na sonda de processo, que é o primeiro passo DEPOIS do
    /// portão e ANTES da avaliação — se ela não foi chamada, o manifesto não
    /// foi avaliado.
    #[test]
    fn o_portao_de_sequencia_pula_a_avaliacao_quando_nada_muda() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();
        let tela = snap("Codex — Action Required", &[]);

        for _ in 0..5 {
            observer.observe(&tela);
        }

        assert_eq!(
            espiao.probes.load(Ordering::Relaxed),
            1,
            "a tela não mudou e o manifesto foi avaliado assim mesmo"
        );
        assert_eq!(espiao.publicados().len(), 1);
    }

    #[test]
    fn tela_que_muda_reavalia() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        observer.observe(&snap("Codex — Action Required", &[]));

        assert_eq!(espiao.probes.load(Ordering::Relaxed), 2);
        assert_eq!(
            espiao.ultimo().unwrap().state,
            Some(ObservedState::AwaitingInput)
        );
    }

    /// `Hold` é "não mexa no estado", não "limpe".
    #[test]
    fn hold_nao_apaga_o_estado_anterior() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        assert_eq!(espiao.ultimo().unwrap().state, Some(ObservedState::Running));

        // O visualizador de transcript: tela reconhecível que explicitamente
        // não diz nada sobre estado.
        observer.observe(&snap("Codex", &["↑/↓ to scroll", "q to quit"]));

        assert_eq!(
            espiao.ultimo().unwrap().state,
            Some(ObservedState::Running),
            "o Hold apagou o estado que ele mandava preservar"
        );
        assert_eq!(
            espiao.publicados().len(),
            1,
            "palpite idêntico foi republicado"
        );
    }

    /// O mesmo vale para `NoMatch` num agente que já foi identificado: a tela
    /// que o manifesto não descreve não é notícia de estado nenhum.
    #[test]
    fn no_match_num_agente_identificado_mantem_o_estado() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        observer.observe(&snap("Codex", &["nada de especial na tela"]));

        assert_eq!(espiao.ultimo().unwrap().state, Some(ObservedState::Running));
    }

    /// Estado herdado vale para o MESMO agente, e só.
    ///
    /// A tela troca de dono — o usuário sai do Codex e abre o Gemini, ou dois
    /// manifestos reconhecem telas parecidas —, e carregar o `running` do
    /// anterior afirmaria sobre o novo algo que ninguém observou.
    #[test]
    fn o_estado_nao_atravessa_de_um_agente_para_outro() {
        const GEMINI: &str = r#"
id = "gemini"
match = { title = ["Gemini"] }

[[rules]]
id = "sem_estado_para_esta_tela"
region = { bottom_lines = 3 }
contains = ["Ready"]
skip_state_update = true
"#;
        let espiao = Arc::new(Espiao::default());
        let sink_spy = Arc::clone(&espiao);
        let mut observer = ScreenObserver::for_session(
            &SessionKind::Shell,
            Arc::new(ManifestRegistry::from_sources(&[CODEX, GEMINI])),
            Box::new(|| None),
            Box::new(move |observed| sink_spy.publicados.lock().push(observed)),
        )
        .unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        assert_eq!(espiao.ultimo().unwrap().state, Some(ObservedState::Running));

        // Hold do Gemini: reconhece a tela e não diz nada sobre estado.
        observer.observe(&snap("Gemini", &["Ready"]));

        let ultimo = espiao.ultimo().unwrap();
        assert_eq!(ultimo.agent, "gemini");
        assert_eq!(
            ultimo.state, None,
            "o estado do agente anterior atravessou para o novo"
        );
    }

    /// Identidade perdida é presença perdida — o contrário deixaria um agente
    /// morto no quadro para sempre.
    #[test]
    fn identidade_perdida_limpa_o_palpite() {
        let espiao = Arc::new(Espiao::default());
        let sink_spy = Arc::clone(&espiao);
        let mut observer = ScreenObserver::for_session(
            &SessionKind::Ssh {
                host_id: "h".into(),
            },
            registry(),
            Box::new(|| None),
            Box::new(move |observed| sink_spy.publicados.lock().push(observed)),
        )
        .unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        assert!(espiao.ultimo().is_some());

        observer.observe(&snap("guilherme@mac: ~", &["$ "]));

        assert_eq!(espiao.publicados().last().unwrap(), &None);
    }

    /// A prova de que o caminho novo não alcança sessão com hook. Sem a recusa
    /// aqui, o palpite chegaria até `set_observed` para ser recusado lá — e o
    /// recorte de tela seria pago a cada flush por nada.
    #[test]
    fn sessao_de_agente_gerenciada_nunca_ganha_observador() {
        let espiao = Arc::new(Espiao::default());

        assert!(observer(
            &SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode,
            },
            &espiao
        )
        .is_none());
        assert!(observer(
            &SessionKind::Agent {
                runner: AgentRunnerKind::Custom("meu-agente".into()),
            },
            &espiao
        )
        .is_none());

        // E o resto continua ganhando.
        assert!(observer(&SessionKind::Shell, &espiao).is_some());
        assert!(observer(
            &SessionKind::Ssh {
                host_id: "h".into()
            },
            &espiao
        )
        .is_some());
        assert!(observer(
            &SessionKind::Container {
                host_id: None,
                container_id: "c".into(),
            },
            &espiao
        )
        .is_some());
    }

    /// Fora do shell local não há árvore de processos: perguntar ali seria
    /// responder sobre a máquina errada.
    #[test]
    fn so_o_shell_local_consulta_processo() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(
            &SessionKind::Ssh {
                host_id: "h".into(),
            },
            &espiao,
        )
        .unwrap();

        observer.observe(&snap("Codex — Action Required", &[]));

        assert_eq!(espiao.probes.load(Ordering::Relaxed), 0);
        assert_eq!(
            espiao.ultimo().unwrap().state,
            Some(ObservedState::AwaitingInput),
            "sem processo, o título ainda tinha que identificar"
        );
    }

    #[test]
    fn a_morte_do_pty_leva_o_palpite_junto() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        observer.clear();

        assert_eq!(espiao.publicados().last().unwrap(), &None);
    }

    /// Registro vazio (a v1, antes da F4) não faz o PTY recortar tela nenhuma.
    #[test]
    fn sem_manifesto_nao_ha_o_que_recortar() {
        let observer = ScreenObserver::for_session(
            &SessionKind::Shell,
            Arc::new(ManifestRegistry::default()),
            Box::new(|| None),
            Box::new(|_| {}),
        )
        .unwrap();

        assert!(!observer.wants_snapshot());
    }
}
