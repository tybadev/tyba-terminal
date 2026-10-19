//! Guarda-corpo fixo do agente livre (princípios 4 e 5), na parte que é
//! texto de comando. O push para main/master NÃO é decidido aqui: é o hook
//! `pre-push` do TYBA ([`crate::agent::git_hooks`]), que vê o destino de
//! verdade, venha o push de onde vier. Aqui ficam:
//!
//! - tentativa de desligar aquele hook (`git push --no-verify`,
//!   `-c core.hooksPath=`, mexer em `GIT_CONFIG_*`/`GIT_CONFIG_PARAMETERS`/
//!   `GIT_CONFIG_GLOBAL`, `env -i`, `env -u GIT_CONFIG…`, `unset`);
//! - merge de PR/MR e escrita na branch principal por CLI ou API (`gh pr
//!   merge`, `gh api …/merge`, `curl`), e aprovar PR com a credencial do dono.
//!
//! Multiplataforma: vale para o hook do agente (`tyba _hook --observe`), para
//! o plugin e para as ferramentas de shell do Claude (`Bash`, `PowerShell`) e
//! do Codex. É texto de comando: pega engano, não agente determinado.

use serde_json::{json, Value};

pub const HOOK_BYPASS_MESSAGE: &str = "TYBA: este comando desligaria o hook pre-push do TYBA (--no-verify, core.hooksPath ou as variáveis GIT_CONFIG_*), que recusa push para main/master (guarda-corpo fixo do TYBA). Rode o comando sem isso; o push para a main fica com o humano.";
pub const PUSH_MAIN_MESSAGE: &str = "TYBA: escrita na main/master por agente é recusada (guarda-corpo fixo do TYBA). Trabalhe numa branch própria e abra um PR; o merge na main fica com o humano.";
pub const PR_MERGE_MESSAGE: &str = "TYBA: merge de PR/MR por agente é recusado (guarda-corpo fixo do TYBA). Deixe o PR aberto; quem revisa e faz o merge é o humano.";
pub const PR_APPROVE_MESSAGE: &str = "TYBA: aprovar PR por agente é recusado (guarda-corpo fixo do TYBA): a aprovação sairia com a credencial do dono. Quem revisa e aprova é o humano.";

/// Quantos níveis aninhados (`sh -c '...'`, `eval`, `$(...)`/crase entre
/// aspas ou em heredoc) o guarda-corpo abre.
const MAX_NESTING: usize = 4;

/// O guarda-corpo do `PreToolUse` de uma ferramenta de shell: o motivo da
/// recusa, ou `None` (sem decisão).
///
/// O que ele entende do script:
/// - separadores `&&`, `||`, `;`, `|`, `&`, quebra de linha, parênteses e
///   crase; comentário `#`;
/// - aspas: texto entre aspas é argumento, não comando
///   (`echo "gh pr merge 1"` passa). Entre aspas simples nada roda; entre
///   aspas duplas, `$(...)` e crase rodam e são abertos como script;
/// - redirecionamentos saem do argv junto com o alvo; heredoc não é comando,
///   a não ser que alimente um shell (`bash <<EOF`, `bash <<< '...'`);
/// - antes do programa: atribuições, palavras reservadas e os prefixos
///   `env`, `command`, `exec`, `nohup`, `time`, `timeout`, `nice`, `sudo`,
///   `stdbuf`;
/// - `sh -c`, `bash -c -- '...'`, `fish -c`, `cmd /c`, `pwsh -Command`,
///   `eval` e `$(...)` são abertos, até [`MAX_NESTING`] níveis.
pub fn guard_rail(command: &str) -> Option<&'static str> {
    guard_rail_at(command, 0)
}

fn guard_rail_at(command: &str, depth: usize) -> Option<&'static str> {
    if depth > MAX_NESTING {
        return None;
    }
    let src: Vec<char> = command.chars().collect();
    let pieces = scan(&src, &mut 0, depth, false);
    check(&pieces, depth)
}

fn check(pieces: &[Piece], depth: usize) -> Option<&'static str> {
    if depth > MAX_NESTING {
        return None;
    }
    pieces.iter().find_map(|piece| match piece {
        Piece::Nested(inner) => check(inner, depth + 1),
        Piece::Command(words) => command_verdict(words, depth),
    })
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish"];

/// Quem só lê variável: `echo $GIT_CONFIG_COUNT`, `printenv`, `env | grep`.
const READS_ONLY: &[&str] = &["echo", "printenv", "grep", "egrep", "fgrep", "rg"];

/// Regra simples (decisão do dono, 2026-10-19): comando que menciona
/// `GIT_CONFIG_` (COUNT/KEY/VALUE/PARAMETERS/GLOBAL/NOSYSTEM…) é recusado, a
/// não ser que o programa só leia ([`READS_ONLY`]) e nada antes dele atribua
/// a variável. Pega atribuição, `+=`, `export`, `unset`, `env -u`,
/// `printf -v`, `read`, `declare`/`typeset`/`local`, `mapfile` e o
/// `$env:` do PowerShell sem precisar conhecer cada um. `env -i`/`env -`
/// (limpa o env inteiro, hooksPath junto) também.
fn tampers_git_config_env(words: &[String]) -> bool {
    let mentions = |w: &String| w.to_ascii_uppercase().contains("GIT_CONFIG_");
    let canonical = canonical_words(words);
    let program = canonical
        .first()
        .map(|p| program_name(p))
        .unwrap_or_default();
    let prefix = &words[..words.len() - canonical.len()];
    let cleans_env = words.first().is_some_and(|w| program_name(w) == "env")
        && words
            .iter()
            .any(|w| matches!(w.as_str(), "-i" | "-" | "--ignore-environment"));
    if cleans_env {
        return true;
    }
    if !words.iter().any(mentions) {
        return false;
    }
    !READS_ONLY.contains(&program.as_str()) || prefix.iter().any(mentions)
}

