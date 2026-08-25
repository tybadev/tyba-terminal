//! Quando um palpite de tela vira aviso do sistema — e as três razões para não
//! virar.
//!
//! O palpite é barato de mostrar e caro de gritar. No quadro, um estado errado
//! custa um ponto colorido fora de hora; num aviso do sistema, custa a atenção
//! de quem estava fazendo outra coisa. Por isso o caminho até o barulho tem três
//! guardas, e as três precisam passar:
//!
//! 1. **Só `AwaitingInput`.** Nunca `Idle`, nunca `Running`. Um agente sem gate
//!    que terminou não interrompe ninguém: não há nada a responder, e o trabalho
//!    já aconteceu. O que justifica o aviso é o agente estar **parado à espera
//!    de alguém** — quem lê a tela não pode oferecer menos do que isso.
//! 2. **Só manifesto que declara `notifies`** — a guarda vive em
//!    [`crate::status::manifest`], e o default dela é `false`. Descrever como
//!    reconhecer uma tela é uma coisa; autorizar interrupção é outra.
//! 3. **Só quando o estado assenta, e o assentamento é simétrico.** É a guarda
//!    deste módulo, e a que mais custou a acertar.
//!
//! ## Por que o assentamento tem de valer para os dois lados
//!
//! A primeira versão assentava só a **entrada**: o palpite tinha de se manter
//! por [`OBSERVED_SETTLE_MS`] para virar aviso, mas soltava o alvo no instante
//! em que o estado deixava de ser espera. Parecia certo e estava errado, porque
//! nada distingue um turno de verdade de um redesenho do TUI. Uma espera
//! contínua com **uma piscada no meio** — uma linha empurrada para fora das
//! `bottom_lines`, um repaint, um frame do spinner que apagou o título — saía
//! como duas transições, e interrompia o dono da máquina duas vezes. N piscadas,
//! N avisos. Era exatamente o custo que as três guardas existiam para evitar.
//!
//! Então o que assenta não é a entrada: é **o estado**. Há um alvo observado (o
//! `candidate`, que segue a tela de perto) e um alvo assentado (o `settled`, que
//! só muda depois de o candidato se manter pelo intervalo inteiro). O aviso sai
//! na mudança do **assentado**, e uma piscada mais curta que a janela nunca
//! chega a mudá-lo.
//!
//! A alternativa considerada foi um cooldown por (sessão, agente): não avisar de
//! novo antes de N minutos. Foi descartada porque escolhe um número arbitrário
//! para uma pergunta que tem resposta certa — um cooldown curto ainda deixa a
//! piscada passar, e um longo engole o turno seguinte, que é notícia legítima.
//! O assentamento simétrico não estima nada: ele mede a mesma coisa nos dois
//! sentidos, e "o agente voltou a trabalhar" passa a exigir a mesma evidência
//! que "o agente parou".
//!
//! O relógio entra por parâmetro ([`Scheduler`]) porque a alternativa é um teste
//! que dorme: o assentamento é a única parte disto que não se prova sem tempo, e
//! prová-la com `sleep` compraria uma suíte lenta e intermitente.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::session::{ObservedAgent, ObservedState};

/// Quanto tempo um estado precisa se manter para virar o estado assentado.
///
/// O precedente é o `TURN_END_SETTLE_MS` do fim de turno, e pela mesma razão:
/// dois segundos é longo o bastante para uma rajada de desenho terminar e curto
/// o bastante para o aviso ainda ser notícia de agora.
pub const OBSERVED_SETTLE_MS: u64 = 2000;

// O assentamento tem de ser tempo de verdade: zero é a guarda 3 desligada, e um
// valor curto demais não distingue uma espera de um quadro no meio de uma
// rajada de desenho. Conferido na **compilação** porque uma asserção de runtime
// sobre uma constante é uma tautologia que o compilador resolve sozinho — o
// gate não passa se alguém zerar isto.
const _: () = assert!(OBSERVED_SETTLE_MS >= 500);

