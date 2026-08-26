//! Os comandos que existem na máquina — a fonte do PRIMEIRO token.
//!
//! O resto da completação sabe continuar uma linha: `complete_path` completa o
//! caminho, `argument` completa subcomando e flag, o histórico completa a linha
//! inteira. Nenhum deles responde à primeira palavra vinda de fora do histórico,
//! e `next_tokens` recusa prefixo vazio de propósito. Numa máquina onde `pnpm`
//! nunca foi digitado, `pn` devolvia silêncio.
//!
//! Aqui a fonte é o disco: os diretórios do `$PATH`, lidos pelo core. Alias e
//! função NÃO passam por aqui — não existem em disco, e quem os conhece é o
//! shell (ver a tech spec 02 no cofre).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// O separador do `$PATH`: `:` em toda parte, `;` no Windows.
#[cfg(windows)]
const SEPARATOR: char = ';';
#[cfg(not(windows))]
const SEPARATOR: char = ':';

/// Os diretórios de um `$PATH` cru, na ordem em que ele os declara.
///
/// Entrada vazia é descartada em vez de virar `.`: o shell interpreta `::` e o
/// `$PATH` terminado em `:` como "o diretório atual", e herdar isso poria o cwd
/// na varredura. Um `./deploy.sh` no repo apareceria como comando do sistema, e
/// entrar num diretório mudaria a lista de comandos disponíveis.
pub fn split(raw: &str) -> Vec<PathBuf> {
    raw.split(SEPARATOR)
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Quando o diretório mudou pela última vez, ou `None` se ele não pode ser lido.
///
/// `None` é estado legítimo e não erro: `~/.cargo/bin` antes do primeiro
/// install. E ele COMPARA — um diretório que passa a existir muda de `None`
/// para `Some`, e é assim que o cache descobre que ganhou entradas novas.
fn stamp(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).ok()?.modified().ok()
}

/// A varredura e, junto dela, o que a invalida.
///
/// O cache é por sessão e não por tecla: `/usr/bin` tem mais de mil entradas e
/// varrer o `$PATH` inteiro a cada caractere digitado é o tipo de custo que o
/// core não pode pagar — ele divide CPU com os agentes.
///
/// Guardar o `$PATH` e o carimbo de cada diretório junto dos nomes é o que
/// permite a invalidação ser uma pergunta barata: comparar uma string e fazer
/// um `stat` por diretório, em vez de revarrer para descobrir se mudou.
pub struct Cache {
    raw_path: String,
    stamps: Vec<Option<SystemTime>>,
    names: Vec<String>,
}

impl Cache {
    /// Varre o `$PATH` e carimba os diretórios no mesmo instante.
    pub fn build(raw_path: &str) -> Self {
        let dirs = split(raw_path);
        // O carimbo é tirado ANTES da varredura, de propósito. Tirado depois,
        // uma escrita que acontecesse durante a leitura ficaria com o carimbo
        // novo e o conteúdo velho — e o cache nunca mais se veria desatualizado.
        // Na ordem inversa o pior caso é uma revarredura a mais.
        let stamps = dirs.iter().map(|dir| stamp(dir)).collect();
        let refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        Self {
            raw_path: raw_path.to_string(),
            stamps,
            names: scan(&refs),
        }
    }

    /// Precisa revarrer?
    ///
    /// Pergunta pura e barata de propósito: uma comparação de string e um
    /// `stat` por diretório. É o que permite chamá-la a cada prompt sem que
    /// varrer volte a custar.
    pub fn is_stale(&self, raw_path: &str) -> bool {
        // O `$PATH` primeiro, e não por gosto: quando ele muda, os carimbos
        // guardados são de OUTROS diretórios e comparar um a um não diz nada.
        if self.raw_path != raw_path {
            return true;
        }
        let dirs = split(raw_path);
        // Comprimento diferente com a mesma string não deveria acontecer — mas
        // `zip` para no mais curto, e um dia em que aconteça o cache passaria a
        // ignorar em silêncio a cauda dos diretórios.
        if dirs.len() != self.stamps.len() {
            return true;
        }
        dirs.iter()
            .zip(&self.stamps)
            .any(|(dir, before)| stamp(dir) != *before)
    }