fn command_verdict(words: &[String], depth: usize) -> Option<&'static str> {
    if tampers_git_config_env(words) {
        return Some(HOOK_BYPASS_MESSAGE);
    }
    let canonical = canonical_words(words);
    let (program, args) = canonical.split_first()?;
    match program_name(program).as_str() {
        name if SHELLS.contains(&name) => {
            shell_script(args).and_then(|s| guard_rail_at(s, depth + 1))
        }
        "cmd" => {
            let at = args.iter().position(|a| {
                let a = a.to_ascii_lowercase();
                a == "/c" || a == "/k"
            })?;
            guard_rail_at(&args[at + 1..].join(" "), depth + 1)
        }
        "pwsh" | "powershell" => {
            let at = args.iter().position(|a| {
                matches!(
                    a.to_ascii_lowercase().as_str(),
                    "-c" | "-command" | "/c" | "/command"
                )
            })?;
            guard_rail_at(&args[at + 1..].join(" "), depth + 1)
        }
        "eval" => guard_rail_at(&args.join(" "), depth + 1),
        "git" => git_verdict(args, depth),
        // Plumbing do push: não roda o hook `pre-push`.
        "git-send-pack" => Some(HOOK_BYPASS_MESSAGE),
        "gh" => gh_verdict(args),
        "glab" => {
            let args = without_repo_flag(args);
            if args.first().is_some_and(|a| a == "api") {
                return api_merge_verdict(&args[1..]);
            }
            if subcommand_is(&args, "mr", "approve") {
                return Some(PR_APPROVE_MESSAGE);
            }
            subcommand_is(&args, "mr", "merge").then_some(PR_MERGE_MESSAGE)
        }
        "curl" | "wget" | "http" | "https" | "xh" => api_merge_verdict(args),
        _ => None,
    }
}

/// O nome do programa: o último pedaço do caminho (`/` ou `\`), em
/// minúsculas, sem `.exe`/`.cmd`/`.bat` (`git.exe`, `C:\...\cmd.exe`).
fn program_name(program: &str) -> String {
    let base = program.rsplit(['/', '\\']).next().unwrap_or("");
    let lower = base.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    lower
}

fn basename(program: &str) -> &str {
    std::path::Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
}

/// Um pedaço do script, na ordem em que roda: um comando simples (já sem
/// aspas e sem redirecionamentos), ou o script de um `$(...)`/crase que
/// estava entre aspas duplas ou no corpo de um heredoc.
enum Piece {
    Command(Vec<String>),
    Nested(Vec<Piece>),
}

/// O que a próxima palavra é.
enum Next {
    Arg,
    /// Alvo de redirecionamento: sai do argv.
    Target,
    /// O texto de um `<<<`: script, se o comando for um shell sem `-c`.
    HereString,
    /// Delimitador de heredoc (`<<-` tira os tabs do começo das linhas).
    Delim {
        strip_tabs: bool,
    },
}

/// Um heredoc esperando o corpo, que começa na próxima quebra de linha.
struct Heredoc {
    delim: String,
    strip_tabs: bool,
    /// Delimitador sem aspas: `$(...)` e crase do corpo rodam.
    expands: bool,
}

/// O comando simples sendo montado.
struct Builder {
    words: Vec<String>,
    word: String,
    in_word: bool,
    /// A palavra atual teve aspas ou `\` (heredoc com delimitador assim não
    /// expande o corpo).
    quoted: bool,
    next: Next,
    heredocs: Vec<Heredoc>,
    herestrings: Vec<String>,
}

impl Builder {
    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        match std::mem::replace(&mut self.next, Next::Arg) {
            Next::Arg => self.words.push(word),
            Next::Target => {}
            Next::HereString => self.herestrings.push(word),
            Next::Delim { strip_tabs } => self.heredocs.push(Heredoc {
                delim: word,
                strip_tabs,
                expands: !self.quoted,
            }),
        }
        self.in_word = false;
        self.quoted = false;
    }

    fn end_command(&mut self, pieces: &mut Vec<Piece>, depth: usize) {
        self.end_word();
        self.next = Next::Arg;
        let strings = std::mem::take(&mut self.herestrings);
        if !self.words.is_empty() {
            let words = std::mem::take(&mut self.words);
            // `bash <<< 'git push ...'`: o texto é o script.
            let script_in = reads_script_from_stdin(&words) && depth < MAX_NESTING;
            pieces.push(Piece::Command(words));
            if script_in {
                for text in strings {
                    let src: Vec<char> = text.chars().collect();
                    pieces.push(Piece::Nested(scan(&src, &mut 0, depth + 1, false)));
                }
            }
        }
    }
}