/// Quem entrega o aviso. Recebe o id do manifesto que reconheceu a sessão.
///
/// `Arc` e não `Box` porque quem chama é o assentamento, numa thread que ainda
/// não existia quando o alvo foi armado.
pub type NotifySink = Arc<dyn Fn(&str) + Send + Sync>;

/// Quem faz o tempo passar. Em produção é uma thread que dorme; em teste é o
/// próprio teste, que dispara quando quiser.
pub type Scheduler = Arc<dyn Fn(Duration, Box<dyn FnOnce() + Send>) + Send + Sync>;

/// O agendador de produção: uma thread por armação.
///
/// Uma thread parece caro até se olhar a frequência: armar só acontece quando o
/// **alvo** muda — uma vez quando o agente para, uma vez quando ele volta a
/// trabalhar. O caminho quente do PTY, com dezenas de avaliações por segundo
/// sobre o mesmo alvo, não arma nada.
pub fn sleeping_scheduler() -> Scheduler {
    Arc::new(|delay, task| {
        let _ = std::thread::Builder::new()
            .name("observed-settle".into())
            .spawn(move || {
                std::thread::sleep(delay);
                task();
            });
    })
}

/// O texto do aviso.
///
/// Diz de onde veio: "palpite" não é modéstia, é o que separa este aviso do
/// pedido de um agente com hook. Quem for até a sessão e encontrar o agente
/// trabalhando precisa saber por que foi chamado à toa.
pub fn body(agent: &str) -> String {
    format!("Parece estar esperando você — palpite da tela ({agent})")
}

/// A máquina da terceira guarda: candidato, assentado e cancelamento.
pub struct ObservedNotifier {
    sink: NotifySink,
    scheduler: Scheduler,
    settle: Duration,
    /// O alvo que a tela mostra **agora**, assentado ou não. `None` é "não há
    /// ninguém a avisar" — o estado inicial e o de todo palpite que não passa
    /// nas guardas 1 e 2.
    candidate: Option<String>,
    /// O alvo que se manteve pelo intervalo inteiro. É a mudança **dele** que
    /// vira aviso.
    ///
    /// Compartilhado com as tarefas de assentamento, que rodam noutra thread e
    /// não alcançam o `&mut self` do observador.
    settled: Arc<Mutex<Option<String>>>,
    /// Qual armação ainda vale. Trocar de candidato incrementa isto, e a tarefa
    /// que acorda com o número velho desiste — é o que impede a piscada de
    /// aplicar um candidato que já não existe.
    generation: Arc<AtomicU64>,
}

