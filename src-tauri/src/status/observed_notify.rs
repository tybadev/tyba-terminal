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
//! 3. **Só na transição, e depois de um assentamento.** É a guarda deste
//!    módulo, e a que responde ao modo de falha que motivou tudo: um agente que
//!    morre sem restaurar o título deixa o "Action Required" na tela para
//!    sempre. Sem a transição, isso seria um aviso por avaliação, indefinidamente.
//!    Sem o assentamento, um quadro no meio de uma rajada — uma tela sendo
//!    desenhada, meia pergunta já visível — viraria aviso antes de o agente
//!    terminar de escrever, e o estado seguinte desmentiria o barulho que já
//!    saiu.
//!
//! O relógio entra por parâmetro ([`Scheduler`]) porque a alternativa é um teste
//! que dorme: o assentamento é a única parte disto que não se prova sem tempo, e
//! prová-la com `sleep` compraria uma suíte lenta e intermitente.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::session::{ObservedAgent, ObservedState};

/// Quanto tempo o palpite precisa se manter antes de virar aviso.
///
/// O precedente é o `TURN_END_SETTLE_MS` do fim de turno, e pela mesma razão:
/// dois segundos é longo o bastante para uma rajada de desenho terminar e curto
/// o bastante para o aviso ainda ser notícia de agora.
pub const OBSERVED_SETTLE_MS: u64 = 2000;

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
/// Uma thread parece caro até se olhar a frequência: armar só acontece na
/// **transição** para `AwaitingInput`, que é uma vez por turno de agente. O
/// caminho quente do PTY — dezenas de avaliações por segundo — não arma nada.
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

/// A máquina da terceira guarda: transição, assentamento e cancelamento.
pub struct ObservedNotifier {
    sink: NotifySink,
    scheduler: Scheduler,
    settle: Duration,
    /// De quem é o aviso que está armado. `None` é "não há nada a avisar" — o
    /// estado inicial e o de todo palpite que não passa nas guardas 1 e 2.
    target: Option<String>,
    /// Qual armação ainda vale. Compartilhado com o assentamento, que roda
    /// noutra thread e não tem como olhar o `target`: mudar de alvo incrementa
    /// isto, e o assentamento que acorda com o número velho desiste.
    generation: Arc<AtomicU64>,
}

impl ObservedNotifier {
    pub fn new(sink: NotifySink, scheduler: Scheduler) -> Self {
        Self {
            sink,
            scheduler,
            settle: Duration::from_millis(OBSERVED_SETTLE_MS),
            target: None,
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
        // A guarda da transição. Enquanto o alvo é o mesmo, nada acontece: o
        // agente que permanece esperando já foi anunciado, e o que ficou preso
        // num título que ninguém restaurou não vai ser anunciado de novo.
        if self.target.as_deref() == target {
            return;
        }
        // Toda troca de alvo invalida o que estivesse armado — inclusive quando
        // o alvo novo é `None`. É isto que faz o estado que muda dentro da
        // janela cancelar o aviso em vez de atrasá-lo.
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.target = target.map(str::to_string);
        let Some(agent) = self.target.clone() else {
            return;
        };
        let armed = Arc::clone(&self.generation);
        let sink = Arc::clone(&self.sink);
        (self.scheduler)(
            self.settle,
            Box::new(move || {
                if armed.load(Ordering::SeqCst) != generation {
                    return;
                }
                sink(&agent);
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
    use parking_lot::Mutex;

    use super::*;

    type Tarefa = Box<dyn FnOnce() + Send>;

    /// O relógio do teste, e o que saiu por ele.
    struct Bancada {
        notifier: ObservedNotifier,
        agendados: Arc<Mutex<Vec<Tarefa>>>,
        avisos: Arc<Mutex<Vec<String>>>,
    }

    impl Bancada {
        fn nova() -> Self {
            let agendados: Arc<Mutex<Vec<Tarefa>>> = Arc::new(Mutex::new(Vec::new()));
            let avisos: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let fila = Arc::clone(&agendados);
            let saida = Arc::clone(&avisos);
            let notifier = ObservedNotifier::new(
                Arc::new(move |agent: &str| saida.lock().push(agent.to_string())),
                Arc::new(move |_delay, task| fila.lock().push(task)),
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
        /// acorda e decide se ainda vale.
        fn assenta(&self) {
            let tarefas: Vec<Tarefa> = self.agendados.lock().drain(..).collect();
            for tarefa in tarefas {
                tarefa();
            }
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

    /// Guarda 3, a parte da transição — e o modo de falha que a motiva.
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

    /// Sair e voltar é notícia outra vez: o agente respondeu, trabalhou e parou
    /// de novo.
    #[test]
    fn sair_do_estado_e_voltar_renotifica() {
        let mut b = Bancada::nova();

        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();
        b.ve("codex", Some(ObservedState::Running), true);
        b.assenta();
        b.ve("codex", Some(ObservedState::AwaitingInput), true);
        b.assenta();

        assert_eq!(b.avisos(), ["codex", "codex"]);
    }

    /// Guarda 3, a parte do assentamento: nada sai na hora.
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

    #[test]
    fn o_texto_do_aviso_diz_que_e_palpite() {
        let texto = body("codex");
        assert!(texto.contains("palpite"), "{texto}");
        assert!(texto.contains("codex"), "{texto}");
    }
}