/// Quebra o script em pedaços (ver [`guard_rail`]). Não é um parser de
/// shell — é o bastante para achar o começo de cada comando. Uma passada só
/// por `src`; a recursão (um `$(...)` entre aspas) é limitada por
/// [`MAX_NESTING`]. Com `until_paren`, para no `)` que fecha o `$(` (e o
/// consome).
fn scan(src: &[char], i: &mut usize, depth: usize, until_paren: bool) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut b = Builder {
        words: Vec::new(),
        word: String::new(),
        in_word: false,
        quoted: false,
        next: Next::Arg,
        heredocs: Vec::new(),
        herestrings: Vec::new(),
    };
    let mut parens = 0usize;
    while let Some(&c) = src.get(*i) {
        *i += 1;
        let peek = src.get(*i).copied();
        match c {
            '\'' => {
                b.in_word = true;
                b.quoted = true;
                while let Some(&q) = src.get(*i) {
                    *i += 1;
                    if q == '\'' {
                        break;
                    }
                    b.word.push(q);
                }
            }
            '"' => {
                b.in_word = true;
                b.quoted = true;
                double_quoted(src, i, depth, &mut b.word, &mut pieces);
            }
            '\\' => {
                // `\` + quebra de linha só continua a linha.
                if let Some(n) = peek {
                    *i += 1;
                    if n != '\n' {
                        b.in_word = true;
                        b.quoted = true;
                        b.word.push(n);
                    }
                }
            }
            '#' if !b.in_word => {
                while src.get(*i).is_some_and(|&n| n != '\n') {
                    *i += 1;
                }
            }
            ' ' | '\t' => b.end_word(),
            '\n' => {
                // O comando dono dos heredocs: um shell sem `-c` lê o corpo
                // como script (`bash <<EOF`).
                let owner_is_shell = {
                    let mut words = b.words.clone();
                    if b.in_word {
                        words.push(b.word.clone());
                    }
                    reads_script_from_stdin(&words)
                };
                b.end_command(&mut pieces, depth);
                for doc in std::mem::take(&mut b.heredocs) {
                    let body = heredoc_body(src, i, &doc);
                    if depth >= MAX_NESTING {
                        continue;
                    }
                    if owner_is_shell {
                        pieces.push(Piece::Nested(scan(&body, &mut 0, depth + 1, false)));
                    } else if doc.expands {
                        pieces.extend(substitutions(&body, depth + 1));
                    }
                }
            }
            '&' if peek == Some('>') => {
                // `&>` e `&>>`.
                b.end_word();
                *i += 1;
                if src.get(*i) == Some(&'>') {
                    *i += 1;
                }
                b.next = Next::Target;
            }
            '<' | '>' => {
                // `2>`, `10<`: o número colado é o fd, não argumento.
                if b.in_word && !b.quoted && b.word.chars().all(|d| d.is_ascii_digit()) {
                    b.word.clear();
                    b.in_word = false;
                } else {
                    b.end_word();
                }
                b.next = Next::Target;
                if c == '<' && peek == Some('<') {
                    *i += 1;
                    match src.get(*i) {
                        Some('<') => {
                            *i += 1;
                            b.next = Next::HereString;
                        }
                        next => {
                            let strip_tabs = next == Some(&'-');
                            if strip_tabs {
                                *i += 1;
                            }
                            b.next = Next::Delim { strip_tabs };
                        }
                    }
                } else if matches!(
                    (c, peek),
                    ('>', Some('>' | '|' | '&')) | ('<', Some('&' | '>'))
                ) {
                    *i += 1;
                }
            }
            ')' if until_paren && parens == 0 => break,
            '(' => {
                parens += 1;
                b.end_command(&mut pieces, depth);
            }
            ')' => {
                parens = parens.saturating_sub(1);
                b.end_command(&mut pieces, depth);
            }
            // Crase fora de aspas: o conteúdo roda, e a palavra onde ela está
            // deixa de ser literal (o `` ` `` fica nela de marcador).
            '`' => {
                let inner = backtick_body(src, i);
                if depth < MAX_NESTING {
                    pieces.push(Piece::Nested(scan(&inner, &mut 0, depth + 1, false)));
                }
                b.in_word = true;
                b.word.push('`');
            }
            ';' | '&' | '|' => b.end_command(&mut pieces, depth),
            _ => {
                b.in_word = true;
                b.word.push(c);
            }
        }
    }
    b.end_command(&mut pieces, depth);
    pieces
}

/// O texto de `"..."` vai para a palavra; `$(...)` e crase lá dentro rodam,
/// e viram [`Piece::Nested`]. Para depois da aspa que fecha.
fn double_quoted(
    src: &[char],
    i: &mut usize,
    depth: usize,
    word: &mut String,
    pieces: &mut Vec<Piece>,
) {
    while let Some(&c) = src.get(*i) {
        *i += 1;
        match c {
            '"' => return,
            '\\' => {
                word.extend(src.get(*i));
                *i += 1;
            }
            '$' if src.get(*i) == Some(&'(') && depth < MAX_NESTING => {
                *i += 1;
                pieces.push(Piece::Nested(scan(src, i, depth + 1, true)));
            }
            '`' if depth < MAX_NESTING => {
                let inner = backtick_body(src, i);
                pieces.push(Piece::Nested(scan(&inner, &mut 0, depth + 1, false)));
            }
            _ => word.push(c),
        }
    }
}

/// O conteúdo de uma crase até a que fecha (`` \` `` não fecha).
fn backtick_body(src: &[char], i: &mut usize) -> Vec<char> {
    let mut inner = Vec::new();
    while let Some(&c) = src.get(*i) {
        *i += 1;
        match c {
            '`' => break,
            '\\' => {
                if let Some(&n) = src.get(*i) {
                    *i += 1;
                    if !matches!(n, '`' | '$' | '\\') {
                        inner.push('\\');
                    }
                    inner.push(n);
                }
            }
            _ => inner.push(c),
        }
    }
    inner
}

