//! Política do aviso do sistema: quando sai e como soa.
//!
//! Até aqui todo aviso de agente saía igual. Num setup com um agente isso não
//! incomoda; com quatro, "preciso de você agora" e "terminei" competem pelo
//! mesmo som, e o dono aprende a ignorar os dois. Separar é o que devolve
//! significado ao barulho.
//!
//! A resolução é função pura de propósito — ela decide a partir do que está
//! gravado, e quem lê o banco fica de fora. É o que permite testar o default
//! sem SQLite e sem tocar o `AppHandle`.

/// Por que o aviso está saindo.
///
/// São duas urgências diferentes, não dois rótulos: pedido interrompe (o agente
/// está parado até alguém responder), conclusão pode esperar (o trabalho já
/// aconteceu). É essa diferença que o som carrega.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyKind {
    /// O agente está bloqueado — aprovação pendente ou pergunta aberta.
    Request,
    /// O turno terminou.
    Done,
    /// O **palpite** de que um agente sem hook está esperando alguém, deduzido
    /// da tela.
    ///
    /// Espécie própria, e não um `Request` de outra origem, porque a diferença
    /// não é de urgência: é de certeza. `Request` é o agente falando por um
    /// hook — o TYBA sabe. Este aqui é o TYBA lendo a tela de um programa que
    /// não faz ideia de que está sendo lido, e que pode mudar a interface na
    /// próxima versão sem avisar. Quem cansar dos palpites errados desliga só
    /// este; misturá-los tiraria do usuário a escolha que importa, porque
    /// desligar o palpite desligaria o fato junto.
    ObservedRequest,
}

impl NotifyKind {
    /// Chaves fixas: mudá-las faz a preferência já gravada deixar de ser
    /// encontrada, e o usuário volta ao default sem ter mexido em nada.
    pub const fn enabled_key(self) -> &'static str {
        match self {
            NotifyKind::Request => "pref.notify.request.enabled",
            NotifyKind::Done => "pref.notify.done.enabled",
            NotifyKind::ObservedRequest => "pref.notify.observed_request.enabled",
        }
    }

    pub const fn sound_key(self) -> &'static str {
        match self {
            NotifyKind::Request => "pref.notify.request.sound",
            NotifyKind::Done => "pref.notify.done.sound",
            NotifyKind::ObservedRequest => "pref.notify.observed_request.sound",
        }
    }
}

