//! Plugin do TYBA para o Claude Code (modelo Warp: o agente roda livre, o
//! TYBA observa).
//!
//! O claude que o usuário roda num pane pede permissão pelo prompt dele
//! mesmo; o plugin, instalado só por ação explícita do usuário, não nega
//! ferramenta nenhuma. Ele faz duas coisas:
//!
//! - Fora do TYBA (sem `TYBA_PANE_TOKEN`) o hook sai na hora com 0: um `sh`
//!   puro, sem `jq` e sem spawn de binário nenhum.
//! - Num claude já com gate (`TYBA_HOOK_SOCKET` do `--settings` da sessão
//!   gerenciada/hospedada, ou `TYBA_HOSTED=1`) também sai: os eventos já
//!   chegam pelos hooks da sessão, e não podem chegar duas vezes.
//! - Num claude cru dentro de um pane: `SessionStart`, `Notification`,
//!   `PermissionRequest`, `Stop` e `Subagent*` só avisam o core — nenhum
//!   responde nada ao Claude. O `PreToolUse` (só `Bash`) aplica o
//!   guarda-corpo fixo: `git push` para main/master e merge de PR/MR são
//!   recusados com motivo; todo o resto fica sem decisão, e o prompt nativo
//!   do Claude decide.
//!
//! O guarda-corpo é texto de comando: pega engano, não agente determinado
//! (script, alias contornam). Fronteira de verdade não mora aqui.
//!
//! Canal: o mesmo socket 0600 do shim→core, com peer cred, mais um token por
//! pane (`TYBA_PANE_TOKEN`) que o core confere. Nada aqui usa
//! `onFailure: "block"`: o hook é global, e um app movido de lugar travaria o
//! Claude da máquina inteira.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const MARKETPLACE: &str = "tyba";
pub const PLUGIN: &str = "tyba";
pub const PLUGIN_ID: &str = "tyba@tyba";

const HOOK_SCRIPT: &str = include_str!("claude_plugin_hook.sh");

/// Os eventos que o plugin só repassa ao core (o `SessionStart` em todas as
/// origens). O `PreToolUse` fica de fora: é local, só o guarda-corpo.
const SESSION_START_MATCHER: &str = "startup|resume|clear|compact|fork";
const OBSERVED_EVENTS: &[&str] = &[
    "SessionStart",
    "Notification",
    "PermissionRequest",
    "Stop",
    "SubagentStart",
    "SubagentStop",
];

pub const PUSH_MAIN_MESSAGE: &str = "TYBA: push para main/master por agente é recusado (guarda-corpo fixo do TYBA). Faça o push numa branch própria e abra um PR; o merge na main fica com o humano.";
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
    if depth > MAX_NESTING {
        return None;
    }
    let src: Vec<char> = command.chars().collect();
    let pieces = scan(&src, &mut 0, depth, false);
    check(&pieces, cwd, &mut cwd.to_path_buf(), current_branch, depth)
}

/// Avalia os pedaços em ordem. `dir` é a pasta corrente do script (muda com
/// `cd`); `start` é para onde um `cd` desconhecido volta.
fn check(
    pieces: &[Piece],
    start: &Path,
    dir: &mut PathBuf,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    if depth > MAX_NESTING {
        return None;
    }
    for piece in pieces {
        let verdict = match piece {
            // Substituição roda num subshell: `cd` lá dentro não vaza.
            Piece::Nested(inner) => {
                let here = dir.clone();
                check(inner, &here, &mut here.clone(), current_branch, depth + 1)
            }
            Piece::Command(words) => command_verdict(words, start, dir, current_branch, depth),
        };
        if verdict.is_some() {
            return verdict;
        }
    }
    None
}