/// Consome as linhas do corpo até a linha igual ao delimitador (inclusive) e
/// devolve o corpo.
fn heredoc_body(src: &[char], i: &mut usize, doc: &Heredoc) -> Vec<char> {
    let mut body = Vec::new();
    while *i < src.len() {
        let start = *i;
        while src.get(*i).is_some_and(|&c| c != '\n') {
            *i += 1;
        }
        let line = &src[start..*i];
        *i = (*i + 1).min(src.len());
        let line = if doc.strip_tabs {
            let tabs = line.iter().take_while(|&&c| c == '\t').count();
            &line[tabs..]
        } else {
            line
        };
        if line.iter().copied().eq(doc.delim.chars()) {
            break;
        }
        body.extend_from_slice(line);
        body.push('\n');
    }
    body
}

/// Os `$(...)` e crases de um corpo de heredoc sem aspas: aspas ali são
/// texto, só a substituição roda.
fn substitutions(text: &[char], depth: usize) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut i = 0;
    while let Some(&c) = text.get(i) {
        i += 1;
        match c {
            '\\' => i += 1,
            '$' if text.get(i) == Some(&'(') => {
                i += 1;
                pieces.push(Piece::Nested(scan(text, &mut i, depth, true)));
            }
            '`' => {
                let inner = backtick_body(text, &mut i);
                pieces.push(Piece::Nested(scan(&inner, &mut 0, depth, false)));
            }
            _ => {}
        }
    }
    pieces
}

/// Tira o que vem antes do programa de verdade: atribuições de env
/// (`FOO=1 git push`, cf. CVE-2026-48721 do Warp), palavras reservadas e
/// agrupamento (`if`, `then`, `do`, `{`, `!`...) e os prefixos `env`,
/// `command`, `exec`, `nohup`, `time`, `timeout`, `nice`, `sudo`, `stdbuf`
/// com as opções deles.
fn canonical_words(words: &[String]) -> &[String] {
    let is_assignment = |w: &str| {
        w.split_once('=').is_some_and(|(name, _)| {
            name.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    let is_keyword = |w: &str| {
        matches!(
            w,
            "if" | "then"
                | "elif"
                | "else"
                | "fi"
                | "do"
                | "done"
                | "while"
                | "until"
                | "for"
                | "case"
                | "esac"
                | "{"
                | "}"
                | "!"
        )
    };
    let mut rest = words;
    loop {
        let before = rest.len();
        while rest
            .first()
            .is_some_and(|w| is_assignment(w) || is_keyword(w))
        {
            rest = &rest[1..];
        }
        // As opções que levam valor de cada prefixo: letras curtas e longas.
        let options: Option<(&str, &[&str])> = match rest.first().map(|w| basename(w)) {
            Some("command" | "nohup" | "time") => Some(("", &[])),
            Some("env") => Some(("uCS", &["--unset", "--chdir", "--split-string"])),
            Some("exec") => Some(("a", &[])),
            Some("timeout") => Some(("sk", &["--signal", "--kill-after"])),
            Some("nice") => Some(("n", &["--adjustment"])),
            Some("stdbuf") => Some(("ioe", &["--input", "--output", "--error"])),
            Some("sudo") => Some((
                "ughpCDRTUrt",
                &[
                    "--user",
                    "--group",
                    "--host",
                    "--prompt",
                    "--close-from",
                    "--chdir",
                    "--chroot",
                    "--command-timeout",
                    "--other-user",
                    "--role",
                    "--type",
                ],
            )),
            _ => None,
        };
        if let Some((short, long)) = options {
            let wrapper = basename(&rest[0]);
            rest = skip_options(&rest[1..], short, long);
            // `timeout DURAÇÃO comando`.
            if wrapper == "timeout" {
                rest = rest.get(1..).unwrap_or(&[]);
            }
        }
        if rest.len() == before {
            return rest;
        }
    }
}

/// Pula as opções do começo (até `--` ou a primeira palavra sem `-`). Uma
/// opção leva valor na palavra seguinte se é longa e está em `long` (sem
/// `=`), ou se é curta e a primeira letra com valor do grupo é a última
/// (`-u root`, `-Eu root`; em `-uroot` o valor vem colado).
fn skip_options<'a>(mut rest: &'a [String], short: &str, long: &[&str]) -> &'a [String] {
    while let Some(w) = rest.first() {
        if w == "--" {
            return &rest[1..];
        }
        if !w.starts_with('-') {
            break;
        }
        let takes_value = if w.starts_with("--") {
            long.contains(&w.as_str())
        } else {
            let flags: Vec<char> = w.chars().skip(1).collect();
            flags
                .iter()
                .position(|&f| short.contains(f))
                .is_some_and(|p| p + 1 == flags.len())
        };
        rest = rest.get(if takes_value { 2 } else { 1 }..).unwrap_or(&[]);
    }
    rest
}

/// Os argumentos de `gh`/`glab` sem a opção de repositório, onde quer que ela
/// esteja (`-R dono/repo`, `--repo dono/repo`, `--repo=dono/repo`, `-Rdono/repo`).
fn without_repo_flag(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-R" | "--repo" => {
                rest.next();
            }
            _ if a.starts_with("--repo=") || (a.starts_with("-R") && a.len() > 2) => {}
            _ => out.push(a.clone()),
        }
    }
    out
}

/// `gh pr merge`, `glab mr merge`: o grupo e, depois das opções, a ação.
fn subcommand_is(args: &[String], group: &str, action: &str) -> bool {
    args.first().is_some_and(|g| g == group)
        && args[1..]
            .iter()
            .find(|a| !a.starts_with('-'))
            .is_some_and(|a| a == action)
}

