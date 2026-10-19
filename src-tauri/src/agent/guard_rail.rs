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
        "git" => git_push_verdict(args, dir, current_branch),
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
///
/// Falha FECHADO: sem refspec (ou com `HEAD`/`@`) e a branch atual ilegível
/// (git que não responde, HEAD destacado, pasta sem repo), recusa com
/// [`BRANCH_UNKNOWN_MESSAGE`] — o destino pode ser a main.
fn git_push_verdict(
    args: &[String],
    cwd: &Path,
    current_branch: &dyn Fn(&Path) -> Option<String>,
) -> Option<&'static str> {
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
        return None;
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
        return None;
    }
    if all {
        return Some(PUSH_MAIN_MESSAGE);
    }
    let current = || match current_branch(&dir) {
        Some(b) if is_main_ref(&b) => Some(PUSH_MAIN_MESSAGE),
        Some(_) => None,
        None => Some(BRANCH_UNKNOWN_MESSAGE),
    };
    let refspecs = positional.get(1..).unwrap_or(&[]);
    if refspecs.is_empty() {
        return if tags { None } else { current() };
    }
    refspecs.iter().find_map(|spec| {
        let spec = spec.trim_start_matches('+');
        let dst = spec.rsplit_once(':').map_or(spec, |(_, d)| d);
        if dst == "HEAD" || dst == "@" {
            current()
        } else {
            is_main_ref(dst).then_some(PUSH_MAIN_MESSAGE)
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

/// A branch atual de `dir`, lida do `HEAD` do repositório — sem rodar git:
/// nenhum filtro, `fsmonitor` ou jaula (Linux sem userns) no caminho, e o
/// mesmo código no Windows. Sobe as pastas até achar o `.git` (pasta, ou o
/// arquivo `gitdir:` de um worktree). Também responde numa branch ainda sem
/// commit. `None` (sem repo, HEAD destacado, leitura falhou) faz o push que
/// depende dela ser recusado. Só roda para `git push` sem refspec.
fn current_branch(dir: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(here) = at {
        let dotgit = here.join(".git");
        let git_dir = if dotgit.is_dir() {
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
        return head
            .trim()
            .strip_prefix("ref: refs/heads/")
            .filter(|b| !b.is_empty())
            .map(str::to_string);
    }
    None
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