    /// Os nomes varridos, na ordem do `$PATH`.
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// De onde o nome veio. O `$PATH` o core lê sozinho; o resto só o shell sabe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Path,
    Alias,
    Function,
    Builtin,
}

/// Um comando que existe na sessão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub kind: Kind,
}

/// Um lote de nomes vindo do hook, em `<kind>:<nome>,<kind>:<nome>`.
///
/// **O conteúdo é influenciável por atacante**: qualquer processo dentro da
/// sessão emite a sequência, exatamente como qualquer um emite o OSC 7 do `cwd`
/// (ADR de 2026-07-08). Vale a mesma regra — display-only, nenhuma decisão de
/// segurança sai daqui — e por isso cada nome é conferido: completar escreve
/// texto na caixa, e um "nome" que fosse `rm -rf /` viraria uma linha pronta
/// esperando um Enter distraído.
///
/// Item inválido é descartado sozinho, sem levar o lote junto: um payload
/// meio-truncado ainda entrega os nomes que chegaram inteiros.
pub fn parse_batch(payload: &str) -> Vec<Command> {
    payload
        .split(',')
        .filter_map(|item| {
            let (kind, name) = item.split_once(':')?;
            let kind = match kind {
                "alias" => Kind::Alias,
                "function" => Kind::Function,
                "builtin" => Kind::Builtin,
                // `path` não vem do shell: o core lê o `$PATH` sozinho, e aceitar
                // esse kind daqui deixaria um processo qualquer plantar nomes
                // como se fossem binários instalados.
                _ => return None,
            };
            // Função com `_` é widget de completação do zsh (`_git`, `_docker`),
            // não comando: 1025 das 1302 funções na máquina do dono, medido em
            // 26/08/2026. Ninguém as digita, e oferecê-las encheria a lista com
            // o motor da completação alheia. O corte é só em FUNÇÃO — alias e
            // builtin com `_` são raros e não têm essa convenção.
            if kind == Kind::Function && name.starts_with('_') {
                return None;
            }
            is_name(name).then(|| Command {
                name: name.to_string(),
                kind,
            })
        })
        .collect()
}

/// Quantos bytes um nome pode ter.
///
/// `NAME_MAX` da maioria dos filesystems é 255, e nome de comando não chega
/// perto. O teto existe para que um lote forjado não encha a lista com uma
/// entrada só, gigante.
const MAX_NAME_LEN: usize = 128;

/// Isto se parece com um nome de comando?
///
/// A regra é por lista de PERMISSÃO, não de proibição. Enumerar o proibido
/// (`;`, `|`, `&`, `$`, aspas, espaço, controle) erra por omissão no dia em que
/// aparecer um metacaractere que ninguém lembrou; enumerar o permitido erra por
/// recusar um nome exótico, que é o lado barato.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '@'))
}

/// Ordena os candidatos: o que o dono já usou primeiro, o resto em ordem
/// estável.
///
/// Fica separado de `matching` de propósito. Coletar é decidir QUEM entra —
/// depende do disco e do shell. Ordenar é decidir EM QUE ORDEM — depende do
/// histórico, que o `completion` não conhece e não deve conhecer. Juntas, seriam
/// uma função que precisa de `Store` para testar qual binário é executável.
///
/// `used` chega ordenado por frecência (a de `history::frecency`, que já pesa
/// recência, frequência, sucesso e escopo); esta função só preserva essa ordem.
pub fn rank(mut found: Vec<Command>, used: &[String]) -> Vec<Command> {
    let position = |name: &str| used.iter().position(|u| u == name);
    // `sort_by` do Rust é estável, mas o desempate é declarado mesmo assim: a
    // entrada vem do `read_dir`, cuja ordem é do filesystem — muda entre
    // máquinas e pode mudar na mesma máquina entre duas leituras. Depender da
    // estabilidade seria preservar uma ordem que não existe.
    found.sort_by(|a, b| match (position(&a.name), position(&b.name)) {
        (Some(x), Some(y)) => x.cmp(&y),
        // Usado vence não-usado, e não é preferência: o que ele roda todo dia é
        // a resposta provável, e o resto está ali só para não sumir.
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    });
    found
}