/// `gh`: `pr merge` e `pr review --approve` (com `-R` em qualquer lugar) e
/// `gh api` que faz merge ou escreve na branch principal.
fn gh_verdict(args: &[String]) -> Option<&'static str> {
    let args = without_repo_flag(args);
    if subcommand_is(&args, "pr", "merge") {
        return Some(PR_MERGE_MESSAGE);
    }
    if subcommand_is(&args, "pr", "review") && args.iter().any(|a| a == "--approve" || a == "-a") {
        return Some(PR_APPROVE_MESSAGE);
    }
    if args.first().is_some_and(|a| a == "api") {
        return api_merge_verdict(&args[1..]);
    }
    None
}

/// Merge e escrita na branch principal por API (`gh api`, `glab api`, `curl`,
/// `wget`, `http`), só quando a chamada escreve (método diferente de
/// `GET`/`HEAD`, ou com corpo):
/// - merge de PR/MR (`pulls/<n>/merge`, `merge_requests/<n>/merge`);
/// - mover a ref da main (`git/refs/heads/main|master`);
/// - `merges` com base na main;
/// - `contents/…` (escreve na branch padrão);
/// - GraphQL: `mergePullRequest`, `enablePullRequestAutoMerge`,
///   `enqueuePullRequest`, `createCommitOnBranch`, `updateRefs`.
///
/// Barra final no caminho não engana; `GET` (ler o estado do merge) e busca
/// com a palavra na query passam.
fn api_merge_verdict(args: &[String]) -> Option<&'static str> {
    // O método: `-X`/`--method`/`--request`, colado ou não. Sem ele, dado no
    // corpo (`-f`, `-F`, `--field`, `--input`, `-d`, `--data*`, `--json`) faz
    // a chamada escrever.
    let mut method: Option<String> = None;
    let mut body = false;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-X" | "--method" | "--request" => method = rest.next().map(|m| m.to_uppercase()),
            _ if a.starts_with("-X") && a.len() > 2 => method = Some(a[2..].to_uppercase()),
            _ if a.starts_with("--method=") || a.starts_with("--request=") => {
                method = a.split_once('=').map(|(_, m)| m.to_uppercase())
            }
            "-f" | "-F" | "--field" | "--raw-field" | "--input" | "-d" | "--json" => body = true,
            _ if a.starts_with("--data") || a.starts_with("--field=") => body = true,
            _ => {}
        }
    }
    let writes = match method.as_deref() {
        Some("GET") | Some("HEAD") => false,
        Some(_) => true,
        None => body,
    };
    if !writes {
        return None;
    }
    let path = |a: &String| {
        a.split('?')
            .next()
            .unwrap_or(a)
            .trim_end_matches('/')
            .to_string()
    };
    let paths: Vec<String> = args.iter().map(path).collect();
    let graphql = paths
        .iter()
        .any(|p| p == "graphql" || p.ends_with("/graphql"));
    let joined = args.join(" ");
    let mutation = |name: &str| graphql && joined.contains(name);
    if paths
        .iter()
        .any(|p| (p.contains("pulls/") || p.contains("merge_requests/")) && p.ends_with("/merge"))
        || mutation("mergePullRequest")
        || mutation("enablePullRequestAutoMerge")
        || mutation("enqueuePullRequest")
    {
        return Some(PR_MERGE_MESSAGE);
    }
    // Aprovar PR/MR por API.
    let approves = joined.contains("APPROVE");
    let reviews_path = paths
        .iter()
        .any(|p| p.contains("pulls/") && p.ends_with("/reviews"));
    let reviews_events = paths
        .iter()
        .any(|p| p.contains("pulls/") && p.contains("/reviews/") && p.ends_with("/events"));
    let mr_approve = paths
        .iter()
        .any(|p| p.contains("merge_requests/") && p.ends_with("/approve"));
    if ((reviews_path || reviews_events) && approves)
        || mr_approve
        || (mutation("addPullRequestReview") && approves)
        || (mutation("submitPullRequestReview") && approves)
    {
        return Some(PR_APPROVE_MESSAGE);
    }
    let main_ref = paths
        .iter()
        .any(|p| p.ends_with("git/refs/heads/main") || p.ends_with("git/refs/heads/master"));
    let merges_into_main = paths.iter().any(|p| p.ends_with("/merges"))
        && [
            "base=main",
            "base=master",
            "\"base\":\"main\"",
            "\"base\":\"master\"",
        ]
        .iter()
        .any(|b| joined.contains(b));
    let contents = paths
        .iter()
        .any(|p| p.contains("repos/") && p.contains("/contents"));
    (main_ref
        || merges_into_main
        || contents
        || mutation("createCommitOnBranch")
        || mutation("updateRefs"))
    .then_some(PUSH_MAIN_MESSAGE)
}

/// O script de `sh -c '...'` (também `-lc`, `-ec`, `bash -c -- '...'`).
fn shell_script(args: &[String]) -> Option<&str> {
    let flag = args
        .iter()
        .position(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('c'))?;
    let mut at = flag + 1;
    if args.get(at).is_some_and(|a| a == "--") {
        at += 1;
    }
    args.get(at).map(String::as_str)
}

/// Shell que lê o script da entrada (`bash <<EOF`, `sh <<< '...'`): sem `-c`.
fn reads_script_from_stdin(words: &[String]) -> bool {
    let words = canonical_words(words);
    words.split_first().is_some_and(|(program, args)| {
        SHELLS.contains(&program_name(program).as_str()) && shell_script(args).is_none()
    })
}

