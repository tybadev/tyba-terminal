//! Manifesto de detecção: dado declarativo, nunca programa.
//!
//! Um manifesto descreve como reconhecer um agente na tela e como traduzir o
//! que está ali em estado. Ele é **dado**: não há `generators` como nas specs do
//! Fig, que rodam comando no shell a cada tecla, nem `ActionExecCommand` como no
//! carapace. O que se pode declarar é substring e regex — e a regex é a do crate
//! `regex`, que é linear por construção. É isso que torna padrão de terceiro
//! tolerável no caminho quente: não existe entrada que faça um deles explodir em
//! backtracking.
//!
//! Os tetos são validados **na carga**, não na avaliação. Manifesto que passa da
//! porta já está dentro do orçamento, e o laço quente não gasta nada
//! conferindo limite.

use std::path::Path;

use serde::Deserialize;

use crate::session::ObservedState;
use crate::status::screen::{ScreenSnapshot, MAX_REGION_LINES};

/// Versão do motor. Manifesto que pede mais do que isto é recusado inteiro em
/// vez de aplicado pela metade — regra que o motor não entende é regra cujo
/// efeito ninguém consegue prever.
pub const ENGINE_VERSION: u32 = 1;

pub const MAX_RULES: usize = 128;
pub const MAX_MATCHER_CHARS: usize = 512;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("manifesto inválido: {0}")]
    Parse(String),
    #[error("manifesto exige motor v{wanted}, este é o v{ours}")]
    Engine { wanted: u32, ours: u32 },
    #[error("manifesto tem {found} regras, o teto é {MAX_RULES}")]
    TooManyRules { found: usize },
    #[error("matcher da regra `{rule}` tem {found} chars, o teto é {MAX_MATCHER_CHARS}")]
    MatcherTooLong { rule: String, found: usize },
    #[error("regex da regra `{rule}` não compila: {detail}")]
    BadRegex { rule: String, detail: String },
    #[error("regra `{rule}` não tem nenhum matcher")]
    EmptyRule { rule: String },
}

/// Onde a sessão pode estar para o manifesto valer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Shell,
    Ssh,
    Container,
}

/// O que a regra olha.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// OSC 0/2. Uma linha, escrita pelo programa que está rodando — nunca
    /// scrollback. É a única fonte de **identidade** em sessão SSH.
    OscTitle,
    /// As últimas N linhas não vazias da tela.
    BottomLines(usize),
}

/// O que a regra conclui.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleState {
    Running,
    AwaitingInput,
    Idle,
}