/// O que cada sessão contou, vivo enquanto ela viver.
///
/// Global e não campo de `SessionManager` pelo mesmo motivo que
/// `history::record` é: quem produz o dado é a captura do PTY, que roda numa
/// thread própria e não tem — nem deve ter — referência ao gerenciador.
static REPORTED: std::sync::LazyLock<parking_lot::Mutex<HashMap<String, Vec<Command>>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

/// Absorve um lote reportado pelo hook daquela sessão.
///
/// União com dedup: o hook emite nos dois primeiros prompts (não há canal de
/// volta para confirmar), e sem isto o segundo envio duplicaria a lista.
pub fn absorb_reported(session_id: &str, payload: &str) {
    let mut all = REPORTED.lock();
    let known = all.entry(session_id.to_string()).or_default();
    for command in parse_batch(payload) {
        if known.iter().any(|seen| seen.name == command.name) {
            continue;
        }
        known.push(command);
    }
}

/// O que o shell daquela sessão contou até agora.
pub fn reported(session_id: &str) -> Vec<Command> {
    REPORTED.lock().get(session_id).cloned().unwrap_or_default()
}

/// A sessão morreu; o que ela contou morre junto.
///
/// Sem isto o mapa cresce por toda sessão aberta na vida do app — e alias de um
/// worktree fechado continuaria sendo sugerido em outro.
pub fn forget_reported(session_id: &str) {
    REPORTED.lock().remove(session_id);
}

/// Tudo que existe na sessão: o `$PATH` que o core varreu e o que o shell contou.
///
/// As duas fontes vivem juntas aqui porque a pergunta do usuário é uma só —
/// "que comando começa com `pn`?" — mas envelhecem de formas diferentes: o
/// `$PATH` por carimbo de diretório, o do shell por lote novo. Separá-las em
/// dois lugares faria a completação ter de perguntar duas vezes.
pub struct Known {
    path: Cache,
    from_shell: Vec<Command>,
}

impl Known {
    pub fn new(raw_path: &str) -> Self {
        Self {
            path: Cache::build(raw_path),
            from_shell: Vec::new(),
        }
    }

    /// Absorve um lote vindo do hook.
    ///
    /// União, nunca substituição: o payload vem partido em lotes porque não cabe
    /// num OSC só, e um lote que sobrescrevesse o anterior faria a lista mostrar
    /// apenas o último pedaço — falha pior que estourar o teto, porque é
    /// silenciosa.
    pub fn absorb(&mut self, payload: &str) {
        for command in parse_batch(payload) {
            // Reenvio acontece: o hook reemite quando o core não confirmou até o
            // segundo prompt. Sem esta guarda, cada reenvio duplicaria a lista.
            if self.from_shell.iter().any(|seen| seen.name == command.name) {
                continue;
            }
            self.from_shell.push(command);
        }
    }

    /// O que o shell contou, na ordem em que chegou.
    pub fn from_shell(&self) -> &[Command] {
        &self.from_shell
    }

    /// Os comandos que começam com o prefixo, das duas fontes.
    ///
    /// O que veio do shell vem primeiro: alias e função são o que o dono
    /// escreveu, e valem mais que um binário que veio junto de um pacote.
    pub fn matching(&self, prefix: &str) -> Vec<Command> {
        // Prefixo vazio casa com tudo no `starts_with`: seriam os milhares de
        // binários do `$PATH` despejados na caixa no instante em que o cursor
        // chega na linha. Isso não é sugestão, é um dump — e é a mesma decisão
        // que `next_tokens` já toma para argumento.
        if prefix.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<Command> = self
            .from_shell
            .iter()
            .filter(|c| c.name.starts_with(prefix))
            .cloned()
            .collect();
        for name in self.path.names() {
            if !name.starts_with(prefix) {
                continue;
            }
            // O shell vence: quem tem `alias ls='eza'` quer ver o alias, não o
            // `/bin/ls` que ele mascarou.
            if found.iter().any(|seen| &seen.name == name) {
                continue;
            }
            found.push(Command {
                name: name.clone(),
                kind: Kind::Path,
            });
        }
        found
    }
}

