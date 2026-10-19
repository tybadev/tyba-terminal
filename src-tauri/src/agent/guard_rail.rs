//! Guarda-corpo fixo do agente livre (princípios 4 e 5): `git push` para
//! main/master e merge de PR/MR por agente são recusados, com motivo em texto
//! para o Claude. Multiplataforma: o mesmo parser vale para o hook do claude
//! hospedado (`tyba _hook --observe`), para o plugin e para a sessão de agente
//! do Windows (o `Bash` do Claude Code no Windows é o Git Bash).
//!
//! É texto de comando: pega engano, não agente determinado (script, alias
//! contornam). Fronteira de verdade não mora aqui.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const PUSH_MAIN_MESSAGE: &str = "TYBA: push para main/master por agente é recusado (guarda-corpo fixo do TYBA). Faça o push numa branch própria e abra um PR; o merge na main fica com o humano.";
pub const BRANCH_UNKNOWN_MESSAGE: &str = "TYBA: não deu para ler a branch atual, e um push sem destino explícito pode ir para a main (guarda-corpo fixo do TYBA). Diga o destino: `git push origin <sua-branch>`.";
pub const PR_MERGE_MESSAGE: &str = "TYBA: merge de PR/MR por agente é recusado (guarda-corpo fixo do TYBA). Deixe o PR aberto; quem revisa e faz o merge é o humano.";

/// Quantos níveis aninhados (`sh -c '...'`, `eval`, `$(...)`/crase entre
/// aspas ou em heredoc) o guarda-corpo abre.
const MAX_NESTING: usize = 4;

/// O guarda-corpo do `PreToolUse` do `Bash`: o motivo da recusa, ou `None`
/// (sem decisão). `cwd` é o da sessão; `current_branch` só é chamado para
/// `git push` que empurra a branch atual (sem refspec, `HEAD` ou `@`).
///
/// O que ele entende do script:
/// - separadores `&&`, `||`, `;`, `|`, `&`, quebra de linha, parênteses e
///   crase; comentário `#`;
/// - aspas: texto entre aspas é argumento, não comando
///   (`echo "git push origin main"` passa). Entre aspas simples nada roda;
///   entre aspas duplas, `$(...)` e crase rodam e são abertos como script;
/// - redirecionamentos (`>`, `>>`, `<`, `<<<`, `&>`, `&>>`, `>&`, `<&`, `>|`,
///   `<>`, com fd na frente, colado ou não) saem do argv junto com o alvo;
/// - heredoc (`<<FIM`, `<<-FIM`, `<<'FIM'`, `<<"FIM"`, `<<\FIM`): o corpo não
///   é comando. Com delimitador sem aspas, `$(...)` e crase do corpo rodam e
///   são abertos;
/// - antes do programa: atribuições (`FOO=1`), palavras reservadas e
///   agrupamento (`if`, `then`, `do`, `{`, `!`...) e os prefixos `env`,
///   `command`, `exec`, `nohup`, `time`, `timeout`, `nice`, `sudo`,
///   `stdbuf` com as opções deles;
/// - `sh -c '...'`, `eval` e `$(...)` são abertos, até [`MAX_NESTING`]
///   níveis — além disso, sem decisão;
/// - `cd DIR` muda a pasta em que os comandos seguintes do mesmo script leem
///   a branch. `cd` sem pasta, `cd -`, `cd ~...` e `cd $X` voltam para a pasta
///   de partida (a da sessão): o guarda-corpo não sabe aonde eles vão.
///
/// Limites conhecidos (engano pega; agente determinado contorna): alias,
/// função, script em arquivo, `xargs`, mais de [`MAX_NESTING`] níveis, `cd`
/// dentro de subshell vazando para fora dele, `case` dentro de `$(...)`
/// entre aspas, e `<<` em aritmética lido como heredoc.
pub fn guard_rail(
    command: &str,
    cwd: &Path,
    current_branch: &dyn Fn(&Path) -> Option<String>,
) -> Option<&'static str> {
    guard_rail_at(command, cwd, current_branch, 0)
}

fn guard_rail_at(
    command: &str,
    cwd: &Path,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    guard_rail_in(command, &mut Some(cwd.to_path_buf()), current_branch, depth)
}

/// Como [`guard_rail_at`], a partir de uma pasta que pode ser desconhecida
/// (`None`: um `cd` para caminho não literal).
fn guard_rail_in(
    command: &str,
    dir: &mut Option<PathBuf>,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    if depth > MAX_NESTING {
        return None;
    }
    let src: Vec<char> = command.chars().collect();
    let pieces = scan(&src, &mut 0, depth, false);
    check(&pieces, dir, current_branch, depth)
}

/// Avalia os pedaços em ordem. `dir` é a pasta corrente do script (muda com
/// `cd`/`pushd`); `None` quando ela deixou de ser conhecida.
fn check(
    pieces: &[Piece],
    dir: &mut Option<PathBuf>,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    if depth > MAX_NESTING {
        return None;
    }
    for piece in pieces {
        let verdict = match piece {
            // Substituição roda num subshell: `cd` lá dentro não vaza.
            Piece::Nested(inner) => check(inner, &mut dir.clone(), current_branch, depth + 1),
            Piece::Command(words) => command_verdict(words, dir, current_branch, depth),
        };
        if verdict.is_some() {
            return verdict;
        }
    }
    None
}

/// Palavra que é um caminho literal: sem expansão de shell (`$`, crase, `~`,
/// chaves, curingas) e sem ser opção. Aspas já saíram no scan, então `'$x'`
/// também conta como não literal — na dúvida, desconhecido.
fn is_literal(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with('-')
        && !word.contains(['$', '`', '~', '{', '}', '*', '?', '[', ']', '%'])
}