fn command_verdict(
    words: &[String],
    start: &Path,
    dir: &mut PathBuf,
    current_branch: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Option<&'static str> {
    let words = canonical_words(words);
    let (program, args) = words.split_first()?;
    match basename(program) {
        "cd" => {
            let target = args.iter().find(|a| !a.starts_with('-') || *a == "-");
            *dir = match target.map(String::as_str) {
                Some(t) if t != "-" && !t.starts_with(['~', '$']) => dir.join(t),
                _ => start.to_path_buf(),
            };
            None
        }
        "sh" | "bash" | "zsh" | "dash" | "ksh" => {
            shell_script(args).and_then(|s| guard_rail_at(s, dir, current_branch, depth + 1))
        }
        "eval" => guard_rail_at(&args.join(" "), dir, current_branch, depth + 1),
        "git" => git_push_hits_main(args, dir, current_branch).then_some(PUSH_MAIN_MESSAGE),
        "gh" if gh_subcommand(args).starts_with(&["pr".into(), "merge".into()]) => {
            Some(PR_MERGE_MESSAGE)
        }
        "glab" if gh_subcommand(args).starts_with(&["mr".into(), "merge".into()]) => {
            Some(PR_MERGE_MESSAGE)
        }
        _ => None,
    }
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
            Next::Delim { strip_tabs } => self.heredocs.push(Heredoc {
                delim: word,
                strip_tabs,
                expands: !self.quoted,
            }),
        }
        self.in_word = false;
        self.quoted = false;
    }

    fn end_command(&mut self, pieces: &mut Vec<Piece>) {
        self.end_word();
        self.next = Next::Arg;
        if !self.words.is_empty() {
            pieces.push(Piece::Command(std::mem::take(&mut self.words)));
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
                b.end_command(&mut pieces);
                for doc in std::mem::take(&mut b.heredocs) {
                    let body = heredoc_body(src, i, &doc);
                    if doc.expands && depth < MAX_NESTING {
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
                        Some('<') => *i += 1,
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
                b.end_command(&mut pieces);
            }
            ')' => {
                parens = parens.saturating_sub(1);
                b.end_command(&mut pieces);
            }
            ';' | '&' | '|' | '`' => b.end_command(&mut pieces),
            _ => {
                b.in_word = true;
                b.word.push(c);
            }
        }
    }
    b.end_command(&mut pieces);
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

/// Consome as linhas do corpo até a linha igual ao delimitador (inclusive).
/// Devolve o corpo só se ele expande — senão nada nele importa.
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
        if doc.expands {
            body.extend_from_slice(line);
            body.push('\n');
        }
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

/// Os argumentos de `gh`/`glab` depois da opção global de repositório
/// (`-R dono/repo`, `--repo dono/repo`, `--repo=dono/repo`).
fn gh_subcommand(mut args: &[String]) -> &[String] {
    while let Some(a) = args.first() {
        args = match a.as_str() {
            "-R" | "--repo" => args.get(2..).unwrap_or(&[]),
            _ if a.starts_with("--repo=") || (a.starts_with("-R") && a.len() > 2) => &args[1..],
            _ => break,
        };
    }
    args
}

/// O script de `sh -c '...'` (também `-lc`, `-ec`...).
fn shell_script(args: &[String]) -> Option<&str> {
    let flag = args
        .iter()
        .position(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('c'))?;
    args.get(flag + 1).map(String::as_str)
}

fn is_main_ref(name: &str) -> bool {
    matches!(
        name.strip_prefix("refs/heads/").unwrap_or(name),
        "main" | "master"
    )
}

/// `git [opções globais] push ...` cujo destino é main/master: refspec
/// explícita (`main`, `+main`, `HEAD:main`, `x:refs/heads/main`), `--all`/
/// `--mirror`, ou sem refspec (ou `HEAD`/`@`) com a branch atual sendo main.
/// Não empurram branch nenhuma: `--dry-run`/`-n`, e `--tags` sem refspec.
fn git_push_hits_main(
    args: &[String],
    cwd: &Path,
    current_branch: &dyn Fn(&Path) -> Option<String>,
) -> bool {
    let mut dir = cwd.to_path_buf();
    let mut i = 0;
    while let Some(a) = args.get(i) {
        match a.as_str() {
            "-C" => {
                if let Some(d) = args.get(i + 1) {
                    dir = dir.join(d);
                }
                i += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env" => i += 2,
            _ if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    if args.get(i).map(String::as_str) != Some("push") {
        return false;
    }
    let mut positional = Vec::new();
    let (mut all, mut tags, mut dry_run) = (false, false, false);
    let mut rest = args[i + 1..].iter();
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
        return false;
    }
    if all {
        return true;
    }
    let on_main = || current_branch(&dir).is_some_and(|b| is_main_ref(&b));
    let refspecs = positional.get(1..).unwrap_or(&[]);
    if refspecs.is_empty() {
        return !tags && on_main();
    }
    refspecs.iter().any(|spec| {
        let spec = spec.trim_start_matches('+');
        let dst = spec.rsplit_once(':').map_or(spec, |(_, d)| d);
        if dst == "HEAD" || dst == "@" {
            on_main()
        } else {
            is_main_ref(dst)
        }
    })
}

/// O que o hook imprime no stdout para o Claude. Só o `PreToolUse` do `Bash`
/// que bate no guarda-corpo tem algo a dizer (`deny` com motivo); qualquer
/// outro caso é `None` — stdout vazio e saída 0, que para o Claude Code é
/// "sem decisão": o fluxo de permissão normal dele segue.
pub fn hook_output(
    payload: &Value,
    current_branch: &dyn Fn(&Path) -> Option<String>,
) -> Option<String> {
    let field = |k: &str| payload.get(k).and_then(Value::as_str);
    if field("hook_event_name")? != "PreToolUse" || field("tool_name")? != "Bash" {
        return None;
    }
    let command = payload.get("tool_input")?.get("command")?.as_str()?;
    let cwd = PathBuf::from(field("cwd").filter(|c| !c.is_empty()).unwrap_or("."));
    let reason = guard_rail(command, &cwd, current_branch)?;
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

/// [`hook_output`] com a branch lida do git de verdade: o guarda-corpo que o
/// hook do claude digitado (`tyba _hook --observe`) aplica — a mesma decisão
/// e as mesmas mensagens do plugin.
pub fn guard_rail_output(payload: &Value) -> Option<String> {
    hook_output(payload, &current_branch)
}

/// A branch atual de `dir` (`git symbolic-ref --short HEAD`, que também
/// responde numa branch ainda sem commit), pelo git read-only do TYBA. Só
/// roda para `git push` sem refspec.
fn current_branch(dir: &Path) -> Option<String> {
    let mut cmd = crate::worktree::git_in(dir);
    cmd.args(["symbolic-ref", "--short", "-q", "HEAD"]);
    let out = crate::worktree::run_git(cmd, "branch atual").ok()?;
    Some(String::from_utf8_lossy(&out).trim().to_string())
}

// --- Arquivos do plugin ------------------------------------------------------

pub fn marketplace_json() -> Value {
    json!({
        "name": MARKETPLACE,
        "owner": { "name": "TYBA" },
        "plugins": [{
            "name": PLUGIN,
            "source": "./plugin",
            "description": "Avisa o TYBA do que o claude faz num pane (início, pedido de permissão, fim) e recusa push para main/master e merge de PR.",
        }]
    })
}

pub fn plugin_json() -> Value {
    json!({
        "name": PLUGIN,
        "description": "Integração do TYBA: o claude num pane do TYBA avisa o app quando precisa de você, e push para main/master e merge de PR são recusados. Fora do TYBA não faz nada.",
        "version": env!("CARGO_PKG_VERSION"),
        "author": { "name": "TYBA" },
    })
}

pub fn hooks_json() -> Value {
    let command = "\"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh\"";
    let mut hooks = serde_json::Map::new();
    for event in OBSERVED_EVENTS.iter().chain(&["PreToolUse"]) {
        let mut entry = json!({ "hooks": [{ "type": "command", "command": command }] });
        match *event {
            "SessionStart" => entry["matcher"] = json!(SESSION_START_MATCHER),
            // O guarda-corpo só olha `Bash`: as outras ferramentas nem chamam o hook.
            "PreToolUse" => entry["matcher"] = json!("Bash"),
            _ => {}
        }
        hooks.insert((*event).to_string(), json!([entry]));
    }
    json!({ "description": "TYBA: observa o claude num pane e recusa push para main/master e merge de PR", "hooks": hooks })
}

pub fn hook_script() -> &'static str {
    HOOK_SCRIPT
}

/// Escreve o marketplace local em `dir`: `.claude-plugin/marketplace.json` e
/// o plugin em `plugin/`. Reescreve sempre — o conteúdo é do binário atual.
pub fn write_marketplace(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let plugin = dir.join("plugin");
    std::fs::create_dir_all(dir.join(".claude-plugin"))?;
    std::fs::create_dir_all(plugin.join(".claude-plugin"))?;
    std::fs::create_dir_all(plugin.join("hooks"))?;
    std::fs::create_dir_all(plugin.join("scripts"))?;
    let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    std::fs::write(
        dir.join(".claude-plugin/marketplace.json"),
        pretty(marketplace_json()),
    )?;
    std::fs::write(
        plugin.join(".claude-plugin/plugin.json"),
        pretty(plugin_json()),
    )?;
    std::fs::write(plugin.join("hooks/hooks.json"), pretty(hooks_json()))?;
    let script = plugin.join("scripts/hook.sh");
    std::fs::write(&script, HOOK_SCRIPT)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

// --- Instalação ---------------------------------------------------------------

/// Os comandos documentados do Claude Code, como argv (nunca shell): adiciona
/// o marketplace local e instala o plugin, os dois no escopo do usuário.
pub fn install_commands(claude: &Path, marketplace_dir: &Path) -> Vec<Vec<OsString>> {
    vec![
        vec![
            claude.into(),
            "plugin".into(),
            "marketplace".into(),
            "add".into(),
            marketplace_dir.into(),
            "--scope".into(),
            "user".into(),
        ],
        vec![
            claude.into(),
            "plugin".into(),
            "install".into(),
            PLUGIN_ID.into(),
            "--scope".into(),
            "user".into(),
        ],
    ]
}

pub fn uninstall_commands(claude: &Path) -> Vec<Vec<OsString>> {
    vec![
        vec![
            claude.into(),
            "plugin".into(),
            "uninstall".into(),
            PLUGIN_ID.into(),
        ],
        vec![
            claude.into(),
            "plugin".into(),
            "marketplace".into(),
            "remove".into(),
            MARKETPLACE.into(),
        ],
    ]
}

/// Diretório de config do Claude: `CLAUDE_CONFIG_DIR` ou `~/.claude`.
pub fn claude_config_dir(env_config_dir: Option<&str>, home: &Path) -> PathBuf {
    env_config_dir
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

/// Por que hooks não vão rodar mesmo instalado: `disableAllHooks` em
/// qualquer settings, ou `allowManagedHooksOnly` na config gerenciada
/// (só hooks da empresa rodam). `None` = livre.
pub fn hooks_blocked(user: Option<&Value>, managed: Option<&Value>) -> Option<&'static str> {
    let flag = |v: Option<&Value>, key: &str| {
        v.and_then(|v| v.get(key)).and_then(Value::as_bool) == Some(true)
    };
    if flag(user, "disableAllHooks") || flag(managed, "disableAllHooks") {
        return Some("disable_all_hooks");
    }
    if flag(managed, "allowManagedHooksOnly") {
        return Some("managed_hooks_only");
    }
    None
}

pub fn managed_settings_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")
    } else {
        PathBuf::from("/etc/claude-code/managed-settings.json")
    }
}

/// Se o plugin aparece no registro de instalados do Claude.
pub fn is_installed(installed_plugins: Option<&Value>) -> bool {
    installed_plugins
        .and_then(|v| v.get("plugins").or(Some(v)))
        .and_then(Value::as_object)
        .is_some_and(|map| map.contains_key(PLUGIN_ID))
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// O que a tela de Integrações mostra.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginStatus {
    /// O `claude` está instalado (dá para instalar a integração).
    pub available: bool,
    pub installed: bool,
    /// `disable_all_hooks` / `managed_hooks_only`: instalado ou não, os hooks
    /// não rodam — a tela diz que está bloqueado.
    pub blocked: Option<&'static str>,
}

pub fn status(home: &Path, config_dir: Option<&str>) -> PluginStatus {
    let env_dir = config_dir
        .map(str::to_string)
        .or_else(|| std::env::var("CLAUDE_CONFIG_DIR").ok());
    let config = claude_config_dir(env_dir.as_deref(), home);
    PluginStatus {
        available: crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
            .is_some(),
        installed: is_installed(read_json(&config.join("plugins/installed_plugins.json")).as_ref()),
        blocked: hooks_blocked(
            read_json(&config.join("settings.json")).as_ref(),
            read_json(&managed_settings_path()).as_ref(),
        ),
    }
}

/// Fatia 4 do agente livre: o chip "Instalar plugin" no pane. Só aparece
/// com o plugin ausente e os hooks livres — bloqueado, instalar não adianta
/// nada, e a tela de Integrações já diz por quê.
pub fn should_suggest(installed: bool, blocked: Option<&str>) -> bool {
    !installed && blocked.is_none()
}

/// O mesmo [`status`] sem resolver o binário: quem pergunta já achou um
/// claude rodando no pane. Só lê dois JSON, então cabe no poll do probe.
/// `config_dir` é o `CLAUDE_CONFIG_DIR` do claude do pane — não o do TYBA,
/// que aberto pelo Dock não tem o rc do shell.
pub fn suggest_install(home: &Path, config_dir: Option<&str>) -> bool {
    suggest_in(
        &claude_config_dir(config_dir, home),
        read_json(&managed_settings_path()).as_ref(),
    )
}

fn suggest_in(config: &Path, managed: Option<&Value>) -> bool {
    should_suggest(
        is_installed(read_json(&config.join("plugins/installed_plugins.json")).as_ref()),
        hooks_blocked(read_json(&config.join("settings.json")).as_ref(), managed),
    )
}

/// Roda os comandos do Claude Code em sequência, parando no primeiro que
/// falha. Só por clique do usuário (Configurações → Integrações).
/// `config_dir`: o `CLAUDE_CONFIG_DIR` do claude do pane (chip); `None`
/// herda o do TYBA (Configurações).
pub fn run_commands(commands: &[Vec<OsString>], config_dir: Option<&str>) -> Result<(), String> {
    for argv in commands {
        let Some((program, args)) = argv.split_first() else {
            continue;
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .env("PATH", crate::shell_path::agent_path())
            .stdin(std::process::Stdio::null());
        if let Some(dir) = config_dir.filter(|d| !d.is_empty()) {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        let out = cmd.output().map_err(|e| format!("claude não rodou: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg: String = stderr.trim().chars().take(400).collect();
            return Err(crate::session::redact::redact(&msg).into_owned());
        }
    }
    Ok(())
}

pub fn install(marketplace_dir: &Path, config_dir: Option<&str>) -> Result<(), String> {
    let claude = crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
        .ok_or("claude não encontrado no PATH")?;
    write_marketplace(marketplace_dir).map_err(|e| format!("arquivos do plugin: {e}"))?;
    run_commands(&install_commands(&claude, marketplace_dir), config_dir)
}

pub fn uninstall() -> Result<(), String> {
    let claude = crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
        .ok_or("claude não encontrado no PATH")?;
    run_commands(&uninstall_commands(&claude), None)
}

// --- Cliente: `tyba _plugin-hook` ---------------------------------------------

const MAX_STDIN: u64 = 1024 * 1024;

/// Um resumo curto da chamada para o histórico: o comando do Bash, o
/// arquivo, a URL — já redigido (princípio #10). Nunca o payload inteiro.
pub fn tool_summary(tool: &str, input: Option<&Value>) -> String {
    let field = |k: &str| input.and_then(|i| i.get(k)).and_then(Value::as_str);
    let detail = field("command")
        .or_else(|| field("file_path"))
        .or_else(|| field("url"))
        .or_else(|| field("path"))
        .unwrap_or("");
    let line = if detail.is_empty() {
        tool.to_string()
    } else {
        format!("{tool}: {detail}")
    };
    let short: String = line.chars().take(400).collect();
    crate::session::redact::redact(&short).into_owned()
}

/// Ponto de entrada de `tyba _plugin-hook`, chamado pelo `hook.sh` do plugin
/// só num claude cru dentro de um pane. Sempre sai com 0. O guarda-corpo do
/// `PreToolUse` é decidido aqui, sem o core (o comando inteiro não atravessa
/// o canal); os eventos observados vão ao core e não respondem nada.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn maybe_run_plugin_hook() -> Option<i32> {
    use std::io::Read;

    if std::env::args().nth(1).as_deref() != Some("_plugin-hook") {
        return None;
    }
    let gated = std::env::var_os("TYBA_HOOK_SOCKET").is_some()
        || std::env::var("TYBA_HOSTED").as_deref() == Ok("1");
    let Some(token) = std::env::var("TYBA_PANE_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    else {
        return Some(0);
    };
    if gated {
        return Some(0);
    }
    let mut raw = Vec::new();
    let _ = std::io::stdin().take(MAX_STDIN).read_to_end(&mut raw);
    let payload: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    if let Some(out) = hook_output(&payload, &current_branch) {
        println!("{out}");
    }
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if OBSERVED_EVENTS.contains(&event) {
        let tool = payload
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let summary = tool
            .as_deref()
            .map(|t| tool_summary(t, payload.get("tool_input")));
        let _ =
            crate::hook_ipc::channel::send_plugin_event(token, event.to_string(), tool, summary);
    }
    Some(0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn maybe_run_plugin_hook() -> Option<i32> {
    (std::env::args().nth(1).as_deref() == Some("_plugin-hook")).then_some(0)
}

// --- Core: evento do plugin ---------------------------------------------------

/// Valida o evento: o token tem que ser o de um pane, e o peer tem que
/// descender do shell DESSE pane. Devolve o pane; qualquer falha é `None` —
/// o evento é ignorado.
pub fn resolve_plugin_event(
    token: &str,
    peer_session: Option<crate::session::SessionId>,
    session_for_token: impl Fn(&str) -> Option<crate::session::SessionId>,
) -> Option<crate::session::SessionId> {
    let session = session_for_token(token)?;
    (peer_session == Some(session)).then_some(session)
}

/// Fatia 5: eventos do plugin que viram "o Claude está pedindo algo".
pub fn asks_for_attention(event: &str) -> bool {
    matches!(event, "PermissionRequest" | "Notification")
}

/// O mesmo prompt dispara `PermissionRequest` e `Notification` quase juntos:
/// um aviso só por pane dentro desta janela.
pub const ASK_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(10);

pub fn ask_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= ASK_DEBOUNCE)
}

type AskLog = std::collections::HashMap<crate::session::SessionId, std::time::Instant>;

static LAST_ASK: std::sync::Mutex<Option<AskLog>> = std::sync::Mutex::new(None);

/// Reserva o aviso deste pane, se já passou a janela do último.
pub fn take_ask_slot(session: crate::session::SessionId, now: std::time::Instant) -> bool {
    let Ok(mut guard) = LAST_ASK.lock() else {
        return true;
    };
    let log = guard.get_or_insert_with(Default::default);
    if !ask_due(log.get(&session).copied(), now) {
        return false;
    }
    log.retain(|_, at| now.saturating_duration_since(*at) < ASK_DEBOUNCE);
    log.insert(session, now);
    true
}

/// Comparação em tempo constante para o token — não vaza por tempo quantos
/// caracteres batem.
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionId;

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
        // Sem repositório (branch desconhecida): sem decisão.
        assert_eq!(
            guard_rail("git push", Path::new("/x"), &|_: &Path| None),
            None
        );
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
        // Pasta desconhecida volta para a da sessão.
        assert_eq!(guard("cd /repo/main && cd - && git push"), None);
        assert_eq!(guard("cd /repo/main && cd && git push"), None);
        assert_eq!(on_main("cd ~/x && git push"), Some(PUSH_MAIN_MESSAGE));
        assert_eq!(on_main("cd $REPO && git push"), Some(PUSH_MAIN_MESSAGE));
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

    #[test]
    fn hooks_json_never_blocks_on_failure_and_covers_the_events() {
        let v = hooks_json();
        let text = v.to_string();
        assert!(
            !text.contains("onFailure"),
            "hook global nunca falha fechado"
        );
        for event in OBSERVED_EVENTS.iter().chain(&["PreToolUse"]) {
            assert!(v["hooks"][event].is_array(), "{event}");
        }
        assert_eq!(
            v["hooks"]["SessionStart"][0]["matcher"],
            "startup|resume|clear|compact|fork"
        );
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"],
            "\"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh\"",
            "caminho do plugin entre aspas"
        );
    }

    #[test]
    fn install_commands_are_argv_with_the_user_scope() {
        let cmds = install_commands(Path::new("/x/claude"), Path::new("/data/tyba mkt"));
        let as_str: Vec<Vec<String>> = cmds
            .iter()
            .map(|c| c.iter().map(|a| a.to_string_lossy().into_owned()).collect())
            .collect();
        assert_eq!(
            as_str[0],
            [
                "/x/claude",
                "plugin",
                "marketplace",
                "add",
                "/data/tyba mkt",
                "--scope",
                "user"
            ]
        );
        assert_eq!(
            as_str[1],
            [
                "/x/claude",
                "plugin",
                "install",
                "tyba@tyba",
                "--scope",
                "user"
            ]
        );
        let un: Vec<Vec<String>> = uninstall_commands(Path::new("/x/claude"))
            .iter()
            .map(|c| c.iter().map(|a| a.to_string_lossy().into_owned()).collect())
            .collect();
        assert_eq!(un[0], ["/x/claude", "plugin", "uninstall", "tyba@tyba"]);
        assert_eq!(
            un[1],
            ["/x/claude", "plugin", "marketplace", "remove", "tyba"]
        );
    }

    /// O instalador roda os comandos como argv, em ordem, e para no primeiro
    /// que falha — com um `claude` falso num HOME temporário, nunca o de
    /// verdade desta máquina.
    #[test]
    fn run_commands_runs_in_order_and_stops_at_the_first_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv.log");
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{log}'\necho >> '{log}'\n[ \"$2\" = install ] && {{ echo 'falhou token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789' >&2; exit 3; }}\nexit 0\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mkt = dir.path().join("mkt dir");

        let err = run_commands(&install_commands(&fake, &mkt), None).unwrap_err();
        let lines: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            lines[0],
            format!("plugin|marketplace|add|{}|--scope|user|", mkt.display())
        );
        assert_eq!(lines[1], "plugin|install|tyba@tyba|--scope|user|");
        assert!(err.starts_with("falhou token"), "{err}");
        assert!(
            !err.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            "o stderr do claude passa pela redação: {err}"
        );

        std::fs::remove_file(&log).unwrap();
        run_commands(&uninstall_commands(&fake), None).expect("desinstala");
        let lines = std::fs::read_to_string(&log).unwrap();
        assert!(lines.contains("plugin|uninstall|tyba@tyba|"));
        assert!(lines.contains("plugin|marketplace|remove|tyba|"));
    }

    /// Pelo chip, o `claude plugin …` roda com o `CLAUDE_CONFIG_DIR` do
    /// claude do pane.
    #[test]
    fn run_commands_passes_the_pane_config_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("env.log");
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho \"${{CLAUDE_CONFIG_DIR:-}}\" >> '{log}'\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        run_commands(&uninstall_commands(&fake), Some("/tmp/cfg-do-pane")).unwrap();
        let seen = std::fs::read_to_string(&log).unwrap();
        assert!(seen.lines().all(|l| l == "/tmp/cfg-do-pane"), "{seen}");
    }

    #[test]
    fn hooks_blocked_detects_both_settings() {
        assert_eq!(hooks_blocked(None, None), None);
        let off = json!({"disableAllHooks": true});
        assert_eq!(hooks_blocked(Some(&off), None), Some("disable_all_hooks"));
        assert_eq!(hooks_blocked(None, Some(&off)), Some("disable_all_hooks"));
        let managed = json!({"allowManagedHooksOnly": true});
        assert_eq!(
            hooks_blocked(None, Some(&managed)),
            Some("managed_hooks_only")
        );
        // `allowManagedHooksOnly` só vale na config gerenciada.
        assert_eq!(hooks_blocked(Some(&managed), None), None);
    }

    #[test]
    fn chip_reads_the_config_dir_of_the_pane_claude() {
        let home = tempfile::tempdir().unwrap();
        let work = home.path().join(".claude-work");
        std::fs::create_dir_all(work.join("plugins")).unwrap();
        std::fs::write(
            work.join("plugins/installed_plugins.json"),
            r#"{"plugins":{"tyba@tyba":[]}}"#,
        )
        .unwrap();
        // Instalado no config do pane: sem chip. O ~/.claude padrão (vazio)
        // pediria o chip.
        assert!(!suggest_in(
            &claude_config_dir(work.to_str(), home.path()),
            None
        ));
        assert!(suggest_in(&claude_config_dir(None, home.path()), None));
    }

    #[test]
    fn permission_and_notification_ask_once_per_window() {
        use std::time::{Duration, Instant};
        assert!(asks_for_attention("PermissionRequest"));
        assert!(asks_for_attention("Notification"));
        assert!(!asks_for_attention("Stop"));
        assert!(!asks_for_attention("SessionStart"));
        let at = Instant::now();
        assert!(ask_due(None, at));
        assert!(!ask_due(Some(at), at + Duration::from_secs(2)));
        assert!(ask_due(Some(at), at + ASK_DEBOUNCE));

        let pane = crate::session::SessionId::new_v4();
        let other = crate::session::SessionId::new_v4();
        assert!(take_ask_slot(pane, at));
        assert!(!take_ask_slot(pane, at + Duration::from_secs(1)));
        assert!(take_ask_slot(other, at + Duration::from_secs(1)));
        assert!(take_ask_slot(pane, at + ASK_DEBOUNCE));
    }

    #[test]
    fn chip_only_when_missing_and_hooks_run() {
        assert!(should_suggest(false, None));
        assert!(!should_suggest(true, None));
        assert!(!should_suggest(false, Some("disable_all_hooks")));
        assert!(!should_suggest(false, Some("managed_hooks_only")));
    }

    #[test]
    fn is_installed_reads_the_registry() {
        assert!(is_installed(Some(
            &json!({"version": 2, "plugins": {"tyba@tyba": []}})
        )));
        assert!(!is_installed(Some(
            &json!({"version": 2, "plugins": {"warp@x": []}})
        )));
        assert!(!is_installed(None));
    }

    #[test]
    fn config_dir_honors_claude_config_dir() {
        let home = Path::new("/Users/x");
        assert_eq!(claude_config_dir(None, home), home.join(".claude"));
        assert_eq!(claude_config_dir(Some(""), home), home.join(".claude"));
        assert_eq!(
            claude_config_dir(Some("/tmp/cfg"), home),
            PathBuf::from("/tmp/cfg")
        );
    }

    #[test]
    fn tool_summary_is_short_and_redacted() {
        let input = json!({"command": "curl -H 'Authorization: Bearer sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789' x"});
        let s = tool_summary("Bash", Some(&input));
        assert!(s.starts_with("Bash: curl"));
        assert!(
            !s.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            "{s}"
        );
        let long = json!({"command": "x".repeat(5000)});
        assert!(tool_summary("Bash", Some(&long)).chars().count() <= 400);
    }

    #[test]
    fn token_must_match_and_the_peer_must_be_in_that_pane() {
        let pane = SessionId::new_v4();
        let other = SessionId::new_v4();
        let lookup = |t: &str| token_eq(t, "segredo-do-pane").then_some(pane);

        assert_eq!(
            resolve_plugin_event("segredo-do-pane", Some(pane), lookup),
            Some(pane)
        );
        assert_eq!(resolve_plugin_event("errado", Some(pane), lookup), None);
        assert_eq!(
            resolve_plugin_event("segredo-do-pane", Some(other), lookup),
            None,
            "token certo vindo de outro pane é recusado"
        );
        assert_eq!(resolve_plugin_event("segredo-do-pane", None, lookup), None);
    }

    #[test]
    fn token_eq_is_exact() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
        assert!(!token_eq("", "a"));
    }

    #[test]
    fn write_marketplace_lays_out_the_documented_structure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write_marketplace(dir.path()).unwrap();
        let mkt = read_json(&dir.path().join(".claude-plugin/marketplace.json")).unwrap();
        assert_eq!(mkt["plugins"][0]["source"], "./plugin");
        assert!(dir
            .path()
            .join("plugin/.claude-plugin/plugin.json")
            .is_file());
        assert!(dir.path().join("plugin/hooks/hooks.json").is_file());
        let script = dir.path().join("plugin/scripts/hook.sh");
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    /// Roda o `hook.sh` de verdade num `sh`, com um `tyba` falso que registra
    /// se foi chamado. Os três casos de env do M6-B.
    fn run_hook(envs: &[(&str, &str)]) -> (i32, bool) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, HOOK_SCRIPT).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log = dir.path().join("called");
        let fake = dir.path().join("tyba");
        std::fs::write(
            &fake,
            format!("#!/bin/sh\necho \"$@\" > '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cmd = std::process::Command::new(&script);
        cmd.env_clear().env("PATH", "/usr/bin:/bin");
        for (k, v) in envs {
            let v = if *v == "$FAKE" {
                fake.to_str().unwrap()
            } else {
                v
            };
            cmd.env(k, v);
        }
        let out = cmd
            .stdin(std::process::Stdio::null())
            .output()
            .expect("hook roda");
        assert!(out.stdout.is_empty(), "o hook.sh nunca escreve no stdout");
        (out.status.code().unwrap_or(-1), log.exists())
    }

    #[test]
    fn hook_script_is_a_silent_no_op_outside_tyba() {
        assert_eq!(run_hook(&[("TYBA_BIN", "$FAKE")]), (0, false));
    }

    #[test]
    fn hook_script_is_a_no_op_in_an_already_gated_claude() {
        let base = [("TYBA_BIN", "$FAKE"), ("TYBA_PANE_TOKEN", "t")];
        let mut hosted = base.to_vec();
        hosted.push(("TYBA_HOSTED", "1"));
        assert_eq!(run_hook(&hosted), (0, false));
        let mut managed = base.to_vec();
        managed.push(("TYBA_HOOK_SOCKET", "/tmp/x.sock"));
        assert_eq!(run_hook(&managed), (0, false));
    }

    #[test]
    fn hook_script_hands_a_raw_claude_in_a_pane_to_tyba() {
        assert_eq!(
            run_hook(&[("TYBA_BIN", "$FAKE"), ("TYBA_PANE_TOKEN", "t")]),
            (0, true)
        );
        // Sem o binário do TYBA (app movido), sai 0 e não trava o Claude.
        assert_eq!(
            run_hook(&[("TYBA_BIN", "/nao/existe"), ("TYBA_PANE_TOKEN", "t")]),
            (0, false)
        );
    }
}