/// Sons de fábrica, por plataforma.
///
/// No macOS os dois nomes vêm de `/System/Library/Sounds` e existem em toda
/// instalação — inventar um nome faz o aviso sair mudo, não faz o sistema cair.
/// Fora do macOS o default é não mexer: no Linux o nome é do tema de sons do
/// freedesktop e no Windows é outro conjunto, então chutar ali trocaria um som
/// certo por silêncio.
pub const fn default_sound(kind: NotifyKind) -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        match kind {
            NotifyKind::Request => Some("Ping"),
            NotifyKind::Done => Some("Glass"),
            // O mesmo som do pedido de propósito: o som carrega **urgência**, e
            // um agente esperando é igualmente urgente venha o sinal do hook ou
            // da tela. O que separa as duas espécies é o interruptor, não o
            // timbre — e quem quiser distinguir de ouvido troca este aqui.
            NotifyKind::ObservedRequest => Some("Ping"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyPolicy {
    /// Falso silencia o aviso do sistema. O toast dentro do app continua: quem
    /// desligou o aviso pediu para não ser interrompido fora da janela, não para
    /// perder o evento.
    pub enabled: bool,
    /// `None` deixa o som a cargo do sistema.
    pub sound: Option<String>,
}

/// A política a partir do que está gravado.
///
/// Ausente é diferente de vazio, e a diferença importa: **ausente** é "nunca
/// escolhi", que cai no default de fábrica; **vazio** é "escolhi silêncio", que
/// precisa sobreviver — senão desligar o som seria impossível, porque a string
/// vazia voltaria a virar o default a cada leitura.
pub fn resolve(
    kind: NotifyKind,
    enabled_raw: Option<&str>,
    sound_raw: Option<&str>,
) -> NotifyPolicy {
    let enabled = !matches!(enabled_raw, Some("off") | Some("false") | Some("0"));
    let sound = match sound_raw {
        None => default_sound(kind).map(str::to_string),
        Some("") => None,
        Some(name) => Some(name.to_string()),
    };
    NotifyPolicy { enabled, sound }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sem_preferencia_gravada_o_aviso_sai_com_o_som_de_fabrica() {
        let policy = resolve(NotifyKind::Request, None, None);
        assert!(policy.enabled);
        assert_eq!(policy.sound.as_deref(), default_sound(NotifyKind::Request));
    }

    #[test]
    fn string_vazia_e_silencio_escolhido_e_nao_volta_para_o_default() {
        // O caso que a implementação ingênua erra: tratar `Some("")` como
        // ausente faria o default reaparecer, e o usuário não conseguiria
        // desligar o som de jeito nenhum.
        let policy = resolve(NotifyKind::Done, None, Some(""));
        assert_eq!(policy.sound, None);
    }

    #[test]
    fn nome_escolhido_ganha_do_default() {
        let policy = resolve(NotifyKind::Done, None, Some("Submarine"));
        assert_eq!(policy.sound.as_deref(), Some("Submarine"));
    }

    #[test]
    fn desligar_silencia_o_aviso_do_sistema() {
        assert!(!resolve(NotifyKind::Request, Some("off"), None).enabled);
        assert!(resolve(NotifyKind::Request, Some("on"), None).enabled);
    }

    #[test]
    fn valor_estranho_mantem_o_aviso_ligado() {
        // Fail-open aqui é o certo, ao contrário do gate de aprovação: o pior
        // caso é um aviso a mais, e perder o aviso de um agente bloqueado por
        // causa de um valor corrompido é o dano maior.
        assert!(resolve(NotifyKind::Request, Some("talvez"), None).enabled);
    }

    const ESPECIES: [NotifyKind; 3] = [
        NotifyKind::Request,
        NotifyKind::Done,
        NotifyKind::ObservedRequest,
    ];

    #[test]
    fn as_chaves_das_especies_nao_colidem() {
        let mut chaves: Vec<&str> = ESPECIES
            .iter()
            .flat_map(|k| [k.enabled_key(), k.sound_key()])
            .collect();
        let total = chaves.len();
        chaves.sort_unstable();
        chaves.dedup();
        assert_eq!(chaves.len(), total, "duas espécies dividem a mesma chave");
        // Toda chave de preferência precisa do prefixo, ou `set_pref` recusa.
        for key in chaves {
            assert!(key.starts_with("pref."), "{key} sem o prefixo `pref.`");
        }
    }

    /// O ponto de o palpite ter espécie própria: desligá-lo não pode desligar o
    /// fato.
    ///
    /// O teste lê **pelas chaves**, como o `notify_native` faz, e não passando
    /// os valores na mão — é a leitura por chave que faz uma espécie alcançar a
    /// preferência da outra quando as chaves colidem.
    #[test]
    fn desligar_o_palpite_nao_desliga_o_pedido_do_hook() {
        // O banco com só a preferência do palpite gravada.
        let gravado = [(NotifyKind::ObservedRequest.enabled_key(), "off")];
        let leia = |kind: NotifyKind| {
            let bruto = gravado
                .iter()
                .find(|(chave, _)| *chave == kind.enabled_key())
                .map(|(_, valor)| *valor);
            resolve(kind, bruto, None)
        };

        assert!(!leia(NotifyKind::ObservedRequest).enabled);
        assert!(
            leia(NotifyKind::Request).enabled,
            "desligar o palpite levou o pedido do hook junto"
        );
        assert!(leia(NotifyKind::Done).enabled);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn no_macos_pedido_e_conclusao_soam_diferente_de_fabrica() {
        // É o ponto da entrega: se os dois defaults empatarem, nada foi feito.
        assert_ne!(
            default_sound(NotifyKind::Request),
            default_sound(NotifyKind::Done)
        );
        assert!(default_sound(NotifyKind::Request).is_some());
    }
}