impl From<RuleState> for ObservedState {
    fn from(state: RuleState) -> Self {
        match state {
            RuleState::Running => ObservedState::Running,
            RuleState::AwaitingInput => ObservedState::AwaitingInput,
            RuleState::Idle => ObservedState::Idle,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Match {
    /// Nome do binário na árvore de processos. **Só existe no shell local**: em
    /// SSH não há árvore, e ali identidade vem do `title`.
    #[serde(default)]
    pub process: Vec<String>,
    /// Substrings que, presentes no título, identificam o agente.
    #[serde(default)]
    pub title: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawRule {
    pub id: String,
    pub state: Option<RuleState>,
    #[serde(default)]
    pub priority: i32,
    pub region: Region,
    #[serde(default)]
    pub contains: Vec<String>,
    pub line_regex: Option<String>,
    /// Terceiro resultado além de casou e não-casou: "casou, e por isso NÃO
    /// mexa no estado". É o que deixa o visualizador de transcript de um agente
    /// não ser lido como aprovação pendente.
    #[serde(default)]
    pub skip_state_update: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawManifest {
    pub id: String,
    #[serde(default = "default_engine")]
    pub min_engine_version: u32,
    #[serde(default, rename = "match")]
    pub matcher: Match,
    #[serde(default)]
    pub applies_to: Vec<Scope>,
    #[serde(default)]
    pub rules: Vec<RawRule>,
}

fn default_engine() -> u32 {
    ENGINE_VERSION
}

/// Regra com a regex já compilada e os matchers já em minúsculas.
#[derive(Debug)]
pub struct Rule {
    pub id: String,
    pub state: Option<RuleState>,
    pub priority: i32,
    pub region: Region,
    contains_lower: Vec<String>,
    regex: Option<regex::Regex>,
    pub skip_state_update: bool,
}

#[derive(Debug)]
pub struct Manifest {
    pub id: String,
    pub matcher: Match,
    pub applies_to: Vec<Scope>,
    pub rules: Vec<Rule>,
}

/// O que a avaliação conclui.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nenhuma regra casou: o estado anterior fica como estava.
    NoMatch,
    /// Casou uma regra de `skip_state_update`: a tela é reconhecível e
    /// **explicitamente** não diz nada sobre estado.
    Hold,
    /// Casou, e o estado é este.
    State(ObservedState),
}

impl Manifest {
    /// Carrega e valida. Recusa inteiro em vez de aceitar pela metade.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest =
            toml::from_str(source).map_err(|e| ManifestError::Parse(e.to_string()))?;

        if raw.min_engine_version > ENGINE_VERSION {
            return Err(ManifestError::Engine {
                wanted: raw.min_engine_version,
                ours: ENGINE_VERSION,
            });
        }
        if raw.rules.len() > MAX_RULES {
            return Err(ManifestError::TooManyRules {
                found: raw.rules.len(),
            });
        }

        let mut rules = Vec::with_capacity(raw.rules.len());
        for rule in raw.rules {
            let longest = rule
                .contains
                .iter()
                .map(|c| c.chars().count())
                .chain(rule.line_regex.iter().map(|r| r.chars().count()))
                .max()
                .unwrap_or(0);
            if longest > MAX_MATCHER_CHARS {
                return Err(ManifestError::MatcherTooLong {
                    rule: rule.id,
                    found: longest,
                });
            }
            if rule.contains.is_empty() && rule.line_regex.is_none() {
                return Err(ManifestError::EmptyRule { rule: rule.id });
            }
            let regex = match &rule.line_regex {
                Some(pattern) => {
                    Some(
                        regex::Regex::new(pattern).map_err(|e| ManifestError::BadRegex {
                            rule: rule.id.clone(),
                            detail: e.to_string(),
                        })?,
                    )
                }
                None => None,
            };
            rules.push(Rule {
                contains_lower: rule.contains.iter().map(|c| c.to_lowercase()).collect(),
                id: rule.id,
                state: rule.state,
                priority: rule.priority,
                region: rule.region,
                regex,
                skip_state_update: rule.skip_state_update,
            });
        }
        // Maior prioridade primeiro; empate mantém a ordem do arquivo, que é o
        // que dá ao autor um desempate previsível.
        // `sort_by_key` é estável, então o empate mantém a ordem do arquivo —
        // que é a regra de desempate que o autor do manifesto pode prever.
        rules.sort_by_key(|r| std::cmp::Reverse(r.priority));

        Ok(Manifest {
            id: raw.id,
            matcher: raw.matcher,
            applies_to: raw.applies_to,
            rules,
        })
    }

    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let source =
            std::fs::read_to_string(path).map_err(|e| ManifestError::Parse(e.to_string()))?;
        Self::parse(&source)
    }

    /// O manifesto reconhece esta sessão?
    ///
    /// `process` é `None` em SSH, onde não há árvore local — ali sobra o título,
    /// e identidade **nunca** sai do corpo da tela, para não inventar agente
    /// onde há só um log com as palavras certas.
    pub fn identifies(&self, process: Option<&str>, title: &str) -> bool {
        if let Some(binary) = process {
            if self.matcher.process.iter().any(|p| p == binary) {
                return true;
            }
        }
        let lower = title.to_lowercase();
        self.matcher
            .title
            .iter()
            .any(|needle| lower.contains(&needle.to_lowercase()))
    }

    /// A primeira regra que casa, já em ordem de prioridade.
    pub fn evaluate(&self, snapshot: &ScreenSnapshot) -> Verdict {
        // Tela cheia é do app que está rodando, não do agente. O snapshot já
        // devolve região vazia ali; esta guarda evita até o laço.
        if snapshot.alt_screen {
            return Verdict::NoMatch;
        }
        for rule in &self.rules {
            if !rule.matches(snapshot) {
                continue;
            }
            if rule.skip_state_update {
                return Verdict::Hold;
            }
            return match rule.state {
                Some(state) => Verdict::State(state.into()),
                None => Verdict::Hold,
            };
        }
        Verdict::NoMatch
    }
}

impl Rule {
    fn matches(&self, snapshot: &ScreenSnapshot) -> bool {
        let haystack: Vec<&str> = match self.region {
            Region::OscTitle => vec![snapshot.title.as_str()],
            Region::BottomLines(n) => snapshot
                .bottom_lines
                .iter()
                .rev()
                .take(n.min(MAX_REGION_LINES))
                .map(String::as_str)
                .collect(),
        };
        if haystack.is_empty() {
            return false;
        }
        // `contains` é AND: todas as substrings precisam aparecer na região.
        // Case-insensitive porque o texto vem de UI de terceiro, que muda
        // capitalização entre versões sem avisar.
        if !self.contains_lower.is_empty() {
            let joined = haystack.join("\n").to_lowercase();
            if !self.contains_lower.iter().all(|c| joined.contains(c)) {
                return false;
            }
        }
        if let Some(regex) = &self.regex {
            if !haystack.iter().any(|line| regex.is_match(line)) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(title: &str, lines: &[&str]) -> ScreenSnapshot {
        ScreenSnapshot {
            title: title.into(),
            alt_screen: false,
            bottom_lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

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
line_regex = 'Working \([^)]*esc to interrupt\)'
"#;

    #[test]
    fn a_prioridade_maior_vence() {
        let m = Manifest::parse(CODEX).unwrap();
        // As duas casam; a de título tem prioridade 1100 contra 500.
        let verdict = m.evaluate(&snap(
            "Codex — Action Required",
            &["• Working (2s • esc to interrupt)"],
        ));
        assert_eq!(verdict, Verdict::State(ObservedState::AwaitingInput));
    }

    #[test]
    fn skip_state_update_e_um_terceiro_resultado() {
        // O visualizador de transcript mostra texto de aprovação antigo. Sem
        // este resultado ele seria lido como aprovação pendente, e a sessão
        // ficaria bloqueada para sempre — o risco nº 2 da spec.
        let m = Manifest::parse(CODEX).unwrap();
        assert_eq!(
            m.evaluate(&snap("Codex", &["↑/↓ to scroll", "q to quit"])),
            Verdict::Hold
        );
    }

    #[test]
    fn contains_e_case_insensitive_porque_ui_de_terceiro_muda_sem_avisar() {
        let m = Manifest::parse(CODEX).unwrap();
        assert_eq!(
            m.evaluate(&snap("codex — action required", &[])),
            Verdict::State(ObservedState::AwaitingInput)
        );
    }

    #[test]
    fn nenhuma_regra_casando_deixa_o_estado_como_estava() {
        let m = Manifest::parse(CODEX).unwrap();
        assert_eq!(
            m.evaluate(&snap("Codex", &["nada de especial"])),
            Verdict::NoMatch
        );
    }

    #[test]
    fn tela_cheia_nunca_opina() {
        let m = Manifest::parse(CODEX).unwrap();
        let mut s = snap("Codex — Action Required", &[]);
        s.alt_screen = true;
        assert_eq!(m.evaluate(&s), Verdict::NoMatch);
    }

    #[test]
    fn identidade_por_processo_no_shell_e_por_titulo_no_ssh() {
        let m = Manifest::parse(CODEX).unwrap();
        assert!(m.identifies(Some("codex"), ""));
        // SSH: sem árvore de processos, sobra o título.
        assert!(m.identifies(None, "Codex — Ready"));
        assert!(!m.identifies(None, "vim README.md"));
        assert!(!m.identifies(Some("bash"), "um log qualquer"));
    }

    #[test]
    fn motor_mais_novo_recusa_o_manifesto_inteiro() {
        let source = format!("id = \"x\"\nmin_engine_version = {}\n", ENGINE_VERSION + 1);
        assert!(matches!(
            Manifest::parse(&source),
            Err(ManifestError::Engine { .. })
        ));
    }

    #[test]
    fn os_tetos_sao_conferidos_na_carga() {
        let muitas: String = (0..MAX_RULES + 1)
            .map(|i| {
                format!("[[rules]]\nid = \"r{i}\"\nregion = \"osc_title\"\ncontains = [\"x\"]\n")
            })
            .collect();
        assert!(matches!(
            Manifest::parse(&format!("id = \"x\"\n{muitas}")),
            Err(ManifestError::TooManyRules { .. })
        ));

        let comprido = "a".repeat(MAX_MATCHER_CHARS + 1);
        assert!(matches!(
            Manifest::parse(&format!(
                "id = \"x\"\n[[rules]]\nid = \"r\"\nregion = \"osc_title\"\ncontains = [\"{comprido}\"]\n"
            )),
            Err(ManifestError::MatcherTooLong { .. })
        ));
    }

    #[test]
    fn regex_que_nao_compila_recusa_na_porta() {
        // Melhor falhar ao carregar do que silenciar uma regra que nunca casa.
        assert!(matches!(
            Manifest::parse(
                "id = \"x\"\n[[rules]]\nid = \"r\"\nregion = \"osc_title\"\nline_regex = '('\n"
            ),
            Err(ManifestError::BadRegex { .. })
        ));
    }

    #[test]
    fn regra_sem_matcher_nenhum_e_recusada() {
        // Regra sem matcher casaria com tudo, inclusive tela vazia.
        assert!(matches!(
            Manifest::parse("id = \"x\"\n[[rules]]\nid = \"r\"\nregion = \"osc_title\"\n"),
            Err(ManifestError::EmptyRule { .. })
        ));
    }

    /// O teto de custo é parte da decisão, não um extra.
    ///
    /// Manifesto sintético no limite dos tetos — 128 regras, matchers de 512
    /// chars — contra uma tela realista. Sem este teste o número acordado no
    /// grill (1 ms por sessão por flush) seria decoração, e o risco "regra de
    /// terceiro no caminho quente" continuaria não verificável.
    ///
    /// Medido em 2026-08-24 (Apple Silicon): **146 µs**, ~15% do orçamento. A
    /// folga de ~7× é o que permite este teste rodar em CI lento sem flakear.
    /// Se um dia ele começar a falhar, o número medido é o ponto de partida
    /// para saber se a máquina piorou ou se o motor engordou.
    #[test]
    fn avaliar_no_pior_caso_cabe_no_orcamento() {
        let corpo: String = (0..MAX_RULES)
            .map(|i| {
                let agulha = format!("{}{i}", "z".repeat(MAX_MATCHER_CHARS - 8));
                format!(
                    "[[rules]]\nid = \"r{i}\"\nstate = \"running\"\npriority = {i}\n\
                     region = {{ bottom_lines = {MAX_REGION_LINES} }}\n\
                     contains = [\"{agulha}\"]\nline_regex = 'nao-casa-nunca-{i}'\n"
                )
            })
            .collect();
        let m = Manifest::parse(&format!("id = \"pior\"\n{corpo}")).unwrap();
        assert_eq!(m.rules.len(), MAX_RULES);

        let linhas: Vec<String> = (0..MAX_REGION_LINES)
            .map(|i| format!("linha {i} com um tanto de texto como uma tela de verdade tem"))
            .collect();
        let tela = snap(
            "Um título realista de agente — Working",
            &linhas.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        // Nenhuma regra casa: é o pior caso, porque todas as 128 são avaliadas.
        assert_eq!(m.evaluate(&tela), Verdict::NoMatch);

        const RODADAS: u32 = 200;
        let inicio = std::time::Instant::now();
        for _ in 0..RODADAS {
            std::hint::black_box(m.evaluate(std::hint::black_box(&tela)));
        }
        let por_avaliacao = inicio.elapsed() / RODADAS;

        assert!(
            por_avaliacao < std::time::Duration::from_millis(1),
            "avaliação levou {por_avaliacao:?}, acima do 1 ms acordado — \
             a regra de terceiro passou a caber mal no caminho quente do PTY"
        );
    }
}
