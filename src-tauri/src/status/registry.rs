//! De onde os manifestos vêm — e de onde eles **nunca** vêm.
//!
//! Duas origens, nesta ordem: os embutidos no binário (`include_str!`) e os
//! `.toml` do diretório de configuração do app. É o mesmo precedente do
//! [`crate::theme::ThemeManager`], e pela mesma razão: o embutido é o piso que
//! toda instalação tem, e o arquivo do usuário é como se corrige uma regra que
//! envelheceu sem esperar release. Mesmo `id` nas duas origens, o do disco
//! vence.
//!
//! > [!danger] Manifesto nunca vem do repositório aberto.
//! > `.tyba/config.toml` é conteúdo de terceiro que chega por `git pull`. Uma
//! > regra vinda dali seria padrão hostil avaliado no caminho quente do PTY,
//! > por clonar um repositório. O diretório de configuração é do dono da
//! > máquina; o repositório é de quem abriu o PR.
//!
//! Carregado **uma vez**, no boot. Reler o disco a cada avaliação colocaria IO
//! no caminho quente; trocar manifesto exige reabrir o app.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::status::manifest::{Manifest, Scope};

/// Manifestos compilados no binário.
///
/// Vazio na v1 de propósito — a fatia F4 da spec entra exatamente aqui, um
/// `include_str!("../../manifests/<id>.toml")` por agente. A estrutura existe
/// antes do conteúdo para que acrescentar um manifesto seja uma linha, e não
/// uma decisão de arquitetura outra vez.
const BUILTIN: &[&str] = &[
    include_str!("../../manifests/claude-code.toml"),
    include_str!("../../manifests/opencode.toml"),
    include_str!("../../manifests/github-copilot.toml"),
];

/// Teto de tamanho por arquivo. Um manifesto é declaração de algumas dezenas de
/// regras; qualquer coisa desta ordem é engano ou abuso, e o teto do motor
/// (`MAX_RULES`) só é conferido depois do TOML já ter sido parseado inteiro.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Teto de arquivos lidos da pasta. Sem ele, uma pasta com mil `.toml` viraria
/// mil conjuntos de regex compiladas no boot — e o boot é síncrono.
const MAX_MANIFESTS: usize = 64;

/// Os manifestos disponíveis, prontos para identificar uma sessão.
#[derive(Debug, Default)]
pub struct ManifestRegistry {
    /// Por `id`, que é a chave de sobrescrita entre origens. `BTreeMap` porque
    /// a ordem de varredura precisa ser a mesma em toda máquina: quando dois
    /// manifestos reconhecem a mesma sessão, quem vence não pode depender da
    /// ordem em que o sistema de arquivos devolveu os arquivos.
    by_id: BTreeMap<String, Manifest>,
}

impl ManifestRegistry {
    /// Só os embutidos.
    pub fn builtin() -> Self {
        Self::assemble(BUILTIN, None)
    }

    /// Embutidos mais a varredura de `dir`, que é criado se não existir — a
    /// pasta precisa estar lá para o usuário achar onde largar o arquivo.
    pub fn load(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self::assemble(BUILTIN, Some(dir))
    }

    /// Registro montado de fontes em memória. Existe para teste: nenhum caminho
    /// de produção monta manifesto fora das duas origens acima.
    #[cfg(test)]
    pub fn from_sources(sources: &[&str]) -> Self {
        Self::assemble(sources, None)
    }

    /// O corte testável: as duas origens entram por parâmetro.
    fn assemble(builtin: &[&str], dir: Option<&Path>) -> Self {
        let mut by_id = BTreeMap::new();
        for source in builtin {
            match Manifest::parse(source) {
                Ok(manifest) => {
                    by_id.insert(manifest.id.clone(), manifest);
                }
                // Embutido que não carrega é bug nosso, não do usuário — mas
                // derrubar o app por causa disso perderia também os que estão
                // sãos, então segue sem ele.
                Err(err) => eprintln!("tyba: manifesto embutido inválido: {err}"),
            }
        }
        for (path, source) in dir.map(sources_in).unwrap_or_default() {
            match Manifest::parse(&source) {
                Ok(manifest) => {
                    by_id.insert(manifest.id.clone(), manifest);
                }
                Err(err) => eprintln!("tyba: manifesto ignorado ({}): {err}", path.display()),
            }
        }
        Self { by_id }
    }

