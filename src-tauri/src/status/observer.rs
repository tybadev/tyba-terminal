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

use crate::agent::notify::NotifyKind;
use crate::session::{ObservedAgent, SessionId, SessionKind};
use crate::status::manifest::{Scope, Verdict};
use crate::status::observed_notify::{self, ObservedNotifier, Scheduler};
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

/// O que uma sessão precisa para ganhar um observador, sem nada do Tauri.
///
/// Existe porque a montagem é fiação, e fiação errada não aparece em teste
/// nenhum: com o corpo desta fábrica dentro do `set_screen_observers` do
/// `lib.rs`, trocar o aviso por um no-op deixava a suíte inteira verde. É a
/// mesma cegueira que o PR #277 tirou do `spawn`, e a saída é a mesma —
/// nenhuma das quatro peças aqui é um tipo do Tauri, então o teste monta todas.
pub struct ObserverDeps {
    pub registry: Arc<ManifestRegistry>,
    pub process: ProcessLookup,
    pub observed: ObservedRelay,
    pub notify: NotifyRelay,
    pub scheduler: Scheduler,
}

/// O binário que a sonda de processo já detectou nesta sessão. Leitura de
/// estado, nunca a varredura: quem varre a árvore é o poll de 2 s.
pub type ProcessLookup = Arc<dyn Fn(SessionId) -> Option<String> + Send + Sync>;

/// Onde o palpite vira campo de sessão e evento de UI.
pub type ObservedRelay = Arc<dyn Fn(SessionId, Option<ObservedAgent>) + Send + Sync>;

/// Onde o palpite assentado vira aviso do sistema. A espécie viaja junto porque
/// é ela que o usuário liga e desliga — mandar `Request` daqui faria o
/// interruptor do palpite não desligar nada.
pub type NotifyRelay = Arc<dyn Fn(SessionId, NotifyKind, &str) + Send + Sync>;

/// O observador desta sessão, montado a partir das dependências.
///
/// `None` quando a sessão não pode receber palpite — ver
/// [`ScreenObserver::for_session`].
pub fn observer_for(
    deps: &ObserverDeps,
    id: SessionId,
    kind: &SessionKind,
) -> Option<ScreenObserver> {
    let process = Arc::clone(&deps.process);
    let observed = Arc::clone(&deps.observed);
    let notify = Arc::clone(&deps.notify);
    ScreenObserver::for_session(
        kind,
        Arc::clone(&deps.registry),
        Box::new(move || process(id)),
        Box::new(move |guess| observed(id, guess)),
        ObservedNotifier::new(
            Arc::new(move |agent: &str| {
                notify(
                    id,
                    NotifyKind::ObservedRequest,
                    &observed_notify::body(agent),
                );
            }),
            Arc::clone(&deps.scheduler),
        ),
    )
}