impl ObservedNotifier {
    pub fn new(sink: NotifySink, scheduler: Scheduler) -> Self {
        Self {
            sink,
            scheduler,
            settle: Duration::from_millis(OBSERVED_SETTLE_MS),
            candidate: None,
            settled: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Um notificador que não notifica, para os testes cujo assunto é outro.
    /// Fora de teste não existe: um aviso perdido por um `silent()` esquecido
    /// não deixa rastro nenhum.
    #[cfg(test)]
    pub fn silent() -> Self {
        Self::new(Arc::new(|_| {}), Arc::new(|_, _| {}))
    }

    /// O palpite mudou (ou não). Chamado a cada avaliação de manifesto.
    ///
    /// `notifies` é a guarda 2, e vem do manifesto que produziu este palpite —
    /// não do agente, que é só um id: dois manifestos podem reivindicar o mesmo
    /// nome, e quem autoriza é o arquivo que casou agora.
    pub fn observed(&mut self, observed: Option<&ObservedAgent>, notifies: bool) {
        let target = target_of(observed, notifies);
        // O candidato não mudou: nada a fazer, e é importante que seja **nada**.
        // Rearmar aqui reiniciaria a janela a cada quadro, e um spinner girando
        // embaixo de um título parado adiaria o assentamento para sempre — o
        // agente esperaria, a tela mudaria, e o aviso nunca sairia.
        if self.candidate.as_deref() == target {
            return;
        }
        self.candidate = target.map(str::to_string);
        // Trocar de candidato invalida a armação anterior. É isto que faz a
        // piscada se cancelar sozinha: a volta ao alvo antigo aposenta a tarefa
        // que ia soltá-lo.
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        let candidate = self.candidate.clone();
        let armed = Arc::clone(&self.generation);
        let settled = Arc::clone(&self.settled);
        let sink = Arc::clone(&self.sink);
        (self.scheduler)(
            self.settle,
            Box::new(move || {
                if armed.load(Ordering::SeqCst) != generation {
                    return;
                }
                // A troca do estado assentado sob o lock; o aviso, fora dele —
                // o sink chega até a API de notificação do sistema, e segurar um
                // mutex atravessando isso seria pedir para descobrir o custo do
                // outro lado.
                let anunciar = {
                    let mut settled = settled.lock();
                    if *settled == candidate {
                        None
                    } else {
                        settled.clone_from(&candidate);
                        candidate
                    }
                };
                // Assentar em `None` é o agente ter voltado a trabalhar: muda o
                // estado e não avisa ninguém.
                if let Some(agent) = anunciar {
                    sink(&agent);
                }
            }),
        );
    }
}

/// A guarda 1 e a guarda 2, no mesmo lugar porque as duas respondem a mesma
/// pergunta: existe alguém a avisar aqui?
fn target_of(observed: Option<&ObservedAgent>, notifies: bool) -> Option<&str> {
    let observed = observed?;
    if !notifies {
        return None;
    }
    // `None` no estado é presença sem estado: há um agente ali e o sinal não diz
    // o que ele faz. Interromper alguém com isso seria chutar.
    (observed.state == Some(ObservedState::AwaitingInput)).then_some(observed.agent.as_str())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use super::*;

    type Tarefa = Box<dyn FnOnce() + Send>;

    /// O relógio do teste, e o que saiu por ele.
    struct Bancada {
        notifier: ObservedNotifier,
        agendados: Arc<Mutex<Vec<(Duration, Tarefa)>>>,
        avisos: Arc<Mutex<Vec<String>>>,
    }

    impl Bancada {
        fn nova() -> Self {
            let agendados: Arc<Mutex<Vec<(Duration, Tarefa)>>> = Arc::new(Mutex::new(Vec::new()));
            let avisos: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let fila = Arc::clone(&agendados);
            let saida = Arc::clone(&avisos);
            let notifier = ObservedNotifier::new(
                Arc::new(move |agent: &str| saida.lock().push(agent.to_string())),
                Arc::new(move |delay, task| fila.lock().push((delay, task))),
            );
            Self {
                notifier,
                agendados,
                avisos,
            }
        }

        fn ve(&mut self, agent: &str, state: Option<ObservedState>, notifies: bool) {
            let observed = ObservedAgent {
                agent: agent.into(),
                state,
            };
            self.notifier.observed(Some(&observed), notifies);
        }

        /// O palpite sumiu — identidade perdida, ou o PTY morreu.
        fn perde(&mut self) {
            self.notifier.observed(None, false);
        }

        /// O relógio anda o assentamento inteiro: tudo que estava agendado
        /// acorda, na ordem em que foi agendado, e decide se ainda vale.
        fn assenta(&self) {
            let tarefas: Vec<(Duration, Tarefa)> = self.agendados.lock().drain(..).collect();
            for (_, tarefa) in tarefas {
                tarefa();
            }
        }

        /// Os prazos pedidos ao agendador, sem consumi-los.
        fn prazos(&self) -> Vec<Duration> {
            self.agendados.lock().iter().map(|(d, _)| *d).collect()
        }

        fn avisos(&self) -> Vec<String> {
            self.avisos.lock().clone()
        }
    }

    #[test]
    fn awaiting_input_de_manifesto_autorizado_vira_aviso() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();

        assert_eq!(b.avisos(), ["codex"]);
    }

    /// Guarda 1. Conclusão de agente sem gate não interrompe ninguém, e agente
    /// trabalhando muito menos.
    #[test]
    fn idle_e_running_nunca_notificam() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::Running), true);
        b.assenta();
        b.ve("codex", Some(ObservedState::Idle), true);
        b.assenta();

        assert!(b.avisos().is_empty(), "estado que não é espera virou aviso");
    }

    /// Presença sem estado é o caso do agente que o título identifica e não
    /// descreve. Não se interrompe alguém por um agente estar aberto.
    #[test]
    fn presenca_sem_estado_nao_notifica() {
        let mut b = Bancada::nova();

        b.ve("opencode", None, true);
        b.assenta();

        assert!(b.avisos().is_empty());
    }

    /// Guarda 2. O manifesto que não pediu para interromper não interrompe.
    #[test]
    fn manifesto_que_nao_declara_notifies_fica_calado() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), false);
        b.assenta();

        assert!(
            b.avisos().is_empty(),
            "manifesto sem `notifies` virou aviso do sistema"
        );
    }

    /// Guarda 3 — e o modo de falha que a motiva.
    ///
    /// O agente sai sem restaurar o título e a tela fica dizendo "esperando"
    /// para sempre. Cada avaliação seguinte reafirma o mesmo estado; nenhuma é
    /// notícia nova.
    #[test]
    fn permanecer_em_awaiting_input_nao_renotifica() {
        let mut b = Bancada::nova();

        for _ in 0..20 {
            b.ve("codex", Some(ObservedState::AwaitingInput), true);
            b.assenta();
        }

        assert_eq!(
            b.avisos(),
            ["codex"],
            "o estado preso na tela rendeu um aviso por avaliação"
        );
    }

    /// **O achado que derrubou a primeira versão.**
    ///
    /// Uma espera contínua com uma piscada no meio: um único quadro em que o
    /// título sumiu — um repaint, uma linha empurrada para fora da região — e o
    /// estado volta ao que era. Para o usuário isso é uma espera só, e uma
    /// espera só interrompe uma vez.
    ///
    /// Com o assentamento só na entrada, isto rendia **dois** avisos: a saída
    /// era instantânea, então a volta contava como transição nova. Com o
    /// assentamento simétrico, a saída também precisa se manter — e uma piscada
    /// mais curta que a janela nunca chega a soltar o alvo.
    #[test]
    fn uma_piscada_no_meio_da_espera_nao_renotifica() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();

        // A piscada: sai e volta dentro da mesma janela de assentamento.
        b.ve("codex", Some(ObservedState::Running), true);
        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();

        assert_eq!(
            b.avisos(),
            ["codex"],
            "um redesenho do TUI interrompeu o usuário uma segunda vez"
        );
    }

    /// N piscadas não são N avisos. O caso de um TUI que repinta a cada
    /// segundo — a versão antiga rendia um aviso por repaint.
    #[test]
    fn piscadas_repetidas_continuam_valendo_um_aviso_so() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();
        for _ in 0..10 {
            b.ve("codex", Some(ObservedState::Running), true);
            b.ve("codex", Some(ObservedState::AwaitingInput), true);
            b.assenta();
        }

        assert_eq!(b.avisos(), ["codex"]);
    }

    /// O outro lado da simetria: um turno de verdade **é** notícia nova.
    ///
    /// A diferença entre isto e a piscada acima é uma só, e é a que importa: o
    /// estado de não-espera se manteve pelo intervalo inteiro. É o que separa o
    /// usuário ter respondido de o TUI ter repintado.
    #[test]
    fn so_renotifica_se_a_saida_tambem_assentar() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();
        // O agente trabalha o intervalo inteiro: a saída assenta.
        b.ve("codex", Some(ObservedState::Running), true);
        b.assenta();
        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();

        assert_eq!(b.avisos(), ["codex", "codex"]);
    }

    /// Guarda 3, a parte do prazo: nada sai na hora.
    #[test]
    fn o_aviso_nao_sai_antes_do_assentamento() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);

        assert!(
            b.avisos().is_empty(),
            "o aviso saiu sem esperar o estado se manter"
        );
        b.assenta();
        assert_eq!(b.avisos(), ["codex"]);
    }

    /// O que o assentamento existe para pegar: a tela ainda estava sendo
    /// desenhada, o estado seguinte desmente o anterior, e o aviso que teria
    /// saído era sobre algo que não estava acontecendo.
    #[test]
    fn estado_que_muda_dentro_da_janela_cancela_o_aviso() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.ve("codex", Some(ObservedState::Running), true);
        b.assenta();

        assert!(b.avisos().is_empty(), "o aviso cancelado saiu assim mesmo");
    }

    #[test]
    fn perder_o_agente_dentro_da_janela_cancela_o_aviso() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.perde();
        b.assenta();

        assert!(b.avisos().is_empty());
    }

    /// A tela troca de dono dentro da janela: quem é avisado é quem está lá
    /// agora, e uma vez só.
    #[test]
    fn trocar_de_agente_dentro_da_janela_avisa_pelo_agente_certo() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.ve("gemini", Some(ObservedState::AwaitingInput), true);
        b.assenta();

        assert_eq!(b.avisos(), ["gemini"]);
    }

    /// Um manifesto que perde a autorização entre avaliações não deixa aviso
    /// armado para trás.
    #[test]
    fn perder_a_autorizacao_dentro_da_janela_cancela_o_aviso() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.ve("codex", Some(ObservedState::AwaitingInput), false);
        b.assenta();

        assert!(b.avisos().is_empty());
    }

    /// O alvo que não muda não rearma.
    ///
    /// Não é economia de thread: rearmar reiniciaria a janela a cada quadro, e
    /// um spinner girando embaixo de um título parado adiaria o assentamento
    /// para sempre — o agente esperando, a tela mudando, e o aviso nunca saindo.
    #[test]
    fn reafirmar_o_mesmo_alvo_nao_reinicia_a_janela() {
        let mut b = Bancada::nova();

        for _ in 0..20 {
            b.ve("codex", Some(ObservedState::AwaitingInput), true);
        }

        assert_eq!(
            b.prazos().len(),
            1,
            "cada quadro rearmou o assentamento, e a janela nunca fecharia"
        );
    }

    /// O prazo que chega ao agendador é o assentamento inteiro.
    ///
    /// Sem esta afirmação, passar `Duration::ZERO` ali deixaria a suíte verde e
    /// a guarda 3 desligada em produção — os testes fazem o tempo passar na mão
    /// e não olhariam o prazo.
    #[test]
    fn o_agendamento_pede_o_assentamento_inteiro() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);

        assert_eq!(b.prazos(), [Duration::from_millis(OBSERVED_SETTLE_MS)]);
    }

    /// O agendador de produção precisa mesmo esperar.
    ///
    /// É o único teste que gasta tempo de relógio, e não há como não gastar:
    /// `sleeping_scheduler` sem o `sleep` é indistinguível de um que espera, a
    /// não ser esperando. Prazo curto para a suíte não sofrer, e a margem entre
    /// os dois (25 ms contra 150 ms) é de seis vezes.
    #[test]
    fn o_agendador_de_producao_espera_o_prazo() {
        let feito = Arc::new(AtomicBool::new(false));
        let marca = Arc::clone(&feito);

        sleeping_scheduler()(
            Duration::from_millis(150),
            Box::new(move || marca.store(true, Ordering::SeqCst)),
        );

        std::thread::sleep(Duration::from_millis(25));
        assert!(
            !feito.load(Ordering::SeqCst),
            "o agendador rodou a tarefa na hora, e assentamento nenhum acontece"
        );

        // Sem condição de parada derivada do fenômeno: um teto fixo e generoso,
        // e a asserção é sobre o que aconteceu dentro dele.
        let limite = Instant::now() + Duration::from_secs(5);
        while !feito.load(Ordering::SeqCst) && Instant::now() < limite {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            feito.load(Ordering::SeqCst),
            "a tarefa agendada nunca rodou"
        );
    }

    #[test]
    fn o_texto_do_aviso_diz_que_e_palpite() {
        let texto = body("codex");
        assert!(texto.contains("palpite"), "{texto}");
        assert!(texto.contains("codex"), "{texto}");
    }
}