/// O caminho de `target` a partir de `dir`; `None` se ele não é literal ou se
/// a pasta de partida já é desconhecida (e o alvo é relativo). No Windows,
/// `/c/Users/...` do Git Bash vira `C:\Users\...`.
fn resolve(dir: &Option<PathBuf>, target: &str) -> Option<PathBuf> {
    if !is_literal(target) {
        return None;
    }
    let path = git_bash_path(target);
    if path.is_absolute() {
        Some(path)
    } else {
        dir.as_ref().map(|d| d.join(path))
    }
}

#[cfg(windows)]
fn git_bash_path(target: &str) -> PathBuf {
    let bytes = target.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && (bytes.len() == 2 || bytes[2] == b'/')
    {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        let rest = target.get(3..).unwrap_or("").replace('/', "\\");
        return PathBuf::from(format!("{drive}:\\{rest}"));
    }
    PathBuf::from(target)
}

#[cfg(not(windows))]
fn git_bash_path(target: &str) -> PathBuf {
    PathBuf::from(target)
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "fish"];

fn command_verdict(
    words: &[String],
    dir: &mut Option<PathBuf>,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    let canonical = canonical_words(words);
    // `GIT_DIR=x git push`: a branch é a do repositório apontado.
    let mut here = dir.clone();
    for word in &words[..words.len() - canonical.len()] {
        if let Some(v) = word.strip_prefix("GIT_DIR=") {
            here = resolve(dir, v);
        } else if let Some(v) = word.strip_prefix("GIT_WORK_TREE=") {
            if !is_literal(v) {
                here = None;
            }
        }
    }
    let (program, args) = canonical.split_first()?;
    match program_name(program).as_str() {
        "cd" | "pushd" => {
            let target = args.iter().find(|a| !a.starts_with('-') || *a == "-");
            *dir = target.and_then(|t| resolve(dir, t));
            None
        }
        "popd" => {
            *dir = None;
            None
        }
        "export" => {
            for a in args {
                if let Some(v) = a.strip_prefix("GIT_DIR=") {
                    *dir = resolve(dir, v);
                } else if let Some(v) = a.strip_prefix("GIT_WORK_TREE=") {
                    if !is_literal(v) {
                        *dir = None;
                    }
                }
            }
            None
        }
        name if SHELLS.contains(&name) => {
            shell_script(args).and_then(|s| guard_rail_in(s, &mut here, current_branch, depth + 1))
        }
        "cmd" => {
            let at = args.iter().position(|a| {
                let a = a.to_ascii_lowercase();
                a == "/c" || a == "/k"
            })?;
            guard_rail_in(
                &args[at + 1..].join(" "),
                &mut here,
                current_branch,
                depth + 1,
            )
        }
        "pwsh" | "powershell" => {
            let at = args.iter().position(|a| {
                matches!(
                    a.to_ascii_lowercase().as_str(),
                    "-c" | "-command" | "/c" | "/command"
                )
            })?;
            guard_rail_in(
                &args[at + 1..].join(" "),
                &mut here,
                current_branch,
                depth + 1,
            )
        }
        "eval" => guard_rail_in(&args.join(" "), &mut here, current_branch, depth + 1),
        "git" => git_verdict(args, &here, current_branch, depth),
        "gh" => gh_verdict(args),
        "glab" => {
            let args = without_repo_flag(args);
            if args.first().is_some_and(|a| a == "api") {
                return api_merge_verdict(&args[1..]);
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
    Path::new(program)
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

/// `gh`: `pr merge` (com `-R` em qualquer lugar) e `gh api` que faz merge.
fn gh_verdict(args: &[String]) -> Option<&'static str> {
    let args = without_repo_flag(args);
    if subcommand_is(&args, "pr", "merge") {
        return Some(PR_MERGE_MESSAGE);
    }
    if args.first().is_some_and(|a| a == "api") {
        return api_merge_verdict(&args[1..]);
    }
    None
}

/// Merge e escrita na main por API (`gh api`, `curl`, `wget`, `http`): merge
/// de PR/MR (`pulls/<n>/merge`, `merge_requests/<n>/merge`, a mutation
/// `mergePullRequest`/`enablePullRequestAutoMerge`), mover a ref da main
/// (`git/refs/heads/main` sem ser `GET`) e `merges` com base na main.
fn api_merge_verdict(args: &[String]) -> Option<&'static str> {
    let joined = args.join(" ");
    let is_merge_path = |a: &str| {
        let a = a.split('?').next().unwrap_or(a);
        (a.contains("pulls/") || a.contains("merge_requests/")) && a.ends_with("/merge")
    };
    if args.iter().any(|a| is_merge_path(a))
        || joined.contains("mergePullRequest")
        || joined.contains("enablePullRequestAutoMerge")
    {
        return Some(PR_MERGE_MESSAGE);
    }
    // O método: `-X`/`--method`/`--request`, colado ou não. Sem ele, dado no
    // corpo (`-f`, `-F`, `--field`, `--input`, `-d`, `--data*`, `--json`) faz
    // a chamada escrever.
    let mut method: Option<String> = None;
    let mut writes = false;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "-X" | "--method" | "--request" => method = rest.next().map(|m| m.to_uppercase()),
            _ if a.starts_with("-X") && a.len() > 2 => method = Some(a[2..].to_uppercase()),
            _ if a.starts_with("--method=") || a.starts_with("--request=") => {
                method = a.split_once('=').map(|(_, m)| m.to_uppercase())
            }
            "-f" | "-F" | "--field" | "--raw-field" | "--input" | "-d" | "--json" => writes = true,
            _ if a.starts_with("--data") || a.starts_with("--field=") => writes = true,
            _ => {}
        }
    }
    let writes = match method.as_deref() {
        Some("GET") | Some("HEAD") => false,
        Some(_) => true,
        None => writes,
    };
    if !writes {
        return None;
    }
    let main_ref = args.iter().any(|a| {
        let a = a.trim_end_matches('/');
        a.ends_with("git/refs/heads/main") || a.ends_with("git/refs/heads/master")
    });
    let merges_into_main = args
        .iter()
        .any(|a| a.trim_end_matches('/').ends_with("/merges"))
        && [
            "base=main",
            "base=master",
            "\"base\":\"main\"",
            "\"base\":\"master\"",
        ]
        .iter()
        .any(|b| joined.contains(b));
    (main_ref || merges_into_main).then_some(PUSH_MAIN_MESSAGE)
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

fn is_main_ref(name: &str) -> bool {
    let name = name
        .strip_prefix("refs/heads/")
        .or_else(|| name.strip_prefix("heads/"))
        .unwrap_or(name);
    matches!(name, "main" | "master")
}

/// `git [opções globais] <subcomando>`: só o `push` (direto ou por alias
/// `-c alias.x=push`) importa. `-C`, `--git-dir` e `--work-tree` mudam onde a
/// branch é lida; caminho não literal a torna desconhecida.
fn git_verdict(
    args: &[String],
    dir: &Option<PathBuf>,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    let mut here = dir.clone();
    let mut matching = false;
    let mut aliases: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while let Some(a) = args.get(i) {
        let value = |i: usize| args.get(i + 1).map(String::as_str).unwrap_or("");
        match a.as_str() {
            "-C" => {
                here = resolve(&here, value(i));
                i += 2;
            }
            "-c" => {
                if let Some((key, val)) = value(i).split_once('=') {
                    let key = key.to_ascii_lowercase();
                    if key == "push.default" && val.eq_ignore_ascii_case("matching") {
                        matching = true;
                    } else if let Some(name) = key.strip_prefix("alias.") {
                        aliases.push((name.to_string(), val.to_string()));
                    }
                }
                i += 2;
            }
            "--git-dir" => {
                here = resolve(dir, value(i));
                i += 2;
            }
            "--work-tree" => {
                if !is_literal(value(i)) {
                    here = None;
                }
                i += 2;
            }
            _ if a.starts_with("--git-dir=") => {
                here = resolve(dir, &a["--git-dir=".len()..]);
                i += 1;
            }
            _ if a.starts_with("--work-tree=") => {
                if !is_literal(&a["--work-tree=".len()..]) {
                    here = None;
                }
                i += 1;
            }
            "--namespace" | "--config-env" | "--exec-path" => i += 2,
            _ if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let sub = args.get(i)?;
    let rest = &args[i + 1..];
    if sub == "push" {
        return push_verdict(rest, &here, matching, current_branch);
    }
    let (_, alias) = aliases.iter().rev().find(|(name, _)| name == sub)?;
    if let Some(script) = alias.strip_prefix('!') {
        let mut line = script.to_string();
        for a in rest {
            line.push(' ');
            line.push_str(a);
        }
        return guard_rail_in(&line, &mut here, current_branch, depth + 1);
    }
    let mut words: Vec<String> = alias.split_whitespace().map(str::to_string).collect();
    if words.first().map(String::as_str) != Some("push") {
        return None;
    }
    words.remove(0);
    words.extend(rest.iter().cloned());
    push_verdict(&words, &here, matching, current_branch)
}

/// O `git push`: destino main/master por refspec explícita (`main`, `+main`,
/// `HEAD:main`, `heads/main`, `refs/heads/main`), `--all`/`--mirror`, `:`
/// (matching), curinga, destino com expansão de shell, `push.default=matching`
/// sem refspec, ou sem refspec (ou `HEAD`/`@`) com a branch atual sendo main.
/// Não empurram branch nenhuma: `--dry-run`/`-n`, e `--tags` sem refspec.
///
/// Falha FECHADO: sem refspec (ou com `HEAD`/`@`) e a branch atual ilegível
/// (git que não responde, HEAD destacado, pasta sem repo ou desconhecida),
/// recusa com [`BRANCH_UNKNOWN_MESSAGE`] — o destino pode ser a main.
fn push_verdict(
    args: &[String],
    dir: &Option<PathBuf>,
    matching: bool,
    current_branch: &dyn Fn(&Path) -> Option<String>,
) -> Option<&'static str> {
    let mut positional = Vec::new();
    let (mut all, mut tags, mut dry_run) = (false, false, false);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--all" | "--mirror" | "--branches" => all = true,
            "--tags" => tags = true,
            "--dry-run" | "-n" => dry_run = true,
            "--repo" | "-o" | "--push-option" | "--receive-pack" | "--exec" => {
                rest.next();
            }
            "--" => positional.extend(rest.by_ref()),
            _ if a.starts_with('-') => {}
            _ => positional.push(a),
        }
    }
    if dry_run {
        return None;
    }
    if all {
        return Some(PUSH_MAIN_MESSAGE);
    }
    let current = || match dir.as_deref().and_then(current_branch) {
        Some(b) if is_main_ref(&b) => Some(PUSH_MAIN_MESSAGE),
        Some(_) => None,
        None => Some(BRANCH_UNKNOWN_MESSAGE),
    };
    let refspecs = positional.get(1..).unwrap_or(&[]);
    if refspecs.is_empty() {
        if tags {
            return None;
        }
        if matching {
            return Some(PUSH_MAIN_MESSAGE);
        }
        return current();
    }
    refspecs.iter().find_map(|spec| {
        let spec = spec.trim_start_matches('+');
        // Expansão, curinga ou `:` (matching): o destino pode ser a main.
        if !is_literal(spec) && spec != ":" && !spec.starts_with(':') {
            return Some(PUSH_MAIN_MESSAGE);
        }
        let dst = spec.rsplit_once(':').map_or(spec, |(_, d)| d);
        if dst.is_empty() || !is_literal(dst) {
            Some(PUSH_MAIN_MESSAGE)
        } else if dst == "HEAD" || dst == "@" {
            current()
        } else {
            is_main_ref(dst).then_some(PUSH_MAIN_MESSAGE)
        }
    })
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
pub fn hook_output(
    payload: &Value,
    current_branch: &dyn Fn(&Path) -> Option<String>,
) -> Option<String> {
    let field = |k: &str| payload.get(k).and_then(Value::as_str);
    if field("hook_event_name")? != "PreToolUse" || !SHELL_TOOLS.contains(&field("tool_name")?) {
        return None;
    }
    let mut command = tool_command(payload.get("tool_input")?)?;
    // No PowerShell `\` é caractere de caminho, não escape.
    if field("tool_name") == Some("PowerShell") {
        command = command.replace('\\', "/");
    }
    let cwd = PathBuf::from(field("cwd").filter(|c| !c.is_empty()).unwrap_or("."));
    let reason = guard_rail(&command, &cwd, current_branch)?;
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

/// [`hook_output`] com a branch lida do repositório de verdade: o
/// guarda-corpo que o hook do agente (`tyba _hook --observe`) e o plugin
/// aplicam.
pub fn guard_rail_output(payload: &Value) -> Option<String> {
    hook_output(payload, &current_branch)
}

/// A branch atual de `dir`, lida do `HEAD` do repositório — sem rodar git
/// no caso comum: nenhum filtro, `fsmonitor` ou jaula no caminho, e o mesmo
/// código no Windows. Sobe as pastas até achar o repositório: a própria pasta
/// se ela é um git dir (`--git-dir`, `GIT_DIR`, repo bare), ou o `.git`
/// (pasta, ou o arquivo `gitdir:` de um worktree). Também responde numa
/// branch ainda sem commit. Repositório reftable (HEAD `.invalid`, ou
/// `extensions.refStorage` no config) pergunta ao git
/// (`symbolic-ref --short -q HEAD`). `None` (sem repo, HEAD destacado,
/// leitura falhou) faz o push que depende dela ser recusado.
fn current_branch(dir: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(here) = at {
        let dotgit = here.join(".git");
        let git_dir = if here.join("HEAD").is_file() && here.join("objects").is_dir() {
            here.to_path_buf()
        } else if dotgit.is_dir() {
            dotgit
        } else if dotgit.is_file() {
            let text = std::fs::read_to_string(&dotgit).ok()?;
            let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
            if target.is_absolute() {
                target
            } else {
                here.join(target)
            }
        } else {
            at = here.parent();
            continue;
        };
        let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
        let branch = head.trim().strip_prefix("ref: refs/heads/")?;
        if branch == ".invalid" || uses_reftable(&git_dir) {
            return symbolic_ref(here);
        }
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    None
}

/// O repositório guarda as refs em reftable: o `HEAD` em arquivo é só um
/// marcador. Olha o config do git dir e, num worktree, o do repo principal.
fn uses_reftable(git_dir: &Path) -> bool {
    let mut dirs = vec![git_dir.to_path_buf()];
    if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
        let common = PathBuf::from(common.trim());
        dirs.push(if common.is_absolute() {
            common
        } else {
            git_dir.join(common)
        });
    }
    dirs.iter().any(|d| {
        std::fs::read_to_string(d.join("config"))
            .is_ok_and(|c| c.to_ascii_lowercase().contains("refstorage"))
    })
}

/// A branch pelo git (reftable), sem jaula aninhada e com as defesas de
/// [`crate::worktree::git_in`]: fsmonitor, hooks e pager desligados, env de
/// git do processo limpo. Falhou: `None` — recusa.
fn symbolic_ref(dir: &Path) -> Option<String> {
    let out = crate::worktree::git_in_unjailed(dir)
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .output()
        .ok()?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !branch.is_empty()).then_some(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Branch atual falsa: `main` em qualquer pasta com "main" no nome (com
    /// `..` resolvido), `feat/x` no resto.
    fn fake_branch(dir: &Path) -> Option<String> {
        let mut resolved = PathBuf::new();
        for part in dir.components() {
            match part {
                std::path::Component::ParentDir => {
                    resolved.pop();
                }
                other => resolved.push(other),
            }
        }
        Some(if resolved.to_string_lossy().contains("main") {
            "main".into()
        } else {
            "feat/x".into()
        })
    }

    fn guard(command: &str) -> Option<&'static str> {
        guard_rail(command, Path::new("/repo/feat"), &fake_branch)
    }

    #[test]
    fn guard_rail_denies_push_to_main_or_master() {
        for cmd in [
            "git push origin main",
            "git push origin master",
            "git push origin HEAD:main",
            "git push -f origin master",
            "git push --force-with-lease origin main",
            "git push origin +main",
            "git push origin x:refs/heads/main",
            "git push origin feature:main",
            "git push origin :main",
            "git push -u origin feat/x main",
            "git push --all origin",
            "git push origin -- main",
            "/usr/bin/git push origin main",
            "git -C /outro push origin main",
            "git -c push.default=current push origin main",
            "FOO=1 git push origin main",
            "FOO=1 BAR='a b' git push origin main",
            "env FOO=1 git push origin main",
            "env -i git push origin main",
            "command git push origin main",
            "exec git push origin main",
            "cargo test && git push origin main",
            "cargo test; git push origin main",
            "make || git push origin main",
            "echo ok | git push origin main",
            "true\ngit push origin main",
            "sh -c 'git push origin main'",
            "bash -lc \"cd x && git push origin main\"",
            "sh -c 'sh -c \"git push origin main\"'",
            "eval git push origin main",
            "(git push origin main)",
            "echo $(git push origin main)",
            "git push 'origin' \"main\"",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
    }

    #[test]
    fn guard_rail_denies_bare_push_only_when_the_branch_is_main() {
        let on_main = |cmd| guard_rail(cmd, Path::new("/repo/main"), &fake_branch);
        for cmd in [
            "git push",
            "git push origin",
            "git push -u origin",
            "git push origin HEAD",
        ] {
            assert_eq!(guard(cmd), None, "{cmd} em feat/x");
            assert_eq!(on_main(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd} em main");
        }
        // `-C` muda a pasta em que a branch é lida.
        assert_eq!(guard("git -C ../main push"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(on_main("git -C /repo/feat push"), None);
        // Push explícito para outra branch não pergunta a branch atual.
        assert_eq!(on_main("git push origin feat/x"), None);
    }

    /// Branch ilegível (git que não responde, HEAD destacado, sem repo): o push
    /// que depende dela é recusado — falha fechado. Destino explícito que não
    /// é a main continua passando sem ler a branch.
    #[test]
    fn guard_rail_fails_closed_when_the_branch_is_unreadable() {
        let unknown = |cmd| guard_rail(cmd, Path::new("/x"), &|_: &Path| None);
        for cmd in [
            "git push",
            "git push origin",
            "git push origin HEAD",
            "git push origin @",
            "git push -u origin HEAD",
        ] {
            assert_eq!(unknown(cmd), Some(BRANCH_UNKNOWN_MESSAGE), "{cmd}");
        }
        assert_eq!(unknown("git push origin main"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(unknown("git push origin feat/x"), None);
        assert_eq!(unknown("git push origin HEAD:feat/x"), None);
        assert_eq!(unknown("git push --tags"), None);
        assert_eq!(unknown("git push --dry-run"), None);
    }

    #[test]
    fn guard_rail_denies_pr_merge() {
        for cmd in [
            "gh pr merge",
            "gh pr merge 42 --squash",
            "FOO=1 gh pr merge 42",
            "gh pr checks && gh pr merge --auto",
            "glab mr merge 7",
            "sh -c 'glab mr merge 7'",
        ] {
            assert_eq!(guard(cmd), Some(PR_MERGE_MESSAGE), "{cmd}");
        }
    }

    #[test]
    fn guard_rail_lets_everything_else_through() {
        for cmd in [
            "git push origin feat/x",
            "git push origin main-fix",
            "git push origin feat/main",
            "git push origin HEAD:feat/x",
            "git push origin --delete feat/x",
            "git log main",
            "git checkout main",
            "git merge main",
            "git pull origin main",
            "gh pr view 42",
            "gh pr create --base main",
            "gh pr list",
            "glab mr view 7",
            "ls ~/x",
            "rm -rf target",
            "curl https://example.com",
            // Escolha documentada: texto entre aspas é argumento, não comando.
            "echo \"git push origin main\"",
            "echo 'gh pr merge 42'",
            "git commit -m 'git push origin main'",
            "git push origin feat/x # main",
            "",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    /// Sessão parada na main: push sem refspec bate.
    fn on_main(command: &str) -> Option<&'static str> {
        guard_rail(command, Path::new("/repo/main"), &fake_branch)
    }

    #[test]
    fn guard_rail_drops_redirections_and_their_targets() {
        for cmd in [
            "git push origin 2>&1",
            "git push origin 2>&1 | tail -5",
            "git push origin 2>/dev/null",
            "git push origin 2> /dev/null",
            "git push origin > /tmp/log",
            "git push origin >/tmp/log",
            "git push origin >> /tmp/log 2>&1",
            "git push origin &> /tmp/log",
            "git push origin &>>/tmp/log",
            "git push origin >| /tmp/log",
            "git push origin 1>&2",
            "git push origin < /dev/null",
            "git push origin <<< x",
            "git push origin <&0",
            "git push origin 2>&-",
        ] {
            assert_eq!(on_main(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        for cmd in [
            "git push origin feat/x 2>&1",
            "git push origin feat/x > /tmp/main",
            "git push origin feat/x 2>main",
            "git push origin feat/x &>main",
            "git push origin feat/x <<< main",
        ] {
            assert_eq!(on_main(cmd), None, "{cmd}");
        }
        // Número que não está colado no operador é argumento.
        assert_eq!(
            guard("git push origin main 2 >/dev/null"),
            Some(PUSH_MAIN_MESSAGE)
        );
    }

    #[test]
    fn guard_rail_skips_keywords_grouping_and_wrapper_prefixes() {
        for cmd in [
            "if true; then git push origin main; fi",
            "if git push origin main; then echo ok; fi",
            "if false; then :; elif true; then git push origin main; fi",
            "if false; then :; else git push origin main; fi",
            "while true; do git push origin main; break; done",
            "until false; do git push origin main; done",
            "for r in a; do git push origin main; done",
            "{ git push origin main; }",
            "! git push origin main",
            "time git push origin main",
            "time -p git push origin main",
            "timeout 60 git push origin main",
            "timeout -s KILL 60 git push origin main",
            "timeout --signal TERM -k 5 60 git push origin main",
            "nice git push origin main",
            "nice -n 5 git push origin main",
            "nice -5 git push origin main",
            "nohup git push origin main",
            "sudo git push origin main",
            "sudo -u root git push origin main",
            "sudo -Eu root git push origin main",
            "sudo -uroot -E git push origin main",
            "sudo --user root git push origin main",
            "sudo -- git push origin main",
            "env -u FOO git push origin main",
            "env --unset=FOO BAR=1 git push origin main",
            "env -i PATH=/bin /usr/bin/git push origin main",
            "command -p git push origin main",
            "exec -a foo git push origin main",
            "exec -cl git push origin main",
            "stdbuf -o L git push origin main",
            "stdbuf -oL git push origin main",
            "sudo timeout 9 nice -n 1 env FOO=1 git push origin main",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        assert_eq!(guard("then gh pr merge 3"), Some(PR_MERGE_MESSAGE));
        for cmd in [
            "if true; then git push origin feat/x; fi",
            "echo fi then git push origin main",
            "timeout 60 git push origin feat/x",
            "sudo -u main git push origin feat/x",
            "env -u main git status",
            "time cargo test",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn guard_rail_opens_substitutions_inside_double_quotes() {
        for cmd in [
            "OUT=\"$(git push origin main 2>&1)\"; echo $OUT",
            "echo \"`git push origin main`\"",
            "echo \"saida: $(echo \"$(git push origin main)\")\"",
            "echo \"$( (cd x && git push origin main) )\"",
            "echo \"$(echo ')' && git push origin main)\"",
            "echo \"x $(gh pr merge 1) y\"",
        ] {
            assert!(guard(cmd).is_some(), "{cmd}");
        }
        for cmd in [
            "git commit -m \"fix: git push origin main docs\"",
            "echo \"$(git log main)\"",
            "echo \"$(git rev-parse HEAD) main\"",
            "echo '$(git push origin main)'",
            "echo '`gh pr merge 1`'",
            "echo \"\\$(git push origin main)\"",
            "echo \"\\`git push origin main\\`\"",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
        // Além de MAX_NESTING níveis, sem decisão (e sem estourar a pilha).
        let deep = format!(
            "{}git push origin main{}",
            "echo \"$(".repeat(MAX_NESTING + 1),
            ")\"".repeat(MAX_NESTING + 1)
        );
        assert_eq!(guard(&deep), None);
        let shallow = format!(
            "{}git push origin main{}",
            "echo \"$(".repeat(MAX_NESTING),
            ")\"".repeat(MAX_NESTING)
        );
        assert_eq!(guard(&shallow), Some(PUSH_MAIN_MESSAGE));
        let huge = "\"$(".repeat(100_000);
        assert_eq!(guard(&huge), None);
    }

    #[test]
    fn guard_rail_skips_heredoc_bodies() {
        for cmd in [
            "git commit -F - <<'EOF'\nfix(plugin): recusa `gh pr merge` no hook\nEOF",
            "gh pr create --body-file - <<'EOF'\nResumo.\n\ngit push origin main fica com o humano\nEOF",
            "cat > CONTRIBUTING.md <<'EOF'\nDepois:\ngit push origin main\nEOF",
            "cat <<\"EOF\"\n$(git push origin main)\nEOF",
            "cat <<\\EOF\n`gh pr merge 1`\nEOF",
            "cat <<EOF\ngit push origin main\nEOF",
            "cat <<-EOF\n\tgit push origin main\n\tEOF\necho ok",
            "cat <<A <<B\ngit push origin main\nA\ngh pr merge 1\nB",
            "cat <<EOF | grep x\ngit push origin main\nEOF",
            "git commit -m \"$(cat <<'EOF'\ndon't push to main\ngit push origin main\nEOF\n)\"",
            "git commit -m \"$(cat <<'EOF'\nfix(x): a) b (c\n\nCo-Authored-By: X <x@example.com>\nEOF\n)\" && git push origin feat/x",
        ] {
            assert_eq!(on_main(cmd), None, "{cmd:?}");
        }
        for cmd in [
            "git commit -F - <<'EOF'\nfix: it's ok\nEOF\ngit push origin main",
            "git commit -m \"$(cat <<'EOF'\ndon't\nEOF\n)\" && git push origin main",
            "git commit -m \"$(cat <<'EOF'\nuse 5\" screen\nEOF\n)\" && git push origin main",
            "cat <<-EOF\n\tx\n\tEOF\ngit push origin main",
            "cat <<A <<B\nx\nA\ny\nB\ngit push origin main",
            "cat <<EOF\n$(git push origin main)\nEOF",
            "cat <<EOF\n`git push origin main`\nEOF",
            "cat <<EOF\nlinha \"$(git push origin main)\" aspas\nEOF",
        ] {
            assert_eq!(on_main(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd:?}");
        }
        assert_eq!(
            on_main("cat > notes.md <<'EOF'\nwe don't push\nEOF\ngh pr merge 3"),
            Some(PR_MERGE_MESSAGE)
        );
        // `<<-` só tira tab: o delimitador com espaço na frente não fecha.
        assert_eq!(
            on_main("cat <<-EOF\n  EOF\ngit push origin main"),
            None,
            "o corpo segue até o fim"
        );
    }

    #[test]
    fn guard_rail_lets_pushes_that_do_not_push_the_branch_through() {
        for cmd in [
            "git push --tags",
            "git push origin --tags",
            "git push --dry-run",
            "git push -n origin",
            "git push --dry-run origin main",
            "git push origin v1.2.0",
        ] {
            assert_eq!(on_main(cmd), None, "{cmd}");
        }
        for cmd in [
            "git push --follow-tags",
            "git push origin --follow-tags",
            "git push --tags origin main",
            "git push origin --delete main",
            "git push -d origin master",
        ] {
            assert_eq!(on_main(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        assert_eq!(
            guard("git push origin --delete main"),
            Some(PUSH_MAIN_MESSAGE)
        );
    }

    #[test]
    fn guard_rail_follows_cd_within_the_command() {
        assert_eq!(guard("cd /repo/main && git push"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(guard("cd ../main && git push"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(guard("cd ../main; git -C . push"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(
            guard("cd /repo && git -C main push"),
            Some(PUSH_MAIN_MESSAGE)
        );
        assert_eq!(
            guard("cd -P /repo/main\ngit push origin"),
            Some(PUSH_MAIN_MESSAGE)
        );
        assert_eq!(on_main("cd /repo/feat && git push"), None);
        assert_eq!(on_main("cd ../feat && git push -u origin"), None);
        // Pasta não literal: a branch fica desconhecida — falha fechado.
        for cmd in [
            "cd /repo/main && cd - && git push",
            "cd /repo/main && cd && git push",
            "cd ~/x && git push",
            "cd $REPO && git push",
            "cd \"$(dirname x)\" && git push",
            "pushd ~/x && git push",
            "pushd /repo/feat; popd; git push",
            "git -C ~/x push",
            "git -C $D push origin",
            "git --git-dir=$G push",
            "git --git-dir ~/x/.git push",
            "git --work-tree=$W push",
            "GIT_DIR=$G git push",
            "export GIT_DIR=~/r/.git; git push",
        ] {
            assert_eq!(guard(cmd), Some(BRANCH_UNKNOWN_MESSAGE), "{cmd}");
        }
        // Caminho literal: lê a branch de lá.
        assert_eq!(
            guard("pushd /repo/main && git push"),
            Some(PUSH_MAIN_MESSAGE)
        );
        assert_eq!(
            guard("GIT_DIR=/repo/main/.git git push"),
            Some(PUSH_MAIN_MESSAGE)
        );
        assert_eq!(
            guard("git --git-dir=/repo/main/.git push"),
            Some(PUSH_MAIN_MESSAGE)
        );
        assert_eq!(on_main("git -C /repo/feat push"), None);
        // Destino explícito que não é a main não depende da pasta.
        assert_eq!(guard("cd $X && git push origin feat/x"), None);
        // `cd` dentro de `$(...)` não vaza para fora.
        assert_eq!(guard("X=\"$(cd /repo/main)\"; git push"), None);
        assert_eq!(
            guard("X=\"$(cd /repo/main && git push)\""),
            Some(PUSH_MAIN_MESSAGE)
        );
        // `sh -c` começa na pasta corrente.
        assert_eq!(
            guard("cd /repo/main && sh -c 'git push'"),
            Some(PUSH_MAIN_MESSAGE)
        );
    }

    #[test]
    fn guard_rail_skips_the_gh_repo_flag() {
        for cmd in [
            "gh -R o/r pr merge 1",
            "gh --repo o/r pr merge 1",
            "gh --repo=o/r pr merge 1",
            "gh -Ro/r pr merge 1",
            "glab -R o/r mr merge 1",
        ] {
            assert_eq!(guard(cmd), Some(PR_MERGE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh -R o/r pr view 1",
            "gh --repo=o/r pr list",
            "glab -R o/r mr view 1",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn guard_rail_reads_at_as_head() {
        assert_eq!(on_main("git push origin @"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(on_main("git push origin @:main"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(guard("git push origin @:main"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(guard("git push origin @"), None);
        assert_eq!(on_main("git push origin @:feat/x"), None);
    }

    #[test]
    fn guard_rail_line_continuation_does_not_add_a_word() {
        assert_eq!(on_main("git push \\\n  --tags"), None);
        assert_eq!(
            guard("git push \\\n  origin \\\n  main"),
            Some(PUSH_MAIN_MESSAGE)
        );
    }

    #[test]
    fn current_branch_reads_the_repo_of_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(ok.success());
        assert_eq!(current_branch(dir.path()).as_deref(), Some("main"));
        assert_eq!(
            guard_rail("git push", dir.path(), &current_branch),
            Some(PUSH_MAIN_MESSAGE)
        );
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(current_branch(outside.path()), None);
    }

    /// Repo sintético: subpasta, worktree (`.git` é arquivo) e HEAD destacado.
    #[test]
    fn current_branch_reads_head_without_running_git() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let git = |dir: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(ok.success(), "git {args:?}");
        };
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "x"]);
        assert_eq!(
            current_branch(&repo.join("src/deep")).as_deref(),
            Some("main")
        );

        let wt = tmp.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/y",
                wt.to_str().unwrap(),
            ],
        );
        assert_eq!(current_branch(&wt).as_deref(), Some("feat/y"));

        git(&repo, &["checkout", "-q", "--detach"]);
        assert_eq!(current_branch(&repo), None);
        assert_eq!(
            guard_rail("git push", &repo, &current_branch),
            Some(BRANCH_UNKNOWN_MESSAGE)
        );
    }

    /// Destinos que podem ser a main: `heads/main`, `:` (matching), curinga,
    /// expansão de shell, `push.default=matching`.
    #[test]
    fn guard_rail_denies_destinations_that_may_be_main() {
        for cmd in [
            "git push origin heads/main",
            "git push origin refs/heads/master",
            "git push origin HEAD:heads/main",
            "git push origin :",
            "git push origin ':'",
            "git push origin 'refs/heads/*:refs/heads/*'",
            "git push origin 'feat/*'",
            "git push origin $b",
            "git push origin \"$BRANCH\"",
            "git push origin ${B:-main}",
            "git push origin HEAD:$b",
            "git push origin {main,dev}",
            "git push origin \"$(git rev-parse --abbrev-ref HEAD)\"",
            "git push origin `echo main`",
            "git -c push.default=matching push",
            "git -c push.default=matching push origin",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        // `push.default=current|upstream` com a branch ilegível: recusa.
        let unknown = |cmd| guard_rail(cmd, Path::new("/x"), &|_: &Path| None);
        for cmd in [
            "git -c push.default=current push",
            "git -c push.default=upstream push origin",
        ] {
            assert_eq!(unknown(cmd), Some(BRANCH_UNKNOWN_MESSAGE), "{cmd}");
        }
        assert_eq!(guard("git -c push.default=current push"), None);
        assert_eq!(guard("git push origin heads/feat"), None);
    }

    /// Windows e afins: `git.exe`, `cmd /c`, `pwsh -c`, `powershell -Command`.
    #[test]
    fn guard_rail_reads_windows_programs_and_shells() {
        for cmd in [
            "git.exe push origin main",
            "'C:\\Git\\bin\\git.exe' push origin main",
            "GIT push origin main",
            "cmd /c git push origin main",
            "cmd.exe /C \"git push origin main\"",
            "pwsh -c 'git push origin main'",
            "powershell -Command \"git push origin main\"",
            "pwsh.exe -NoProfile -Command git push origin main",
            "fish -c 'git push origin main'",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        assert_eq!(guard("cmd /c gh pr merge 3"), Some(PR_MERGE_MESSAGE));
        assert_eq!(guard("pwsh -c 'git status'"), None);
    }

    /// Merge e escrita na main por API.
    #[test]
    fn guard_rail_denies_merges_through_the_api() {
        for cmd in [
            "gh api -X PUT repos/o/r/pulls/12/merge",
            "gh api repos/o/r/pulls/12/merge -X PUT",
            "gh api --method PUT /repos/o/r/pulls/12/merge",
            "gh api graphql -f query='mutation { mergePullRequest(input:{pullRequestId:\"x\"}) { clientMutationId } }'",
            "gh api graphql -f query='mutation { enablePullRequestAutoMerge(input:{}) { clientMutationId } }'",
            "gh pr -R o/r merge 3",
            "gh pr merge -R o/r 3",
            "curl -X PUT -H 'Authorization: token x' https://api.github.com/repos/o/r/pulls/12/merge",
            "curl https://gitlab.com/api/v4/projects/1/merge_requests/7/merge -X PUT",
            "glab api -X PUT projects/1/merge_requests/7/merge",
        ] {
            assert_eq!(guard(cmd), Some(PR_MERGE_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh api -X PATCH repos/o/r/git/refs/heads/main -f sha=abc",
            "gh api repos/o/r/git/refs/heads/master -f sha=abc -F force=true",
            "curl -X PATCH https://api.github.com/repos/o/r/git/refs/heads/main -d '{\"sha\":\"x\"}'",
            "gh api repos/o/r/merges -f base=main -f head=feat",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        for cmd in [
            "gh api repos/o/r/pulls/12",
            "gh api repos/o/r/git/refs/heads/main",
            "gh api -X GET repos/o/r/git/refs/heads/main",
            "gh api repos/o/r/merges -f base=feat -f head=x",
            "curl https://api.github.com/repos/o/r/pulls/12",
            "gh pr view -R o/r 3 --json mergeable",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    /// `bash -c -- '...'`, `bash <<EOF`, `bash <<< '...'` e alias de push.
    #[test]
    fn guard_rail_opens_more_shell_forms_and_git_aliases() {
        for cmd in [
            "bash -c -- 'git push origin main'",
            "bash <<EOF\ngit push origin main\nEOF",
            "sh <<'EOF'\ncd x && git push origin main\nEOF",
            "bash <<< 'git push origin main'",
            "git -c alias.p=push p origin main",
            "git -c alias.ship='push origin main' ship",
            "git -c alias.x='!git push origin main' x",
        ] {
            assert_eq!(guard(cmd), Some(PUSH_MAIN_MESSAGE), "{cmd}");
        }
        for cmd in [
            "cat <<EOF\ngit push origin main\nEOF",
            "bash -c -- 'git push origin feat/x'",
            "git -c alias.st=status st",
            "git -c alias.p=push p origin feat/x",
        ] {
            assert_eq!(guard(cmd), None, "{cmd}");
        }
    }

    /// Ferramentas de shell além do `Bash`: `PowerShell` e as do Codex (com o
    /// comando em argv).
    #[test]
    fn hook_output_covers_powershell_and_codex_shell_tools() {
        for (tool, input) in [
            ("PowerShell", json!({"command": "git push origin main"})),
            (
                "shell",
                json!({"command": ["bash", "-lc", "git push origin main"]}),
            ),
            (
                "local_shell",
                json!({"command": ["git", "push", "origin", "main"]}),
            ),
            ("exec_command", json!({"cmd": "git push origin main"})),
        ] {
            let out = hook_output(&pre_tool_use(tool, input), &fake_branch);
            assert!(out.is_some_and(|o| o.contains(PUSH_MAIN_MESSAGE)), "{tool}");
        }
        assert_eq!(
            hook_output(
                &pre_tool_use("PowerShell", json!({"command": "git status"})),
                &fake_branch
            ),
            None
        );
    }

    /// Reftable: o `HEAD` em arquivo é `.invalid`; a branch vem do git.
    #[test]
    fn current_branch_asks_git_in_a_reftable_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("rt");
        std::fs::create_dir_all(&repo).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q", "--ref-format=reftable", "-b", "main"])
            .current_dir(&repo)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("git local sem reftable: teste pulado");
            return;
        }
        let head = std::fs::read_to_string(repo.join(".git/HEAD")).unwrap();
        assert!(head.contains(".invalid"), "{head}");
        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(
            guard_rail("git push", &repo, &current_branch),
            Some(PUSH_MAIN_MESSAGE)
        );
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
        let deny = hook_output(
            &pre_tool_use("Bash", json!({"command": "git push origin main"})),
            &fake_branch,
        )
        .expect("nega");
        let deny: Value = serde_json::from_str(&deny).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            deny["hookSpecificOutput"]["permissionDecisionReason"],
            PUSH_MAIN_MESSAGE
        );
        let merge = hook_output(
            &pre_tool_use("Bash", json!({"command": "gh pr merge 1"})),
            &fake_branch,
        )
        .unwrap();
        assert!(merge.contains(PR_MERGE_MESSAGE));

        // O `cwd` do payload é o que decide a branch de um push sem refspec.
        let mut bare = pre_tool_use("Bash", json!({"command": "git push"}));
        assert_eq!(hook_output(&bare, &fake_branch), None);
        bare["cwd"] = json!("/repo/main");
        assert!(hook_output(&bare, &fake_branch).is_some());

        // Escrita, Bash comum e rede: nada — nem `allow`. Quem decide é o
        // prompt nativo do Claude.
        for payload in [
            pre_tool_use(
                "Write",
                json!({"file_path": "/repo/feat/a.rs", "content": "x"}),
            ),
            pre_tool_use("Edit", json!({"file_path": "/repo/feat/a.rs"})),
            pre_tool_use("Bash", json!({"command": "ls ~/x"})),
            pre_tool_use("Bash", json!({"command": "rm -rf ~/x"})),
            pre_tool_use("WebFetch", json!({"url": "https://example.com"})),
            pre_tool_use("mcp__github__create_issue", json!({})),
            // Ferramenta que não é `Bash` não passa pelo guarda-corpo.
            pre_tool_use("Task", json!({"command": "git push origin main"})),
            pre_tool_use("Bash", json!({})),
        ] {
            assert_eq!(hook_output(&payload, &fake_branch), None, "{payload}");
        }

        // `PermissionRequest` só avisa o core: nunca responde, nem para o
        // que o guarda-corpo nega.
        for event in [
            "PermissionRequest",
            "SessionStart",
            "Notification",
            "Stop",
            "SubagentStart",
            "SubagentStop",
        ] {
            let mut payload = pre_tool_use("Bash", json!({"command": "git push origin main"}));
            payload["hook_event_name"] = json!(event);
            assert_eq!(hook_output(&payload, &fake_branch), None, "{event}");
        }
        assert_eq!(hook_output(&Value::Null, &fake_branch), None);
    }
}