pub struct ScreenObserver {
    registry: Arc<ManifestRegistry>,
    scope: Scope,
    /// `None` fora do shell local: em SSH e em container não há árvore de
    /// processos que o TYBA alcance, e ali identidade sai só do título.
    process: Option<ProcessProbe>,
    sink: ObservedSink,
    /// Quando o palpite pode virar aviso do sistema. Separado do `sink` porque
    /// as duas saídas têm exigências diferentes: mostrar no quadro é barato e
    /// vale para todo palpite; interromper o dono da máquina passa por três
    /// guardas — ver [`crate::status::observed_notify`].
    notifier: ObservedNotifier,
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
        notifier: ObservedNotifier,
    ) -> Option<Self> {
        let scope = scope_of(kind)?;
        Some(Self {
            registry,
            scope,
            process: matches!(scope, Scope::Shell).then_some(process),
            sink,
            notifier,
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
    ///
    /// Devolve se a avaliação chegou a acontecer — `false` quando o portão de
    /// sequência cortou. Quem chama em produção ignora; é o único jeito de um
    /// teste separar "o portão segurou" de "avaliou e concluiu o mesmo", que
    /// para o resto do mundo são indistinguíveis.
    pub fn observe(&mut self, snapshot: &ScreenSnapshot) -> bool {
        // A sonda entra ANTES do portão, e isso inverte a ordem que este
        // arquivo pregava ("do mais barato para o mais caro").
        //
        // A identidade tem duas entradas — a tela e o binário —, e elas mudam
        // por relógios independentes: a tela a cada flush, o binário a cada
        // volta do poll de 2 s. Um portão que só olha a tela declara "nada
        // mudou" quando o que mudou foi a outra entrada, e aí a mudança nunca é
        // vista: `claude` cru sobe, a tela assenta, o poll descobre o processo
        // dois segundos depois e nenhuma reavaliação acontece mais.
        //
        // Em tela cheia isso é pior, não melhor: ali o recorte de linhas vem
        // vazio de propósito, então a impressão digital fica no seu estado mais
        // ESTÁVEL justamente na tela em que o Claude Code roda — e o portão
        // fecha para sempre. Era por isso que a faixa âmbar (que vem do poll)
        // e a lista (que vem da tela) discordavam na mesma janela.
        //
        // O custo é uma leitura de estado do prober por flush (~60 Hz por
        // sessão), não a varredura da árvore de processos — essa continua sendo
        // do poll. Barato não vale nada quando é a resposta errada.
        let process = self.process.as_ref().and_then(|probe| probe());
        let fingerprint = fingerprint_with(snapshot, process.as_deref());
        if self.last_fingerprint == Some(fingerprint) {
            return false;
        }
        self.last_fingerprint = Some(fingerprint);

        // Clone do `Arc` para o empréstimo do manifesto não brigar com o
        // `publish`, que é `&mut self`.
        let registry = Arc::clone(&self.registry);
        let Some(manifest) = registry.identify(self.scope, process.as_deref(), &snapshot.title)
        else {
            // Ninguém reconhece o que está na tela. Identidade é o que sustenta
            // a presença: sem ela não há agente a apontar, e manter o último
            // palpite deixaria um agente morto no quadro para sempre.
            self.publish(None, false);
            return true;
        };
        let agent = manifest.id.clone();
        // A autorização vem do manifesto que casou **agora**, não do id do
        // agente: é o arquivo que descreve as regras que sabe se elas separam
        // "esperando você" de "desenhando um menu".
        let notifies = manifest.notifies;
        let state = match manifest.evaluate(snapshot, self.scope) {
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
        self.publish(Some(ObservedAgent { agent, state }), notifies);
        true
    }

    /// O PTY morreu: o que estava na tela deixou de ser notícia do presente.
    pub fn clear(&mut self) {
        self.last_fingerprint = None;
        self.publish(None, false);
    }

    fn publish(&mut self, observed: Option<ObservedAgent>, notifies: bool) {
        // Antes do corte de repetição, e de propósito: o aviso do sistema tem a
        // própria noção de novidade — a transição —, e ela é mais estrita do que
        // esta. Uma tela que reafirma o mesmo estado com um spinner girando
        // muda o palpite em nada e ainda assim chega aqui; deixá-la fora faria
        // este método decidir por um assunto que não é dele.
        self.notifier.observed(observed.as_ref(), notifies);
        if self.last == observed {
            return;
        }
        self.last.clone_from(&observed);
        (self.sink)(observed);
    }
}

/// A impressão digital das DUAS entradas da identidade.
///
/// Separada de [`ScreenSnapshot::fingerprint`] porque aquela responde "a tela
/// mudou?", que é uma pergunta legítima e de outro dono. Esta responde "a
/// decisão pode ter mudado?", e é essa que o portão precisa.
fn fingerprint_with(snapshot: &ScreenSnapshot, process: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    snapshot.fingerprint().hash(&mut hasher);
    process.hash(&mut hasher);
    hasher.finish()
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

    struct Espiao {
        probes: AtomicUsize,
        /// O que a sonda de processo devolve. Mutável porque o poll de 2 s
        /// muda esse valor por baixo de uma tela parada — é exatamente o caso
        /// que o portão precisa enxergar.
        binario: parking_lot::Mutex<Option<String>>,
        publicados: parking_lot::Mutex<Vec<Option<ObservedAgent>>>,
    }

    impl Default for Espiao {
        fn default() -> Self {
            Self {
                probes: AtomicUsize::new(0),
                binario: parking_lot::Mutex::new(Some("codex".to_string())),
                publicados: parking_lot::Mutex::new(Vec::new()),
            }
        }
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
        observer_com(kind, espiao, registry(), ObservedNotifier::silent())
    }

    fn observer_com(
        kind: &SessionKind,
        espiao: &Arc<Espiao>,
        registry: Arc<ManifestRegistry>,
        notifier: ObservedNotifier,
    ) -> Option<ScreenObserver> {
        let probe_spy = Arc::clone(espiao);
        let sink_spy = Arc::clone(espiao);
        ScreenObserver::for_session(
            kind,
            registry,
            Box::new(move || {
                probe_spy.probes.fetch_add(1, Ordering::Relaxed);
                probe_spy.binario.lock().clone()
            }),
            Box::new(move |observed| sink_spy.publicados.lock().push(observed)),
            notifier,
        )
    }

    fn snap(title: &str, lines: &[&str]) -> ScreenSnapshot {
        ScreenSnapshot {
            title: title.into(),
            alt_screen: false,
            bottom_lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    /// O portão de sequência não é decoração: com a tela parada **e** o mesmo
    /// binário, a regra de terceiro não roda.
    ///
    /// O sensor é o retorno do `observe`, e não mais o contador de sondas: a
    /// sonda passou para antes do portão, então ela é chamada sempre. Trocar o
    /// sensor foi obrigatório — mantê-lo teria feito este teste ficar vermelho
    /// por uma mudança que ele não mede.
    #[test]
    fn o_portao_de_sequencia_pula_a_avaliacao_quando_nada_muda() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();
        let tela = snap("Codex — Action Required", &[]);

        let avaliacoes = (0..5).filter(|_| observer.observe(&tela)).count();

        assert_eq!(
            avaliacoes, 1,
            "a tela não mudou e o manifesto foi avaliado assim mesmo"
        );
        assert_eq!(espiao.publicados().len(), 1);
    }

    /// O bug que a lista de agentes tinha: `claude` cru sobe, a tela assenta, e
    /// o poll de processo só descobre o binário dois segundos depois.
    ///
    /// Sem a sonda dentro do portão, a segunda tela — idêntica à primeira — é
    /// cortada e o agente NUNCA entra na lista. Em tela cheia é permanente: o
    /// recorte de linhas vem vazio, então a impressão digital nunca mais muda.
    /// Era por isso que a faixa âmbar (poll) e a lista (tela) discordavam.
    #[test]
    fn binario_descoberto_depois_reavalia_com_a_tela_parada() {
        let espiao = Arc::new(Espiao::default());
        *espiao.binario.lock() = None;
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();
        // Tela que NÃO identifica sozinha: sem o título do agente e sem linha
        // que alguma regra reconheça. Só o binário pode identificá-la.
        let tela = snap("~/swell-system/rio-api", &[]);

        assert!(observer.observe(&tela), "primeira avaliação não aconteceu");
        assert_eq!(espiao.ultimo(), None, "identificou sem sinal nenhum");

        *espiao.binario.lock() = Some("codex".to_string());

        assert!(
            observer.observe(&tela),
            "o portão cortou a reavaliação: o binário mudou e a tela não"
        );
        assert_eq!(
            espiao.ultimo().map(|o| o.agent),
            Some("codex".to_string()),
            "o agente descoberto pelo poll não chegou na lista"
        );
    }

    /// E o contrário: binário some (agente saiu), tela igual — a linha some.
    #[test]
    fn binario_que_some_limpa_a_linha_com_a_tela_parada() {
        let espiao = Arc::new(Espiao::default());
        let mut observer = observer(&SessionKind::Shell, &espiao).unwrap();
        let tela = snap("~/swell-system/rio-api", &[]);

        observer.observe(&tela);
        assert_eq!(espiao.ultimo().map(|o| o.agent), Some("codex".to_string()));

        *espiao.binario.lock() = None;

        assert!(
            observer.observe(&tela),
            "o portão segurou a saída do agente"
        );
        assert_eq!(espiao.ultimo(), None, "agente morto ficou no quadro");
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
            ObservedNotifier::silent(),
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
            ObservedNotifier::silent(),
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

    /// O mesmo manifesto do resto do arquivo, autorizado a interromper.
    fn registry_que_avisa() -> Arc<ManifestRegistry> {
        let fonte = format!("notifies = true\n{CODEX}");
        Arc::new(ManifestRegistry::from_sources(&[fonte.as_str()]))
    }

    /// O relógio e a saída do aviso, na mão do teste.
    struct Avisos {
        agendados: parking_lot::Mutex<Vec<Box<dyn FnOnce() + Send>>>,
        saidos: parking_lot::Mutex<Vec<String>>,
    }

    impl Avisos {
        fn novo() -> (Arc<Self>, ObservedNotifier) {
            let avisos = Arc::new(Self {
                agendados: parking_lot::Mutex::new(Vec::new()),
                saidos: parking_lot::Mutex::new(Vec::new()),
            });
            let fila = Arc::clone(&avisos);
            let saida = Arc::clone(&avisos);
            let notifier = ObservedNotifier::new(
                Arc::new(move |agent: &str| saida.saidos.lock().push(agent.to_string())),
                Arc::new(move |_, task| fila.agendados.lock().push(task)),
            );
            (avisos, notifier)
        }

        /// O tempo passa: o que estava agendado acorda e decide se ainda vale.
        fn assenta(&self) {
            let tarefas: Vec<Box<dyn FnOnce() + Send>> = self.agendados.lock().drain(..).collect();
            for tarefa in tarefas {
                tarefa();
            }
        }

        fn saidos(&self) -> Vec<String> {
            self.saidos.lock().clone()
        }
    }

    /// A prova da fiação, e o modo de falha que motivou a guarda da transição.
    ///
    /// O agente saiu sem restaurar o título e a tela ficou dizendo "Action
    /// Required". O spinner embaixo continua girando, então cada quadro é uma
    /// tela **diferente** — o portão de sequência não segura nenhum, e cada um
    /// reafirma o mesmo estado. Um aviso, e só um.
    #[test]
    fn a_tela_presa_em_action_required_avisa_uma_vez_so() {
        let espiao = Arc::new(Espiao::default());
        let (avisos, notifier) = Avisos::novo();
        let mut observer =
            observer_com(&SessionKind::Shell, &espiao, registry_que_avisa(), notifier).unwrap();

        for i in 0..20 {
            observer.observe(&snap(
                "Codex — Action Required",
                &[&format!("aguardando há {i}s")],
            ));
            avisos.assenta();
        }

        assert_eq!(
            avisos.saidos(),
            ["codex"],
            "o título preso na tela rendeu um aviso por quadro"
        );
    }

    /// O `notifies` que chega ao notificador é o do manifesto que casou. Sem
    /// esta ligação, autorizar deixaria de significar alguma coisa.
    #[test]
    fn o_manifesto_sem_notifies_nao_chega_a_avisar() {
        let espiao = Arc::new(Espiao::default());
        let (avisos, notifier) = Avisos::novo();
        let mut observer =
            observer_com(&SessionKind::Shell, &espiao, registry(), notifier).unwrap();

        observer.observe(&snap("Codex — Action Required", &[]));
        avisos.assenta();

        assert_eq!(
            espiao.ultimo().unwrap().state,
            Some(ObservedState::AwaitingInput),
            "o palpite tinha que continuar aparecendo no quadro"
        );
        assert!(
            avisos.saidos().is_empty(),
            "manifesto sem `notifies` interrompeu o dono da máquina"
        );
    }

    /// Trabalhando não é esperar: o estado que chega ao notificador é o de
    /// verdade, não "algum estado".
    #[test]
    fn tela_de_agente_trabalhando_nao_avisa() {
        let espiao = Arc::new(Espiao::default());
        let (avisos, notifier) = Avisos::novo();
        let mut observer =
            observer_com(&SessionKind::Shell, &espiao, registry_que_avisa(), notifier).unwrap();

        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        avisos.assenta();

        assert_eq!(espiao.ultimo().unwrap().state, Some(ObservedState::Running));
        assert!(avisos.saidos().is_empty());
    }

    /// O agente respondeu e voltou a esperar: notícia nova, aviso novo.
    #[test]
    fn responder_e_voltar_a_esperar_avisa_de_novo() {
        let espiao = Arc::new(Espiao::default());
        let (avisos, notifier) = Avisos::novo();
        let mut observer =
            observer_com(&SessionKind::Shell, &espiao, registry_que_avisa(), notifier).unwrap();

        observer.observe(&snap("Codex — Action Required", &[]));
        avisos.assenta();
        observer.observe(&snap("Codex", &["• esc to interrupt"]));
        avisos.assenta();
        observer.observe(&snap("Codex — Action Required", &["de novo"]));
        avisos.assenta();

        assert_eq!(avisos.saidos(), ["codex", "codex"]);
    }

    /// A sonda do revisor, na tela: espera contínua com **uma piscada**.
    ///
    /// O agente continua parado esperando; o que mudou foi o desenho — um
    /// repaint, uma linha empurrada para fora das `bottom_lines`, um quadro em
    /// que o título ainda não voltou. Cada uma dessas telas é diferente da
    /// anterior, então o portão de sequência não segura nenhuma e as três
    /// chegam à avaliação. Ainda assim é **uma** espera, e uma espera
    /// interrompe uma vez.
    ///
    /// Com o assentamento só na entrada isto dava dois avisos: soltar o alvo
    /// era instantâneo, e a volta contava como transição nova.
    #[test]
    fn uma_piscada_na_tela_nao_interrompe_de_novo() {
        let espiao = Arc::new(Espiao::default());
        let (avisos, notifier) = Avisos::novo();
        // Em SSH a identidade sai só do título: é ali que a piscada dói mais,
        // porque um quadro sem o título é um quadro sem agente nenhum.
        let mut observer = observer_com(
            &SessionKind::Ssh {
                host_id: "h".into(),
            },
            &espiao,
            registry_que_avisa(),
            notifier,
        )
        .unwrap();

        observer.observe(&snap("Codex — Action Required", &["esperando"]));
        avisos.assenta();

        // Um único quadro sem o título, e a espera volta — tudo dentro da mesma
        // janela de assentamento.
        observer.observe(&snap("guilherme@mac: ~", &["esperando"]));
        observer.observe(&snap("Codex — Action Required", &["esperando ainda"]));
        avisos.assenta();

        assert_eq!(
            avisos.saidos(),
            ["codex"],
            "um redesenho do TUI interrompeu o usuário uma segunda vez"
        );
    }

    /// A fiação inteira, do recorte de tela ao aviso — sem nada do Tauri.
    ///
    /// É o teste que faltava: com a montagem dentro do `set_screen_observers`
    /// do `lib.rs`, trocar o aviso por um no-op deixava 1541 testes verdes.
    /// Aqui o quadro preso entra por [`ScreenObserver::observe`] e a asserção é
    /// sobre a tripla que chega ao adaptador — sessão, **espécie** e corpo.
    ///
    /// A espécie faz parte da asserção de propósito: mandar `Request` daqui
    /// faria o interruptor do palpite não desligar coisa nenhuma, e o do hook
    /// desligar as duas.
    #[test]
    fn a_fiacao_leva_o_palpite_assentado_ate_o_aviso() {
        type Fila = Arc<parking_lot::Mutex<Vec<Box<dyn FnOnce() + Send>>>>;
        type Avisos = Arc<parking_lot::Mutex<Vec<(SessionId, NotifyKind, String)>>>;
        type Palpites = Arc<parking_lot::Mutex<Vec<(SessionId, Option<ObservedAgent>)>>>;

        let id = SessionId::new_v4();
        let agendados: Fila = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let avisos: Avisos = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let palpites: Palpites = Arc::new(parking_lot::Mutex::new(Vec::new()));

        let fila = Arc::clone(&agendados);
        let saida = Arc::clone(&avisos);
        let quadro = Arc::clone(&palpites);
        let deps = ObserverDeps {
            registry: registry_que_avisa(),
            process: Arc::new(|_| None),
            observed: Arc::new(move |id, observed| quadro.lock().push((id, observed))),
            notify: Arc::new(move |id, kind, body| saida.lock().push((id, kind, body.to_string()))),
            scheduler: Arc::new(move |_, task| fila.lock().push(task)),
        };

        let mut observer = observer_for(&deps, id, &SessionKind::Shell).expect("shell observa");
        observer.observe(&snap("Codex — Action Required", &[]));

        // O palpite chega ao quadro na hora; o aviso, só depois do assentamento.
        assert_eq!(palpites.lock().len(), 1, "o palpite não chegou ao quadro");
        assert!(avisos.lock().is_empty());

        for tarefa in agendados.lock().drain(..).collect::<Vec<_>>() {
            tarefa();
        }

        assert_eq!(
            avisos.lock().as_slice(),
            [(
                id,
                NotifyKind::ObservedRequest,
                crate::status::observed_notify::body("codex")
            )]
        );
    }

    /// Registro vazio (a v1, antes da F4) não faz o PTY recortar tela nenhuma.
    #[test]
    fn sem_manifesto_nao_ha_o_que_recortar() {
        let observer = ScreenObserver::for_session(
            &SessionKind::Shell,
            Arc::new(ManifestRegistry::default()),
            Box::new(|| None),
            Box::new(|_| {}),
            ObservedNotifier::silent(),
        )
        .unwrap();

        assert!(!observer.wants_snapshot());
    }
}