/// O arquivo pode ser executado por alguém?
///
/// Basta um dos três bits: um binário instalado por outro usuário costuma vir
/// `0o755`, e exigir o bit do dono esconderia metade do `/usr/bin`. Quem não
/// puder executar descobre no `exec`, não na sugestão — errar para o lado de
/// mostrar é melhor do que sumir com um comando que existe.
#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

/// No Windows não há bit de execução: quem decide é a extensão.
#[cfg(not(unix))]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    meta.is_file()
}

/// Nomes de executável encontrados nos diretórios, sem repetir.
pub fn scan(dirs: &[&Path]) -> Vec<String> {
    let mut found = Vec::new();
    // A ordem dos diretórios é a do `$PATH`, e ela é significativa: o primeiro
    // que declara um nome é o que o shell vai executar. Como o que se completa é
    // só o NOME, quem vence não muda o texto — mas manter "o primeiro vence" faz
    // a lista contar a mesma história que o `exec` conta.
    let mut seen: HashSet<String> = HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // `metadata()` segue link simbólico de propósito: metade do
            // `/usr/local/bin` de um Mac com Homebrew é link para a Cellar, e
            // olhar o link em vez do alvo esconderia tudo isso.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !is_executable(&meta) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if seen.insert(name.clone()) {
                found.push(name);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Cria um arquivo com o modo pedido — `0o755` executável, `0o644` não.
    fn file(dir: &Path, name: &str, mode: u32) {
        let path = dir.join(name);
        std::fs::write(&path, "").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn finds_the_executable_and_ignores_what_cannot_run() {
        // O `$PATH` tem README, LICENSE e afins em diretório de pacote. Listá-los
        // como comando é oferecer algo que não roda.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tyba", 0o755);
        file(dir.path(), "README.md", 0o644);

        assert_eq!(scan(&[dir.path()]), vec!["tyba"]);
    }

    #[test]
    fn the_same_name_in_two_directories_is_listed_once() {
        // É o caso comum, não a exceção: `python3` existe no shim do asdf e no
        // `/usr/bin`, `node` no Homebrew e no nvm. Listar duas vezes mostraria
        // uma lista com o mesmo comando repetido, sem nada que os distinga —
        // porque o que se completa é o NOME, e ele é o mesmo.
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        file(first.path(), "node", 0o755);
        file(second.path(), "node", 0o755);
        file(second.path(), "deno", 0o755);

        let mut found = scan(&[first.path(), second.path()]);
        found.sort();
        assert_eq!(found, vec!["deno", "node"]);
    }

    #[test]
    fn a_directory_that_cannot_be_read_does_not_take_the_rest_down() {
        // `$PATH` com entrada morta é o normal, não o defeito: `~/.cargo/bin`
        // antes do primeiro `cargo install`, um volume desmontado, um shim de
        // ferramenta desinstalada. Uma varredura que aborta ali devolve lista
        // vazia e o usuário perde a completação inteira por causa de um
        // diretório que nunca importou.
        let good = tempfile::tempdir().unwrap();
        file(good.path(), "tyba", 0o755);
        let missing = good.path().join("nao-existe");

        // O ilegível vem PRIMEIRO: se ele abortasse, o bom nunca seria lido.
        assert_eq!(scan(&[&missing, good.path()]), vec!["tyba"]);
    }

    #[test]
    fn the_empty_entry_never_becomes_the_current_directory() {
        // `PATH=/a::/b` e `PATH=/a:` são o shell dizendo "e também o cwd".
        // Herdar isso poria o diretório atual na varredura: o `./deploy.sh` de
        // um repo apareceria como comando do sistema, e trocar de pasta mudaria
        // a lista de comandos. O `$PATH` real vem do env de uma sessão — não é
        // conteúdo que o TYBA escreve.
        assert_eq!(
            split("/usr/bin::/bin:"),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }

    #[test]
    fn the_cache_answers_with_what_it_scanned() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tyba", 0o755);

        let cache = Cache::build(dir.path().to_str().unwrap());
        assert_eq!(cache.names(), ["tyba"]);
    }

    #[test]
    fn a_fresh_cache_is_not_stale() {
        // Sem este caso o cache que sempre se diz velho passaria em todos os
        // outros: revarrer a cada tecla está correto e é exatamente o custo que
        // o cache existe para não pagar.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tyba", 0o755);
        let raw = dir.path().to_str().unwrap();

        assert!(!Cache::build(raw).is_stale(raw));
    }

    #[test]
    fn a_changed_path_makes_the_cache_stale() {
        // O `$PATH` muda dentro da sessão, não só entre elas: `nvm use`,
        // `direnv`, ativar um venv. Os diretórios antigos podem continuar
        // intactos — quem mudou foi a LISTA, e nenhum carimbo denuncia isso.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tyba", 0o755);
        let raw = dir.path().to_str().unwrap();
        let cache = Cache::build(raw);

        assert!(cache.is_stale(&format!("{raw}:/opt/novo/bin")));
    }

    #[test]
    fn a_binary_that_appears_makes_the_cache_stale() {
        // `cargo install`, `brew install`, `npm i -g` no meio da sessão. Sem
        // isto o comando recém-instalado só apareceria no próximo boot do app —
        // e "instalei e o terminal não vê" é a forma mais visível de o cache
        // estar mentindo.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tyba", 0o755);
        let raw = dir.path().to_str().unwrap();
        let cache = Cache::build(raw);

        file(dir.path(), "recem-instalado", 0o755);
        assert!(cache.is_stale(raw));
    }

    #[test]
    fn a_batch_carries_where_each_name_came_from() {
        // O `kind` não é enfeite: é o que deixa a lista distinguir o que é do
        // dono (`gst`, que ele mesmo escreveu) do que é do sistema.
        assert_eq!(
            parse_batch("alias:gst,function:mkcd,builtin:cd"),
            vec![
                Command {
                    name: "gst".into(),
                    kind: Kind::Alias
                },
                Command {
                    name: "mkcd".into(),
                    kind: Kind::Function
                },
                Command {
                    name: "cd".into(),
                    kind: Kind::Builtin
                },
            ]
        );
    }

    #[test]
    fn two_batches_add_up_instead_of_replacing_each_other() {
        // O payload vem em LOTES porque não cabe num OSC só (1302 funções na
        // máquina do dono contra um teto de 8 KB). Se o segundo substituísse o
        // primeiro, o teto deixaria de estourar e a lista passaria a mostrar só
        // o último pedaço — falha pior, porque é silenciosa.
        let mut known = Known::new("");
        known.absorb("alias:gst,alias:gco");
        known.absorb("function:mkcd");

        let names: Vec<&str> = known.from_shell().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["gst", "gco", "mkcd"]);
    }

    #[test]
    fn a_completion_widget_is_not_a_command() {
        // Medido na máquina do dono em 26/08/2026: 1302 funções no zsh, e 1025
        // delas começam com `_`. São os widgets que completam OUTROS comandos
        // (`_git`, `_docker`) — ninguém os digita. Oferecê-los seria encher a
        // lista com o motor da completação alheia, e é o que faz o payload
        // passar de 16 KB contra um teto de 8.
        let mut known = Known::new("");
        known.absorb("function:_git,function:mkcd,function:_docker");

        let names: Vec<&str> = known.from_shell().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["mkcd"]);
    }

    #[test]
    fn a_forged_name_never_becomes_a_suggestion() {
        // A sequência é emitida por qualquer processo da sessão, como o OSC 7 do
        // cwd. Completar escreve texto na caixa: um "nome" com espaço ou `;`
        // viraria uma linha pronta esperando um Enter distraído.
        let mut known = Known::new("");
        known.absorb("alias:rm -rf /,alias:x;curl evil,alias:$(id),alias:ok");

        let names: Vec<&str> = known.from_shell().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["ok"]);
    }

    fn cmd(name: &str) -> Command {
        Command {
            name: name.into(),
            kind: Kind::Path,
        }
    }

    fn names(found: &[Command]) -> Vec<&str> {
        found.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn what_was_used_comes_before_what_was_never_used() {
        // Sem isto a lista é o `read_dir` do `/usr/bin`: `pnpm` que o dono roda
        // dez vezes por dia apareceria atrás de qualquer binário que veio junto
        // de um pacote. A frecência já existe e já pesa recência, frequência,
        // sucesso e escopo — aqui ela só é respeitada.
        let found = vec![cmd("curl"), cmd("pnpm"), cmd("awk")];
        let used = vec!["pnpm".to_string()];

        assert_eq!(names(&rank(found, &used)), ["pnpm", "awk", "curl"]);
    }

    #[test]
    fn two_never_used_tie_break_alphabetically_not_by_read_dir() {
        // A ordem do `read_dir` é do filesystem: muda entre máquinas, e pode
        // mudar na mesma máquina entre duas leituras. Uma lista que se reordena
        // sozinha entre teclas é pior que uma lista errada — o item que o olho
        // mirou sai do lugar antes do dedo chegar.
        let found = vec![cmd("zoxide"), cmd("bat"), cmd("rg")];

        assert_eq!(names(&rank(found, &[])), ["bat", "rg", "zoxide"]);
    }

    #[test]
    fn what_a_session_reported_dies_with_it() {
        // O registro é global e vive fora do `SessionManager` (a captura roda em
        // thread própria e não tem referência a ele). Sem a limpeza no fim, o
        // mapa cresce por toda sessão da vida do app — e o alias de um worktree
        // fechado seguiria sendo sugerido em outro.
        absorb_reported("sessao-a", "alias:gst");
        absorb_reported("sessao-b", "alias:outro");
        // Reenvio: o hook fala nos DOIS primeiros prompts, e sem dedup a lista
        // dobraria.
        absorb_reported("sessao-a", "alias:gst");

        assert_eq!(reported("sessao-a").len(), 1);
        forget_reported("sessao-a");
        assert!(reported("sessao-a").is_empty());
        // Fechar uma não pode levar a vizinha junto.
        assert_eq!(reported("sessao-b").len(), 1);
        forget_reported("sessao-b");
    }

    #[test]
    fn matching_answers_from_both_sources_at_once() {
        // A pergunta do usuário é uma só — "que comando começa com `pn`?" — e
        // ele não sabe nem se importa se a resposta veio do disco ou do shell.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "pnpm", 0o755);
        file(dir.path(), "curl", 0o755);
        let mut known = Known::new(dir.path().to_str().unwrap());
        known.absorb("alias:pn,alias:gst");

        let found = known.matching("pn");
        let mut names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["pn", "pnpm"]);
    }

    #[test]
    fn an_empty_prefix_suggests_nothing() {
        // Com `starts_with`, prefixo vazio casa com TUDO: seriam os milhares de
        // binários do `$PATH` despejados na caixa no instante em que o cursor
        // chega na linha. Isso não é sugestão, é um dump — e é a mesma decisão
        // que `next_tokens` já toma para argumento.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "pnpm", 0o755);
        let mut known = Known::new(dir.path().to_str().unwrap());
        known.absorb("alias:gst");

        assert!(known.matching("").is_empty());
    }

    #[test]
    fn the_shell_wins_over_the_binary_it_masks() {
        // Quem tem `alias ls='eza'` quer ver o alias. O binário mascarado
        // aparecendo junto seria a lista mostrando duas vezes um nome que só
        // pode significar uma coisa naquela sessão.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "ls", 0o755);
        let mut known = Known::new(dir.path().to_str().unwrap());
        known.absorb("alias:ls");

        let found = known.matching("ls");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Alias);
    }
}