/// `git [opções globais] <subcomando>`: recusa o que desligaria o hook do
/// TYBA — `--no-verify` no `push` (direto ou por alias) e `core.hooksPath` por
/// `-c`/`--config-env` (ou num alias). `-n` é dry-run, não pula hook.
fn git_verdict(args: &[String], depth: usize) -> Option<&'static str> {
    let mut aliases: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while let Some(a) = args.get(i) {
        let value = |i: usize| args.get(i + 1).map(String::as_str).unwrap_or("");
        match a.as_str() {
            "-c" | "--config-env" => {
                let setting = value(i);
                if setting.to_ascii_lowercase().starts_with("core.hookspath") {
                    return Some(HOOK_BYPASS_MESSAGE);
                }
                if let Some((key, val)) = setting.split_once('=') {
                    if let Some(name) = key.to_ascii_lowercase().strip_prefix("alias.") {
                        aliases.push((name.to_string(), val.to_string()));
                    }
                }
                i += 2;
            }
            _ if a
                .to_ascii_lowercase()
                .starts_with("--config-env=core.hookspath") =>
            {
                return Some(HOOK_BYPASS_MESSAGE);
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" => i += 2,
            _ if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let sub = args.get(i)?;
    let rest = &args[i + 1..];
    // `--no-verify` e qualquer prefixo dele que o git aceita (`--no-veri`…;
    // `--no-ver` é ambíguo com `--no-verbose` e o git recusa).
    let skips_hook =
        |a: &String| a.len() >= "--no-veri".len() && "--no-verify".starts_with(a.as_str());
    let pushes_without_hook = |args: &[String]| args.iter().any(skips_hook);
    if sub == "push" {
        return pushes_without_hook(rest).then_some(HOOK_BYPASS_MESSAGE);
    }
    // `git send-pack` empurra sem passar pelo `pre-push`.
    if sub == "send-pack" {
        return Some(HOOK_BYPASS_MESSAGE);
    }
    // Alias: o nome do git ignora maiúsculas.
    let lower = sub.to_ascii_lowercase();
    let (_, alias) = aliases.iter().rev().find(|(name, _)| *name == lower)?;
    if alias.to_ascii_lowercase().contains("hookspath") {
        return Some(HOOK_BYPASS_MESSAGE);
    }
    if let Some(script) = alias.strip_prefix('!') {
        let mut line = script.to_string();
        for a in rest {
            line.push(' ');
            line.push_str(a);
        }
        return guard_rail_at(&line, depth + 1);
    }
    let mut words: Vec<String> = alias.split_whitespace().map(str::to_string).collect();
    words.extend(rest.iter().cloned());
    git_verdict(&words, depth + 1)
}

/// Ferramentas de shell do Claude Code e do Codex: o guarda-corpo vale para
/// todas (`PowerShell` no Windows).
const SHELL_TOOLS: &[&str] = &[
    "Bash",
    "PowerShell",
    "shell",
    "local_shell",
    "exec_command",
    "unified_exec",
];

/// O comando de uma ferramenta de shell: texto (`command`/`cmd`) ou argv
/// (lista), que vira texto com cada palavra entre aspas simples.
fn tool_command(input: &Value) -> Option<String> {
    let field = input.get("command").or_else(|| input.get("cmd"))?;
    if let Some(text) = field.as_str() {
        return Some(text.to_string());
    }
    let words = field.as_array()?;
    let quoted: Vec<String> = words
        .iter()
        .map(|w| {
            w.as_str()
                .map(|w| format!("'{}'", w.replace('\'', "'\\''")))
        })
        .collect::<Option<_>>()?;
    Some(quoted.join(" "))
}

/// O que o hook imprime no stdout. Só o `PreToolUse` de uma ferramenta de
/// shell que bate no guarda-corpo tem algo a dizer (`deny` com motivo);
/// qualquer outro caso é `None` — stdout vazio e saída 0, que para o Claude
/// Code e o Codex é "sem decisão": o fluxo de permissão normal deles segue.
pub fn hook_output(payload: &Value) -> Option<String> {
    let field = |k: &str| payload.get(k).and_then(Value::as_str);
    if field("hook_event_name")? != "PreToolUse" || !SHELL_TOOLS.contains(&field("tool_name")?) {
        return None;
    }
    let mut command = tool_command(payload.get("tool_input")?)?;
    // No PowerShell `\` é caractere de caminho, não escape.
    if field("tool_name") == Some("PowerShell") {
        command = command.replace('\\', "/");
    }
    let reason = guard_rail(&command)?;
    Some(
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        })
        .to_string(),
    )
}