    /// O manifesto que reconhece esta sessão, se algum reconhece.
    ///
    /// `process` é `None` fora do shell local — em SSH e em container não há
    /// árvore de processos que o TYBA alcance, e ali identidade sai só do
    /// título.
    pub fn identify(&self, scope: Scope, process: Option<&str>, title: &str) -> Option<&Manifest> {
        self.by_id
            .values()
            .find(|manifest| applies(manifest, scope) && manifest.identifies(process, title))
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// `applies_to` vazio vale para todo escopo.
///
/// A alternativa — vazio não valer em lugar nenhum — transformaria um campo
/// esquecido em manifesto que carrega, não reclama e nunca casa: o pior modo de
/// falha possível, porque é silencioso.
fn applies(manifest: &Manifest, scope: Scope) -> bool {
    manifest.applies_to.is_empty() || manifest.applies_to.contains(&scope)
}

/// Os `.toml` legíveis de uma pasta, em ordem de nome.
fn sources_in(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    paths.sort();
    paths.truncate(MAX_MANIFESTS);

    paths
        .into_iter()
        .filter_map(|path| {
            let meta = fs::metadata(&path).ok()?;
            if !meta.is_file() {
                return None;
            }
            if meta.len() > MAX_MANIFEST_BYTES {
                eprintln!(
                    "tyba: manifesto ignorado ({}): passa de {MAX_MANIFEST_BYTES} bytes",
                    path.display()
                );
                return None;
            }
            let source = fs::read_to_string(&path).ok()?;
            Some((path, source))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::session::ObservedState;
    use crate::status::manifest::Verdict;
    use crate::status::screen::ScreenSnapshot;

    fn tela(title: &str) -> ScreenSnapshot {
        ScreenSnapshot {
            title: title.into(),
            alt_screen: false,
            bottom_lines: Vec::new(),
        }
    }

    /// Todo manifesto embutido tem de carregar. Um TOML quebrado no `BUILTIN` é
    /// pulado em silêncio pelo `assemble` — o que é certo para arquivo do
    /// usuário e péssimo para o que nós enviamos: sairia uma release com um
    /// agente que simplesmente não é reconhecido, sem nenhum erro.
    #[test]
    fn todo_manifesto_embutido_carrega() {
        let registry = ManifestRegistry::builtin();
        assert_eq!(
            registry.len(),
            BUILTIN.len(),
            "algum manifesto embutido não passou na validação e foi pulado"
        );
    }

    /// O caso que o tmux do dono cria: com `$TMUX` visível o Claude **trava** o
    /// título em `✳`, então o mesmo glifo aparece parado e trabalhando. Ler `✳`
    /// como ocioso marcaria de parado um agente que está trabalhando —
    /// justamente dentro do tmux, que é o público da feature de SSH.
    #[test]
    fn o_glifo_parado_do_claude_nunca_vira_ocioso() {
        let registry = ManifestRegistry::builtin();
        let m = registry
            .identify(Scope::Ssh, None, "✳ Echo alo")
            .expect("o glifo tem de identificar sozinho");
        assert_eq!(m.id, "claude-code");

        assert_eq!(
            m.evaluate(&tela("✳ Echo alo")),
            Verdict::Hold,
            "`✳` só pode dizer «reconheço o agente e não sei o estado»"
        );
        assert_eq!(m.evaluate(&tela("✻ Cogitated for 5s")), Verdict::Hold);
    }

    #[test]
    fn o_spinner_do_claude_e_o_unico_que_afirma_estado() {
        let registry = ManifestRegistry::builtin();
        let m = registry
            .identify(Scope::Shell, Some("claude"), "")
            .expect("o processo tem de identificar no shell local");
        assert_eq!(
            m.evaluate(&tela("◐ Claude Code")),
            Verdict::State(ObservedState::Running)
        );
        assert_eq!(
            m.evaluate(&tela("◑ Echo alo")),
            Verdict::State(ObservedState::Running)
        );
    }

    /// Medido: depois do primeiro turno o título vira o resumo (`✳ Echo alo`) e
    /// "Claude" some. Identidade por nome de produto deixaria de reconhecer a
    /// sessão exatamente quando ela passa a ter algo a dizer.
    #[test]
    fn a_identidade_do_claude_nao_depende_do_nome_do_produto() {
        let registry = ManifestRegistry::builtin();
        assert!(registry.identify(Scope::Ssh, None, "✳ Echo alo").is_some());
        // E não casa com título comum.
        assert!(registry
            .identify(Scope::Ssh, None, "vim README.md")
            .is_none());
        assert!(registry.identify(Scope::Ssh, None, "~/projeto").is_none());
    }

    /// opencode e Copilot identificam e não dizem estado — é o caso de
    /// "presença sem estado" da spec. Se um dia ganharem regra de estado, este
    /// teste avisa que a decisão mudou.
    #[test]
    fn presenca_sem_estado_nao_afirma_nada() {
        let registry = ManifestRegistry::builtin();
        for (titulo, id) in [
            ("OpenCode", "opencode"),
            ("GitHub Copilot", "github-copilot"),
        ] {
            let m = registry
                .identify(Scope::Ssh, None, titulo)
                .unwrap_or_else(|| panic!("{titulo} tem de identificar"));
            assert_eq!(m.id, id);
            assert_eq!(
                m.evaluate(&tela(titulo)),
                Verdict::NoMatch,
                "{id} não tem sinal de estado e não pode inventar um"
            );
        }
    }

    /// O Codex fica fora de propósito: medido, o título ocioso dele é o basename
    /// do cwd, que não identifica nada. Casá-lo faria um `vim` numa pasta
    /// chamada `codex` virar agente. Decisão de 2026-08-24.
    #[test]
    fn o_codex_nao_e_identificado_por_titulo() {
        let registry = ManifestRegistry::builtin();
        assert!(registry.identify(Scope::Ssh, None, "meu-projeto").is_none());
        assert!(registry.identify(Scope::Ssh, None, "codex").is_none());
    }

    const EMBUTIDO: &str = r#"
id = "codex"
match = { process = ["codex"], title = ["Codex"] }
applies_to = ["shell"]

[[rules]]
id = "embutido"
state = "running"
region = "osc_title"
contains = ["Working"]
"#;

    fn dir_de_teste() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn escreve(dir: &Path, nome: &str, conteudo: &str) {
        fs::write(dir.join(nome), conteudo).unwrap();
    }

    /// A razão de a pasta existir: corrigir uma regra que envelheceu sem
    /// esperar release.
    #[test]
    fn manifesto_do_config_dir_vence_o_embutido_de_mesmo_id() {
        let dir = dir_de_teste();
        escreve(
            &dir,
            "codex.toml",
            r#"
id = "codex"
match = { title = ["Codex"] }
applies_to = ["shell"]

[[rules]]
id = "do_usuario"
state = "awaiting_input"
region = "osc_title"
contains = ["Working"]
"#,
        );

        let registry = ManifestRegistry::assemble(&[EMBUTIDO], Some(&dir));

        assert_eq!(registry.len(), 1, "sobrescrita virou duplicata");
        let manifest = registry
            .identify(Scope::Shell, None, "Codex — Working")
            .unwrap();
        assert_eq!(manifest.rules.len(), 1);
        assert_eq!(
            manifest.rules[0].id, "do_usuario",
            "o embutido continuou valendo depois de ter sido sobrescrito"
        );
    }

    #[test]
    fn manifesto_do_config_dir_com_id_novo_soma_em_vez_de_substituir() {
        let dir = dir_de_teste();
        escreve(
            &dir,
            "gemini.toml",
            r#"
id = "gemini"
match = { title = ["Gemini"] }

[[rules]]
id = "r"
state = "running"
region = "osc_title"
contains = ["Working"]
"#,
        );

        let registry = ManifestRegistry::assemble(&[EMBUTIDO], Some(&dir));

        assert_eq!(registry.len(), 2);
        assert!(registry.identify(Scope::Shell, None, "Gemini").is_some());
        assert!(registry.identify(Scope::Shell, Some("codex"), "").is_some());
    }

    #[test]
    fn arquivo_invalido_nao_derruba_os_validos() {
        // Um manifesto quebrado é o caso comum de quem está escrevendo o
        // próprio: perder os outros junto seria punir o resto da pasta.
        let dir = dir_de_teste();
        escreve(&dir, "quebrado.toml", "isto não é toml de manifesto {{");
        escreve(&dir, "leia-me.txt", "id = \"ignorado\"");

        let registry = ManifestRegistry::assemble(&[EMBUTIDO], Some(&dir));

        assert_eq!(registry.len(), 1);
        assert!(registry.identify(Scope::Shell, Some("codex"), "").is_some());
    }

    #[test]
    fn o_escopo_filtra_antes_da_identidade() {
        // O manifesto do embutido declara `applies_to = ["shell"]`: em SSH ele
        // não opina, mesmo com o título casando.
        let registry = ManifestRegistry::assemble(&[EMBUTIDO], None);

        assert!(registry.identify(Scope::Shell, None, "Codex").is_some());
        assert!(registry.identify(Scope::Ssh, None, "Codex").is_none());
    }

    #[test]
    fn applies_to_vazio_vale_em_todo_escopo() {
        let sem_escopo = r#"
id = "qualquer"
match = { title = ["Agente"] }

[[rules]]
id = "r"
state = "running"
region = "osc_title"
contains = ["Working"]
"#;
        let registry = ManifestRegistry::assemble(&[sem_escopo], None);

        for scope in [Scope::Shell, Scope::Ssh, Scope::Container] {
            assert!(registry.identify(scope, None, "Agente").is_some());
        }
    }

    #[test]
    fn arquivo_grande_demais_nao_e_lido() {
        let dir = dir_de_teste();
        let gordo = format!(
            "id = \"gordo\"\n# {}\n",
            "x".repeat(MAX_MANIFEST_BYTES as usize)
        );
        escreve(&dir, "gordo.toml", &gordo);

        assert!(ManifestRegistry::assemble(&[], Some(&dir)).is_empty());
    }

    /// O teto de origem: uma pasta grande não vira uma carga grande no boot.
    #[test]
    fn a_pasta_para_no_teto_de_arquivos() {
        let dir = dir_de_teste();
        for i in 0..MAX_MANIFESTS + 5 {
            escreve(
                &dir,
                // Zero à esquerda para a ordem de nome ser a ordem numérica —
                // é ela que decide quais entram quando o teto corta.
                &format!("m{i:03}.toml"),
                &format!(
                    "id = \"m{i:03}\"\nmatch = {{ title = [\"m{i:03}\"] }}\n\
                     [[rules]]\nid = \"r\"\nstate = \"running\"\n\
                     region = \"osc_title\"\ncontains = [\"x\"]\n"
                ),
            );
        }

        let registry = ManifestRegistry::assemble(&[], Some(&dir));

        assert_eq!(registry.len(), MAX_MANIFESTS);
        assert!(registry.identify(Scope::Shell, None, "m000").is_some());
        assert!(
            registry
                .identify(Scope::Shell, None, &format!("m{:03}", MAX_MANIFESTS + 4))
                .is_none(),
            "arquivo além do teto entrou assim mesmo"
        );
    }

    #[test]
    fn pasta_inexistente_nao_e_erro() {
        let dir = std::env::temp_dir().join(format!("tyba-nao-existe-{}", uuid::Uuid::new_v4()));
        assert!(ManifestRegistry::assemble(&[EMBUTIDO], Some(&dir)).len() == 1);
    }

    /// Embutido que não compila é bug que só aparece em produção: este teste é
    /// o portão. Vale a partir da F4, e passa de graça enquanto a lista é vazia.
    #[test]
    fn os_embutidos_carregam() {
        assert_eq!(ManifestRegistry::builtin().len(), BUILTIN.len());
    }
}