/// O guarda-corpo que o hook do agente (`tyba _hook --observe`) e o plugin
/// aplicam.
pub fn guard_rail_output(payload: &Value) -> Option<String> {
    hook_output(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(command: &str) -> Option<&'static str> {
        guard_rail(command)
    }

    /// O push para a main é do hook; o texto só recusa desligar o hook.
    #[test]
    fn push_destinations_are_left_to_the_hook() {
        for cmd in [
            "git push origin main",
            "git push",
            "git push origin $b",
            "cd $X && git push",
            "git -c push.default=matching push",
            "git push -n origin main",
            "git push --dry-run origin main",
            "git commit --no-verify -m x",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn turning_the_hook_off_is_refused() {
        for cmd in [
            "git push --no-verify origin feat",
            "git push origin feat --no-verify",
            "git -c core.hooksPath=/dev/null push",
            "git -c core.hookspath= push origin main",
            "git -c CORE.HOOKSPATH=x push",
            "git --config-env=core.hooksPath=HP push",
            "git -c alias.p='push --no-verify' p origin main",
            "git -c alias.p='-c core.hooksPath= push' p",
            "git -c alias.x='!git push --no-verify' x",
            "git -c alias.P=push P --no-verify",
            "GIT_CONFIG_COUNT=0 git push origin main",
            "GIT_CONFIG_PARAMETERS= git push",
            "GIT_CONFIG_GLOBAL=/tmp/x git push",
            "env -u GIT_CONFIG_COUNT git push",
            "env -uGIT_CONFIG_COUNT git push",
            "env --unset=GIT_CONFIG_COUNT git push",
            "env -i git push origin main",
            "env - PATH=/bin git push",
            "unset GIT_CONFIG_COUNT; git push",
            "export GIT_CONFIG_COUNT=0",
            "GIT_CONFIG_KEY_0=x",
            "sh -c 'git push --no-verify'",
            "bash -c -- 'unset GIT_CONFIG_COUNT && git push'",
            "cmd /c git push --no-verify",
            "pwsh -Command \"$env:GIT_CONFIG_COUNT=0; git push\"",
            "Remove-Item Env:GIT_CONFIG_COUNT",
            "bash <<EOF\ngit push --no-verify\nEOF",
            "bash <<< 'git push --no-verify'",
        ] {
            assert_eq!(guard(cmd), Some(HOOK_BYPASS_MESSAGE), "{cmd}");
        }
        for cmd in [
            "echo GIT_CONFIG_COUNT",
            "printenv GIT_CONFIG_COUNT",
            "echo \"git push --no-verify\"",
            "git commit -m 'git push --no-verify'",
            "git config --get core.hooksPath",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn plumbing_abbreviations_and_git_config_writes_are_refused() {
        for cmd in [
            "git send-pack origin refs/heads/main",
            "git -C x send-pack ../r main",
            "git-send-pack ../r main",
            "git push --no-veri origin main",
            "git push --no-verif origin main",
            "git push origin main --no-verify",
            "printf -v GIT_CONFIG_COUNT 0; git push",
            "read GIT_CONFIG_COUNT <<< 0",
            "GIT_CONFIG_COUNT+=1 git push",
            "declare GIT_CONFIG_COUNT=0",
            "typeset -x GIT_CONFIG_COUNT=0",
            "local GIT_CONFIG_PARAMETERS=x",
            "mapfile -t GIT_CONFIG_KEY_0 < f",
            "export GIT_CONFIG_NOSYSTEM=1",
            "echo GIT_CONFIG_COUNT=0 | GIT_CONFIG_COUNT=0 grep x",
            "GIT_CONFIG_COUNT=0 echo ok",
        ] {
            assert_eq!(guard(cmd), Some(HOOK_BYPASS_MESSAGE), "{cmd}");
        }
        for cmd in [
            "echo $GIT_CONFIG_COUNT",
            "printenv GIT_CONFIG_KEY_0",
            "env | grep GIT_CONFIG_",
            "git push --no-ver origin feat",
            "git push --verbose origin feat",
            "git commit --no-verify -m x",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn pr_approval_through_the_api_is_refused() {
        for cmd in [
            "gh api -X POST repos/o/r/pulls/3/reviews -f event=APPROVE",
            "gh api repos/o/r/pulls/3/reviews -f event=APPROVE -f body=ok",
            "gh api repos/o/r/pulls/3/reviews/9/events -f event=APPROVE",
            "gh api graphql -f query='mutation { addPullRequestReview(input:{pullRequestId:\"x\", event: APPROVE}) { clientMutationId } }'",
            "gh api graphql -f query='mutation { submitPullRequestReview(input:{event: APPROVE}) { clientMutationId } }'",
            "curl -X POST https://api.github.com/repos/o/r/pulls/3/reviews -d '{\"event\":\"APPROVE\"}'",
            "glab mr approve 7",
            "glab -R o/r mr approve 7",
            "glab api -X POST projects/1/merge_requests/7/approve",
            "curl -X POST https://gitlab.com/api/v4/projects/1/merge_requests/7/approve",
        ] {
            assert_eq!(guard(cmd), Some(PR_APPROVE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh api repos/o/r/pulls/3/reviews",
            "gh api -X POST repos/o/r/pulls/3/reviews -f event=COMMENT -f body=x",
            "gh api -X POST repos/o/r/pulls/3/reviews -f event=REQUEST_CHANGES -f body=x",
            "glab mr view 7",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn pr_merge_and_approve_are_refused() {
        for cmd in [
            "gh pr merge",
            "gh pr merge 42 --squash",
            "FOO=1 gh pr merge 42",
            "gh pr -R o/r merge 3",
            "gh -R o/r pr merge 1",
            "gh --repo=o/r pr merge 1",
            "glab mr merge 7",
            "glab -R o/r mr merge 1",
            "sh -c 'glab mr merge 7'",
            "cmd /c gh pr merge 3",
        ] {
            assert_eq!(guard(cmd), Some(PR_MERGE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh pr review 3 --approve",
            "gh pr review --approve -b ok",
            "gh pr review -R o/r 3 -a",
        ] {
            assert_eq!(guard(cmd), Some(PR_APPROVE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh pr view 42",
            "gh pr create --base main",
            "gh pr review 3 --comment -b 'ver x'",
            "gh pr review 3 --request-changes -b 'falta teste'",
            "glab mr view 7",
            "echo 'gh pr merge 42'",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn merges_and_main_writes_through_the_api_are_refused() {
        for cmd in [
            "gh api -X PUT repos/o/r/pulls/12/merge",
            "gh api -X PUT repos/o/r/pulls/12/merge/",
            "gh api repos/o/r/pulls/12/merge -X PUT",
            "gh api --method PUT /repos/o/r/pulls/12/merge",
            "gh api graphql -f query='mutation { mergePullRequest(input:{pullRequestId:\"x\"}) { clientMutationId } }'",
            "gh api graphql -f query='mutation { enablePullRequestAutoMerge(input:{}) { clientMutationId } }'",
            "gh api graphql -f query='mutation { enqueuePullRequest(input:{}) { clientMutationId } }'",
            "curl -X PUT -H 'Authorization: token x' https://api.github.com/repos/o/r/pulls/12/merge",
            "curl https://gitlab.com/api/v4/projects/1/merge_requests/7/merge -X PUT",
            "glab api -X PUT projects/1/merge_requests/7/merge",
        ] {
            assert_eq!(guard(cmd), Some(PR_MERGE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh api -X PATCH repos/o/r/git/refs/heads/main -f sha=abc",
            "gh api repos/o/r/git/refs/heads/master/ -f sha=abc -F force=true",
            "curl -X PATCH https://api.github.com/repos/o/r/git/refs/heads/main -d '{\"sha\":\"x\"}'",
            "gh api repos/o/r/merges -f base=main -f head=feat",
            "gh api -X PUT repos/o/r/contents/README.md -f message=x -f content=eA==",
            "gh api graphql -f query='mutation { createCommitOnBranch(input:{}) { commit { oid } } }'",
            "gh api graphql -f query='mutation { updateRefs(input:{}) { clientMutationId } }'",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh api repos/o/r/pulls/12",
            "gh api repos/o/r/pulls/12/merge",
            "gh api -X GET repos/o/r/pulls/12/merge",
            "curl https://api.github.com/repos/o/r/pulls/12/merge",
            "gh api repos/o/r/git/refs/heads/main",
            "gh api repos/o/r/contents/README.md",
            "gh api repos/o/r/merges -f base=feat -f head=x",
            "gh api 'search/issues?q=mergePullRequest'",
            "gh api search/code -f q=createCommitOnBranch -X GET",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn opens_shells_wrappers_and_windows_programs() {
        for cmd in [
            "sudo -u root gh pr merge 1",
            "timeout 9 nice -n 1 env FOO=1 gh pr merge 1",
            "if true; then gh pr merge 1; fi",
            "echo \"$(gh pr merge 1)\"",
            "echo `gh pr merge 1`",
            "eval gh pr merge 1",
            "'C:\\Git\\bin\\gh.exe' pr merge 1",
            "powershell -Command \"gh pr merge 1\"",
            "fish -c 'gh pr merge 1'",
            "sh <<'EOF'\ngh pr merge 1\nEOF",
        ] {
            assert_eq!(guard(cmd), Some(PR_MERGE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "cat <<EOF\ngh pr merge 1\nEOF",
            "git commit -F - <<'EOF'\nrecusa gh pr merge\nEOF",
            "echo '$(gh pr merge 1)'",
            "",
        ] {
            assert_eq!(guard(cmd), None, "{cmd:?}");
        }
        // Além de MAX_NESTING níveis, sem decisão (e sem estourar a pilha).
        let deep = format!(
            "{}gh pr merge 1{}",
            "echo \"$(".repeat(MAX_NESTING + 1),
            ")\"".repeat(MAX_NESTING + 1)
        );
        assert_eq!(guard(&deep), None);
        let huge = "\"$(".repeat(100_000);
        assert_eq!(guard(&huge), None);
    }

    fn pre_tool_use(tool: &str, input: Value) -> Value {
        json!({
            "hook_event_name": "PreToolUse",
            "session_id": "00000000-0000-0000-0000-000000000000",
            "cwd": "/repo/feat",
            "tool_name": tool,
            "tool_input": input,
        })
    }

    #[test]
    fn hook_output_denies_only_the_guard_rail_and_is_silent_otherwise() {
        let deny =
            hook_output(&pre_tool_use("Bash", json!({"command": "gh pr merge 1"}))).expect("nega");
        let deny: Value = serde_json::from_str(&deny).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            deny["hookSpecificOutput"]["permissionDecisionReason"],
            PR_MERGE_MESSAGE
        );
        for (tool, input) in [
            ("PowerShell", json!({"command": "git push --no-verify"})),
            (
                "shell",
                json!({"command": ["bash", "-lc", "git push --no-verify"]}),
            ),
            (
                "local_shell",
                json!({"command": ["git", "push", "--no-verify"]}),
            ),
            ("exec_command", json!({"cmd": "git push --no-verify"})),
        ] {
            let out = hook_output(&pre_tool_use(tool, input));
            assert!(
                out.is_some_and(|o| o.contains(HOOK_BYPASS_MESSAGE)),
                "{tool}"
            );
        }
        for payload in [
            pre_tool_use(
                "Write",
                json!({"file_path": "/repo/feat/a.rs", "content": "x"}),
            ),
            pre_tool_use("Bash", json!({"command": "git push origin main"})),
            pre_tool_use("Bash", json!({"command": "rm -rf ~/x"})),
            pre_tool_use("Task", json!({"command": "gh pr merge 1"})),
            pre_tool_use("Bash", json!({})),
        ] {
            assert_eq!(hook_output(&payload), None, "{payload}");
        }
        for event in ["PermissionRequest", "SessionStart", "Notification", "Stop"] {
            let mut payload = pre_tool_use("Bash", json!({"command": "gh pr merge 1"}));
            payload["hook_event_name"] = json!(event);
            assert_eq!(hook_output(&payload), None, "{event}");
        }
        assert_eq!(hook_output(&Value::Null), None);
    }
}
