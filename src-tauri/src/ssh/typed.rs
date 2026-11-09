//! O `ssh` DIGITADO num shell local: dado o comando de um bloco, decide se é
//! uma sessão interativa e para onde ela vai.
//!
//! Peça pura, sem processo e sem disco: quem a chama é a detecção do bloco
//! (o comando chega pelo OSC 133/633 do shell integration). O resultado só
//! alimenta um aviso informativo — nada aqui vira ação no PTY.
//!
//! Alias e função de shell não são detectados (como no Warp): o comando que
//! chega é o que o dono digitou, e `s prod` não diz que `s` é `ssh`.

use serde::Serialize;

/// Para onde um `ssh` interativo vai, do jeito que a linha disse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedDestination {
    /// O host como digitado: alias do `~/.ssh/config`, nome ou IP.
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

/// Flags do OpenSSH sem argumento.
const NO_ARG: &str = "46AaCfGgKkMNnqsTtVvXxYy";
/// Flags do OpenSSH que levam argumento (`-p 22` ou `-p22`).
const WITH_ARG: &str = "BbcDEeFIiJLlmOoPpQRSWw";
/// Flags que tiram a sessão interativa: túnel sem shell (`-N`), fundo (`-f`),
/// stdio (`-W`), controle do master (`-O`), sem tty (`-T`), só imprimir
/// config/versão/lista (`-G`/`-V`/`-Q`), stdin nulo (`-n`), subsistema (`-s`).
const NOT_INTERACTIVE: &str = "NfWOTGVQns";

/// O destino de um `ssh` interativo, ou `None` quando a linha não é uma
/// sessão de trabalho no host (comando remoto, túnel, `-N`, pipe, `&&`…).
///
/// Na dúvida, `None`: o falso negativo custa um aviso que não aparece; o
/// falso positivo, um aviso sobre um host que não é o da sessão.
pub fn parse(command: &str) -> Option<TypedDestination> {
    let words = words(command)?;
    let mut rest = words.as_slice();
    while rest.first().is_some_and(|w| is_assignment(w)) {
        rest = &rest[1..];
    }
    if rest.first().is_some_and(|w| w == "command") {
        rest = &rest[1..];
    }
    let (bin, args) = rest.split_first()?;
    if bin != "ssh" && !bin.ends_with("/ssh") {
        return None;
    }

    let mut acc = Acc::default();
    let mut host: Option<String> = None;
    // O OpenSSH reabre o getopt depois do host (`ssh host -p 22` vale), e só
    // o `--` encerra as opções de vez.
    let mut options_done = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if !options_done && arg == "--" {
            options_done = true;
            continue;
        }
        if !options_done && arg.len() > 1 && arg.starts_with('-') {
            let flags: Vec<char> = arg[1..].chars().collect();
            for (at, &flag) in flags.iter().enumerate() {
                if NOT_INTERACTIVE.contains(flag) {
                    return None;
                }
                if NO_ARG.contains(flag) {
                    continue;
                }
                if !WITH_ARG.contains(flag) {
                    return None;
                }
                let glued: String = flags[at + 1..].iter().collect();
                let value = if glued.is_empty() {
                    i += 1;
                    args.get(i - 1)?.clone()
                } else {
                    glued
                };
                acc.flag(flag, value)?;
                break;
            }
            continue;
        }
        if host.is_some() {
            // Comando remoto: `ssh host uptime` roda e volta, não é sessão.
            return None;
        }
        host = Some(acc.destination(arg)?);
    }

    Some(TypedDestination {
        host: host?,
        user: acc.user,
        port: acc.port,
        identity_file: acc.identity_file,
        proxy_jump: acc.proxy_jump,
    })
}

/// O que as opções disseram até aqui. O primeiro valor vence, como no
/// OpenSSH (`-l a b@host` entra como `a`).
#[derive(Default)]
struct Acc {
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
    proxy_jump: Option<String>,
}

impl Acc {
    fn flag(&mut self, flag: char, value: String) -> Option<()> {
        match flag {
            'p' => set_port(&mut self.port, &value)?,
            'l' => set_first(&mut self.user, value),
            'i' => set_first(&mut self.identity_file, value),
            'J' => set_first(&mut self.proxy_jump, value),
            'o' => self.option(&value)?,
            _ => {}
        }
        Some(())
    }

    /// `-o Chave=valor` ou `-o 'Chave valor'`. Chave sem diferença de caixa.
    fn option(&mut self, raw: &str) -> Option<()> {
        let split = raw.find(|c: char| c == '=' || c.is_whitespace())?;
        let key = raw[..split].to_ascii_lowercase();
        let value = raw[split..].trim_start_matches(|c: char| c == '=' || c.is_whitespace());
        if value.is_empty() {
            return None;
        }
        let is = |want: &str| value.eq_ignore_ascii_case(want);
        match key.as_str() {
            "port" => set_port(&mut self.port, value)?,
            "user" => set_first(&mut self.user, value.to_string()),
            "identityfile" => set_first(&mut self.identity_file, value.to_string()),
            "proxyjump" => set_first(&mut self.proxy_jump, value.to_string()),
            "remotecommand" if !is("none") => return None,
            "requesttty" if is("no") => return None,
            "sessiontype" if !is("default") => return None,
            "forkafterauthentication" | "stdinnull" if is("yes") => return None,
            _ => {}
        }
        Some(())
    }

    /// `[user@]host` ou `ssh://[user@]host[:port]`; devolve o host.
    fn destination(&mut self, raw: &str) -> Option<String> {
        let (uri, rest) = match raw.strip_prefix("ssh://") {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        let host_part = match rest.rsplit_once('@') {
            Some((user, host)) => {
                if user.is_empty() {
                    return None;
                }
                set_first(&mut self.user, user.to_string());
                host
            }
            None => rest,
        };
        let host = if !uri {
            host_part
        } else if let Some(bracketed) = host_part.strip_prefix('[') {
            let (host, after) = bracketed.split_once(']')?;
            if !after.is_empty() {
                set_port(&mut self.port, after.strip_prefix(':')?)?;
            }
            host
        } else {
            match host_part.split_once(':') {
                Some((host, port)) => {
                    set_port(&mut self.port, port)?;
                    host
                }
                None => host_part,
            }
        };
        (!host.is_empty()).then(|| host.to_string())
    }
}

fn set_first(slot: &mut Option<String>, value: String) {
    if slot.is_none() {
        *slot = Some(value);
    }
}

/// Porta inválida é linha que o `ssh` recusaria: `None` derruba a detecção.
fn set_port(slot: &mut Option<u16>, raw: &str) -> Option<()> {
    let port: u16 = raw.parse().ok().filter(|p| *p > 0)?;
    if slot.is_none() {
        *slot = Some(port);
    }
    Some(())
}

/// `NOME=valor` antes do comando: atribuição de ambiente, não o binário.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Separa a linha em palavras como o shell faria, sem expandir nada.
///
/// `None` quando a linha tem o que só o shell resolve — expansão (`$`,
/// crase, glob) — ou o que a torna mais de um comando (pipe, redirect,
/// `;`, `&`, subshell, quebra de linha), e quando o quoting não fecha.
fn words(line: &str) -> Option<Vec<String>> {
    #[derive(PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = Quote::None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                } else {
                    word.push(c);
                }
            }
            Quote::Double => match c {
                '"' => quote = Quote::None,
                '$' | '`' => return None,
                '\\' => {
                    let next = chars.next()?;
                    if !matches!(next, '"' | '\\' | '$' | '`') {
                        word.push('\\');
                    }
                    word.push(next);
                }
                _ => word.push(c),
            },
            Quote::None => match c {
                ' ' | '\t' => {
                    if in_word {
                        out.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                '\'' => {
                    quote = Quote::Single;
                    in_word = true;
                }
                '"' => {
                    quote = Quote::Double;
                    in_word = true;
                }
                '\\' => {
                    word.push(chars.next()?);
                    in_word = true;
                }
                '|' | '&' | ';' | '<' | '>' | '(' | ')' | '\n' | '\r' | '$' | '`' | '*' | '?' => {
                    return None
                }
                _ => {
                    word.push(c);
                    in_word = true;
                }
            },
        }
    }
    if quote != Quote::None {
        return None;
    }
    if in_word {
        out.push(word);
    }
    Some(out)
}

/// Teto de bytes que a confirmação de login olha. Passou disso sem cara de
/// prompt, o `ssh` vira um bloco comum: melhor não avisar do que avisar
/// errado.
const WATCH_MAX_BYTES: usize = 64 * 1024;
/// Teto de tempo desde o `133;C`: cabe senha, passphrase e segundo fator
/// digitados à mão.
const WATCH_MAX_MS: i64 = 120_000;

/// O que a observação da saída concluiu até aqui.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    Pending,
    /// `Last login:` ou uma linha com cara de prompt remoto.
    Confirmed,
    /// Estourou um dos tetos: desistiu, de vez.
    GaveUp,
}

/// Observa a saída de um `ssh` digitado até ele ter cara de logado.
///
/// Heurística, como a do Warp: a saída do servidor só decide se o aviso
/// informativo aparece, nunca vira ação. Nada do que passa aqui é guardado
/// além da linha corrente.
pub struct LoginWatch {
    started_ms: i64,
    seen: usize,
    line: String,
    esc: Scan,
}

impl LoginWatch {
    pub fn new(started_ms: i64) -> Self {
        Self {
            started_ms,
            seen: 0,
            line: String::new(),
            esc: Scan::Ground,
        }
    }

    pub fn feed(&mut self, bytes: &[u8], now_ms: i64) -> Login {
        if self.seen > WATCH_MAX_BYTES || now_ms - self.started_ms > WATCH_MAX_MS {
            return self.give_up();
        }
        self.seen += bytes.len();
        for ch in String::from_utf8_lossy(bytes).chars() {
            if !self.visible(ch) {
                continue;
            }
            match ch {
                '\n' | '\r' => {
                    if is_last_login(&self.line) {
                        return Login::Confirmed;
                    }
                    self.line.clear();
                }
                c if c.is_control() => {}
                // A linha corrente basta; uma linha gigante sem quebra não
                // tem cara de prompt e não pode crescer sem teto.
                c if self.line.len() < 512 => self.line.push(c),
                _ => {}
            }
        }
        if is_last_login(&self.line) || looks_like_prompt(&self.line) {
            return Login::Confirmed;
        }
        if self.seen > WATCH_MAX_BYTES {
            return self.give_up();
        }
        Login::Pending
    }

    fn give_up(&mut self) -> Login {
        self.seen = WATCH_MAX_BYTES + 1;
        self.line.clear();
        Login::GaveUp
    }

    /// Filtra escapes (CSI, OSC, DCS…) com estado entre chunks: o que sobra
    /// é o texto que o terminal desenharia.
    fn visible(&mut self, ch: char) -> bool {
        let (next, keep) = match (self.esc, ch) {
            (Scan::Str, '\x07') => (Scan::Ground, false),
            (Scan::Str, '\x1b') => (Scan::StrEscape, false),
            (Scan::Str, _) => (Scan::Str, false),
            (Scan::StrEscape, _) => (Scan::Ground, false),
            (_, '\x1b') => (Scan::Escape, false),
            (Scan::Escape, '[') => (Scan::Csi, false),
            (Scan::Escape, ']' | 'P' | 'X' | '^' | '_') => (Scan::Str, false),
            (Scan::Escape, _) => (Scan::Ground, false),
            (Scan::Csi, '\u{40}'..='\u{7e}') => (Scan::Ground, false),
            (Scan::Csi, _) => (Scan::Csi, false),
            (Scan::Ground, _) => (Scan::Ground, true),
        };
        self.esc = next;
        keep
    }
}

fn is_last_login(line: &str) -> bool {
    line.trim_start().starts_with("Last login:")
}

/// A linha em aberto (sem quebra depois) termina como um prompt de shell
/// termina, e não é uma das perguntas do próprio `ssh`.
fn looks_like_prompt(line: &str) -> bool {
    let trimmed = line.trim_end();
    let Some(last) = trimmed.chars().last() else {
        return false;
    };
    if !matches!(last, '$' | '#' | '%' | '>' | '❯' | '➜' | '»') {
        return false;
    }
    let lower = trimmed.to_lowercase();
    ![
        "password",
        "passphrase",
        "yes/no",
        "verification",
        "code:",
        "otp",
        "token",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// Onde o filtro de escapes está, entre um chunk e outro.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    Ground,
    Escape,
    Csi,
    /// Corpo de OSC/DCS/APC/PM/SOS — termina em `BEL` ou `ST`.
    Str,
    StrEscape,
}

/// Um `ssh` digitado que logou: para onde, e qual bloco (o `started_at_ms`
/// do comando em execução — o bloco vivo ainda não tem id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub destination: TypedDestination,
    pub block_started_at_ms: Option<i64>,
}

/// O ciclo do comando de UMA sessão, do ponto de vista da detecção: arma no
/// `ssh` interativo, observa, avisa uma vez, e desarma quando o comando acaba.
#[derive(Default)]
pub struct TypedSshTracker {
    armed: Option<Armed>,
    /// O `started_at_ms` do último comando visto rodando.
    last_started: Option<i64>,
}

struct Armed {
    destination: TypedDestination,
    started_at_ms: Option<i64>,
    watch: LoginWatch,
}

impl TypedSshTracker {
    /// O estado do comando, como a captura o anuncia (`CommandState`).
    ///
    /// O mesmo comando anunciado de novo (mesmo `started_at_ms`) não rearma:
    /// um `ssh` que já avisou não avisa outra vez.
    pub fn on_command(
        &mut self,
        running: bool,
        command: Option<&str>,
        started_at_ms: Option<i64>,
        now_ms: i64,
    ) {
        if !running {
            self.armed = None;
            self.last_started = None;
            return;
        }
        if self.last_started.is_some() && self.last_started == started_at_ms {
            return;
        }
        self.last_started = started_at_ms;
        self.armed = command.and_then(parse).map(|destination| Armed {
            destination,
            started_at_ms,
            watch: LoginWatch::new(now_ms),
        });
    }

    /// Saída do comando armado. `Some` uma única vez, no login.
    pub fn feed(&mut self, bytes: &[u8], now_ms: i64) -> Option<Detected> {
        let armed = self.armed.as_mut()?;
        match armed.watch.feed(bytes, now_ms) {
            Login::Pending => None,
            Login::GaveUp => {
                self.armed = None;
                None
            }
            Login::Confirmed => {
                let armed = self.armed.take()?;
                Some(Detected {
                    destination: armed.destination,
                    block_started_at_ms: armed.started_at_ms,
                })
            }
        }
    }
}

/// O aviso que o core emite quando um `ssh` digitado loga. Informativo: a
/// tela oferece "Abrir como sessão TYBA"/"Cadastrar", e quem age é o dono.
pub const EVENT_DETECTED: &str = "ssh-typed://detected";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedEvent {
    pub session_id: String,
    /// O bloco vivo, pelo `started_at_ms` que o `CommandState` já entrega à
    /// tela: o id do bloco só nasce quando ele termina.
    pub block_started_at_ms: Option<i64>,
    pub destination: TypedDestination,
    /// `Some` quando casou com um Host cadastrado (`Registered`).
    pub host_id: Option<String>,
}

impl DetectedEvent {
    /// `None` quando o aviso fica quieto (ver [`resolve`]).
    pub fn build(session_id: String, detected: Detected, hosts: &[super::Host]) -> Option<Self> {
        let host_id = match resolve(detected.destination.clone(), hosts)? {
            TypedMatch::Registered(id) => Some(id),
            TypedMatch::Unregistered(_) => None,
        };
        // TODO(ssh-digitado): a lista de hosts negados ("Nunca para este
        // host", Settings > SSH > Integração) ainda não existe. Quando entrar,
        // é aqui que um destino negado devolve `None`.
        Some(Self {
            session_id,
            block_started_at_ms: detected.block_started_at_ms,
            destination: detected.destination,
            host_id,
        })
    }
}

/// O `ssh` digitado casado com o cadastro do gestor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedMatch {
    Registered(String),
    Unregistered(TypedDestination),
}

/// Casa o destino com um Host cadastrado — por alias (o nome materializado
/// no `tyba.conf`) ou por hostname, com usuário e porta compatíveis.
///
/// `None` quando o Host casado tem a integração desligada (regra 11): o dono
/// já disse que não quer o TYBA ali, e o aviso fica quieto.
///
/// Só o cadastro do TYBA: o `~/.ssh/config` do dono não é lido. Um alias que
/// só existe lá cai em `Unregistered`, que é o caso do "Cadastrar".
pub fn resolve(destination: TypedDestination, hosts: &[super::Host]) -> Option<TypedMatch> {
    let user_ok = |h: &super::Host| match (&destination.user, &h.username) {
        (Some(typed), Some(registered)) => typed == registered,
        _ => true,
    };
    // Pelo alias, sem `-p` vale a porta do `tyba.conf`; pelo hostname, o
    // bloco do alias não se aplica e sem `-p` é a 22.
    let by_alias = |h: &&super::Host| {
        h.alias == destination.host
            && user_ok(h)
            && destination.port.is_none_or(|p| p == h.port.unwrap_or(22))
    };
    let by_hostname = |h: &&super::Host| {
        h.hostname.eq_ignore_ascii_case(&destination.host)
            && user_ok(h)
            && destination.port.unwrap_or(22) == h.port.unwrap_or(22)
    };
    let found = hosts
        .iter()
        .find(by_alias)
        .or_else(|| hosts.iter().find(by_hostname));
    match found {
        Some(host) if !host.integration_enabled => None,
        Some(host) => Some(TypedMatch::Registered(host.id.clone())),
        None => Some(TypedMatch::Unregistered(destination)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dest(host: &str) -> TypedDestination {
        TypedDestination {
            host: host.into(),
            user: None,
            port: None,
            identity_file: None,
            proxy_jump: None,
        }
    }

    fn with_user(host: &str, user: &str) -> TypedDestination {
        TypedDestination {
            user: Some(user.into()),
            ..dest(host)
        }
    }

    #[test]
    fn interactive_lines_yield_the_destination() {
        let cases: Vec<(&str, TypedDestination)> = vec![
            ("ssh host", dest("host")),
            ("ssh web-01.example.test", dest("web-01.example.test")),
            ("ssh 192.0.2.10", dest("192.0.2.10")),
            ("ssh root@192.0.2.10", with_user("192.0.2.10", "root")),
            ("ssh john.doe@srv", with_user("srv", "john.doe")),
            ("  ssh host  ", dest("host")),
            ("ssh\thost", dest("host")),
            (
                "ssh -p 2222 deploy@host",
                TypedDestination {
                    port: Some(2222),
                    ..with_user("host", "deploy")
                },
            ),
            (
                "ssh -p2222 host",
                TypedDestination {
                    port: Some(2222),
                    ..dest("host")
                },
            ),
            ("ssh -l deploy host", with_user("host", "deploy")),
            ("ssh -ldeploy host", with_user("host", "deploy")),
            (
                "ssh -i ~/.ssh/key host",
                TypedDestination {
                    identity_file: Some("~/.ssh/key".into()),
                    ..dest("host")
                },
            ),
            (
                "ssh -J bastion deploy@app",
                TypedDestination {
                    proxy_jump: Some("bastion".into()),
                    ..with_user("app", "deploy")
                },
            ),
            ("ssh -A -X -v -C host", dest("host")),
            ("ssh -AXY host", dest("host")),
            ("ssh -t host", dest("host")),
            ("ssh -tt host", dest("host")),
            ("ssh -4 -6 -q -K -k -a -x -y -g -M host", dest("host")),
            ("ssh -o StrictHostKeyChecking=no host", dest("host")),
            ("ssh -oServerAliveInterval=30 host", dest("host")),
            ("ssh -o 'ServerAliveInterval 30' host", dest("host")),
            (
                "ssh -o Port=2200 -o User=ops host",
                TypedDestination {
                    port: Some(2200),
                    ..with_user("host", "ops")
                },
            ),
            ("ssh -o user=ops host", with_user("host", "ops")),
            (
                "ssh -o ProxyJump=bastion host",
                TypedDestination {
                    proxy_jump: Some("bastion".into()),
                    ..dest("host")
                },
            ),
            (
                "ssh -o IdentityFile=/k host",
                TypedDestination {
                    identity_file: Some("/k".into()),
                    ..dest("host")
                },
            ),
            (
                "ssh -o 'ProxyCommand=ssh -W %h:%p bastion' host",
                dest("host"),
            ),
            (
                "ssh -F /tmp/cfg -E /tmp/log -c aes128-ctr -m hmac-sha2-256 host",
                dest("host"),
            ),
            ("ssh -L 8080:localhost:80 host", dest("host")),
            ("ssh -R 9000:localhost:9000 -D 1080 host", dest("host")),
            (
                "ssh -b 192.0.2.1 -e none -I pkcs -w 0:1 -B en0 -P tag -S none host",
                dest("host"),
            ),
            ("ssh -- host", dest("host")),
            (
                "ssh -p 22 -- deploy@host",
                TypedDestination {
                    port: Some(22),
                    ..with_user("host", "deploy")
                },
            ),
            // Quoting e escapes.
            ("ssh 'host'", dest("host")),
            ("ssh \"deploy@host\"", with_user("host", "deploy")),
            ("ssh de'pl'oy@host", with_user("host", "deploy")),
            ("ssh deploy\\@host", with_user("host", "deploy")),
            (
                "ssh -i \"/Users/me/My Keys/id\" host",
                TypedDestination {
                    identity_file: Some("/Users/me/My Keys/id".into()),
                    ..dest("host")
                },
            ),
            (
                "ssh -i /Users/me/My\\ Keys/id host",
                TypedDestination {
                    identity_file: Some("/Users/me/My Keys/id".into()),
                    ..dest("host")
                },
            ),
            // Prefixos que ainda são o binário.
            ("command ssh host", dest("host")),
            ("/usr/bin/ssh host", dest("host")),
            (
                "/opt/homebrew/bin/ssh deploy@host",
                with_user("host", "deploy"),
            ),
            ("\\ssh host", dest("host")),
            ("'ssh' host", dest("host")),
            ("TERM=xterm-256color ssh host", dest("host")),
            ("LANG=C LC_ALL=C command ssh host", dest("host")),
            // Opções depois do host: o OpenSSH reabre o getopt ali.
            (
                "ssh host -p 2222",
                TypedDestination {
                    port: Some(2222),
                    ..dest("host")
                },
            ),
            ("ssh host -l deploy", with_user("host", "deploy")),
            ("ssh host -A -t", dest("host")),
            // O primeiro vence, como no OpenSSH.
            ("ssh -l first second@host", with_user("host", "first")),
            ("ssh first@host -l second", with_user("host", "first")),
            (
                "ssh -p 1 -p 2 host",
                TypedDestination {
                    port: Some(1),
                    ..dest("host")
                },
            ),
            (
                "ssh -p 2201 -o Port=2202 host",
                TypedDestination {
                    port: Some(2201),
                    ..dest("host")
                },
            ),
            // `@` dentro do usuário: quem separa é o último.
            ("ssh me@corp@host", with_user("host", "me@corp")),
            // URI.
            ("ssh ssh://host", dest("host")),
            ("ssh ssh://deploy@host", with_user("host", "deploy")),
            (
                "ssh ssh://deploy@host:2222",
                TypedDestination {
                    port: Some(2222),
                    ..with_user("host", "deploy")
                },
            ),
            (
                "ssh ssh://[2001:db8::1]:22",
                TypedDestination {
                    port: Some(22),
                    ..dest("2001:db8::1")
                },
            ),
            ("ssh deploy@2001:db8::1", with_user("2001:db8::1", "deploy")),
            // Opções que não mudam o caráter interativo.
            ("ssh -o RequestTTY=yes host", dest("host")),
            ("ssh -o RemoteCommand=none host", dest("host")),
            ("ssh -o SessionType=default host", dest("host")),
            ("ssh -o ForkAfterAuthentication=no host", dest("host")),
        ];
        for (line, expected) in cases {
            assert_eq!(parse(line), Some(expected), "linha: {line:?}");
        }
    }

    #[test]
    fn non_interactive_or_unknowable_lines_yield_nothing() {
        let cases = [
            // Não é ssh.
            "",
            "   ",
            "ls -la",
            "scp a host:/b",
            "sshfs host:/ /mnt",
            "ssh-add -l",
            "ssh-keygen -t ed25519",
            "rsync -e ssh a host:b",
            "echo ssh host",
            "sudo ssh host",
            "git push",
            "s host",
            // ssh sem destino.
            "ssh",
            "ssh -v",
            "ssh -p 22",
            "ssh -p",
            "ssh -l",
            "ssh --",
            "ssh ''",
            "ssh @host",
            "ssh deploy@",
            "ssh ssh://",
            "ssh ssh://deploy@",
            "ssh ssh://host:notaport",
            "ssh ssh://[2001:db8::1",
            // Comando remoto depois do host.
            "ssh host uptime",
            "ssh host 'ls -la'",
            "ssh host -- ls",
            "ssh -t host htop",
            "ssh host -p 22 uptime",
            // Flags que tiram a sessão interativa.
            "ssh -N host",
            "ssh -fN -L 5432:db:5432 host",
            "ssh -f host",
            "ssh -W db:5432 bastion",
            "ssh -O check host",
            "ssh -Ocheck host",
            "ssh -T host",
            "ssh -G host",
            "ssh -V",
            "ssh -V host",
            "ssh -Q cipher",
            "ssh -n host",
            "ssh -s host sftp",
            "ssh host -N",
            "ssh -AN host",
            "ssh -o RemoteCommand=uptime host",
            "ssh -o 'RemoteCommand uptime' host",
            "ssh -o RequestTTY=no host",
            "ssh -o SessionType=none host",
            "ssh -o SessionType=subsystem host",
            "ssh -o ForkAfterAuthentication=yes host",
            "ssh -oforkafterauthentication=yes host",
            "ssh -o StdinNull=yes host",
            "ssh -o",
            // Pipe, redirect, encadeamento, fundo, subshell.
            "ssh host | tee log",
            "ssh host|tee log",
            "ssh host > out",
            "ssh host>out",
            "ssh host < in",
            "ssh host 2>&1",
            "ssh host && ls",
            "ssh host || true",
            "ssh host; ls",
            "ssh host &",
            "ls && ssh host",
            "(ssh host)",
            "{ ssh host; }",
            "ssh host\nls",
            // Expansão que só o shell resolve.
            "ssh $HOST",
            "ssh \"$HOST\"",
            "ssh ${HOST}",
            "ssh `hostname`",
            "ssh $(cat host)",
            "ssh -p $PORT host",
            "ssh web-0*",
            "ssh web-0?",
            // Quoting quebrado.
            "ssh 'host",
            "ssh \"host",
            "ssh host\\",
            // Porta inválida.
            "ssh -p 0 host",
            "ssh -p 70000 host",
            "ssh -p abc host",
            "ssh -o Port=abc host",
            // Flag que o OpenSSH não conhece.
            "ssh -Z host",
            "ssh --verbose host",
            // `command` com opção é outro comando.
            "command -v ssh",
            "command",
            "TERM=x",
        ];
        for line in cases {
            assert_eq!(parse(line), None, "linha: {line:?}");
        }
    }

    // --- Confirmação de login (fixtures sintéticas, escritas à mão) ---

    fn watch_all(chunks: &[&str]) -> Login {
        let mut watch = LoginWatch::new(0);
        let mut last = Login::Pending;
        for chunk in chunks {
            last = watch.feed(chunk.as_bytes(), 1_000);
            if last != Login::Pending {
                break;
            }
        }
        last
    }

    #[test]
    fn login_is_confirmed_by_last_login_or_a_remote_prompt() {
        let cases: &[&[&str]] = &[
            &["Last login: Mon Nov  9 10:00:00 2026 from 192.0.2.1\r\n"],
            &["Welcome to Ubuntu\r\n\r\nLast login: Mon Nov  9 from 192.0.2.1\r\n"],
            &["Last lo", "gin: Mon Nov  9\r\n"],
            &["deploy@web-01:~$ "],
            &["[root@vps ~]# "],
            &["host% "],
            &["\x1b[01;32mdeploy@web\x1b[00m:\x1b[01;34m~\x1b[00m$ "],
            &["\x1b]0;deploy@web: ~\x07deploy@web:~$ "],
            &["banner\r\n", "motd\r\n", "deploy@web:~$ "],
            &["deploy@web:~$"],
            &["❯ "],
            &["deploy@web ~ > "],
        ];
        for chunks in cases {
            assert_eq!(watch_all(chunks), Login::Confirmed, "saída: {chunks:?}");
        }
    }

    #[test]
    fn login_prompts_banners_and_errors_do_not_confirm() {
        let cases: &[&[&str]] = &[
            &["deploy@web's password: "],
            &["Password:"],
            &["Enter passphrase for key '/Users/me/.ssh/id_ed25519': "],
            &[
                "The authenticity of host 'web (192.0.2.1)' can't be established.\r\n",
                "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            ],
            &["yes\r\nWarning: Permanently added 'web' (ED25519) to the list of known hosts.\r\n"],
            &["(deploy@web) Verification code: "],
            &["ssh: connect to host web port 22: Connection refused\r\n"],
            &["deploy@web: Permission denied (publickey).\r\n"],
            &["Welcome to Ubuntu 24.04 LTS\r\n * Documentation:  https://help.ubuntu.com\r\n"],
            &["Last login is in the past\r\n"],
            &["the price is $\r\n"],
            &[""],
        ];
        for chunks in cases {
            assert_eq!(watch_all(chunks), Login::Pending, "saída: {chunks:?}");
        }
    }

    #[test]
    fn the_watch_gives_up_past_the_byte_or_time_ceiling() {
        let mut watch = LoginWatch::new(0);
        let noise = "x".repeat(1024) + "\r\n";
        let mut outcome = Login::Pending;
        for _ in 0..(WATCH_MAX_BYTES / noise.len() + 2) {
            outcome = watch.feed(noise.as_bytes(), 10);
            if outcome != Login::Pending {
                break;
            }
        }
        assert_eq!(outcome, Login::GaveUp);
        assert_eq!(
            watch.feed(b"Last login: now\r\n", 20),
            Login::GaveUp,
            "desistiu é de vez"
        );

        let mut watch = LoginWatch::new(1_000);
        assert_eq!(
            watch.feed(b"banner\r\n", 1_000 + WATCH_MAX_MS - 1),
            Login::Pending
        );
        assert_eq!(
            watch.feed(b"deploy@web:~$ ", 1_000 + WATCH_MAX_MS + 1),
            Login::GaveUp
        );
    }

    // --- Ciclo do comando ---

    #[test]
    fn the_tracker_reports_once_per_interactive_ssh() {
        let mut t = TypedSshTracker::default();
        t.on_command(true, Some("ssh deploy@web"), Some(500), 500);
        assert_eq!(t.feed(b"deploy@web's password: ", 600), None);
        let got = t.feed(b"\r\nLast login: today\r\n", 700).expect("logou");
        assert_eq!(got.destination, with_user("web", "deploy"));
        assert_eq!(got.block_started_at_ms, Some(500));
        assert_eq!(t.feed(b"deploy@web:~$ ", 800), None, "um aviso só");
        // O mesmo anúncio repetido não rearma.
        t.on_command(true, Some("ssh deploy@web"), Some(500), 900);
        assert_eq!(t.feed(b"deploy@web:~$ ", 900), None);
    }

    #[test]
    fn the_tracker_ignores_other_commands_and_disarms_when_the_command_ends() {
        let mut t = TypedSshTracker::default();
        t.on_command(true, Some("ls -la"), Some(1), 1);
        assert_eq!(t.feed(b"total 0\r\nme@laptop:~$ ", 2), None);

        t.on_command(true, Some("ssh host uptime"), Some(3), 3);
        assert_eq!(t.feed(b"Last login: x\r\n", 4), None);

        t.on_command(true, Some("ssh web"), Some(5), 5);
        t.on_command(false, None, None, 6);
        assert_eq!(
            t.feed(b"me@laptop:~$ ", 7),
            None,
            "prompt LOCAL depois do fim"
        );

        t.on_command(true, Some("ssh web"), Some(8), 8);
        assert!(t.feed(b"web$ ", 9).is_some(), "rearma no comando seguinte");
    }

    // --- Casamento com o cadastro ---

    fn host(id: &str, alias: &str, hostname: &str) -> crate::ssh::Host {
        crate::ssh::Host {
            id: id.into(),
            alias: alias.into(),
            hostname: hostname.into(),
            port: None,
            username: None,
            identity_file: None,
            proxy_jump: None,
            group_id: None,
            color: None,
            notes: None,
            position: 0,
            tunnels: Vec::new(),
            auth_method: crate::ssh::AuthMethod::Auto,
            agent_key: None,
            integration_enabled: true,
            created_at: chrono::Utc::now(),
            last_connected_at: None,
        }
    }

    fn resolved(line: &str, hosts: &[crate::ssh::Host]) -> Option<TypedMatch> {
        resolve(parse(line).expect("linha interativa"), hosts)
    }

    #[test]
    fn the_destination_matches_a_registered_host() {
        let mut prod = host("h-prod", "prod", "web.example.test");
        prod.username = Some("deploy".into());
        prod.port = Some(2222);
        let mut vps = host("h-vps", "vps", "192.0.2.10");
        vps.username = Some("root".into());
        let hosts = vec![prod, vps];
        let registered = |id: &str| Some(TypedMatch::Registered(id.into()));

        assert_eq!(resolved("ssh prod", &hosts), registered("h-prod"));
        assert_eq!(resolved("ssh deploy@prod", &hosts), registered("h-prod"));
        assert_eq!(resolved("ssh -p 2222 prod", &hosts), registered("h-prod"));
        assert_eq!(
            resolved("ssh -p 2222 deploy@web.example.test", &hosts),
            registered("h-prod")
        );
        assert_eq!(
            resolved("ssh -p 2222 WEB.example.test", &hosts),
            registered("h-prod")
        );
        assert_eq!(resolved("ssh root@192.0.2.10", &hosts), registered("h-vps"));
        assert_eq!(resolved("ssh 192.0.2.10", &hosts), registered("h-vps"));
        assert_eq!(
            resolved("ssh -p 22 192.0.2.10", &hosts),
            registered("h-vps")
        );
    }

    #[test]
    fn a_different_user_port_or_host_is_unregistered() {
        let mut prod = host("h-prod", "prod", "web.example.test");
        prod.username = Some("deploy".into());
        prod.port = Some(2222);
        let hosts = vec![prod];
        for line in [
            "ssh other@prod",
            "ssh -p 22 prod",
            "ssh web.example.test",
            "ssh other@web.example.test",
            "ssh -p 2222 db.example.test",
            "ssh Prod",
        ] {
            let dest = parse(line).expect("linha interativa");
            assert_eq!(
                resolve(dest.clone(), &hosts),
                Some(TypedMatch::Unregistered(dest)),
                "linha: {line:?}"
            );
        }
        assert_eq!(
            resolved("ssh anything", &[]),
            Some(TypedMatch::Unregistered(dest("anything")))
        );
    }

    #[test]
    fn a_host_with_integration_off_stays_quiet() {
        let mut off = host("h-off", "off", "off.example.test");
        off.integration_enabled = false;
        assert_eq!(resolved("ssh off", &[off.clone()]), None);
        assert_eq!(resolved("ssh off.example.test", &[off]), None);
    }

    #[test]
    fn the_event_carries_the_destination_and_the_host_when_registered() {
        let mut prod = host("h-prod", "prod", "web.example.test");
        prod.username = Some("deploy".into());
        let mut off = host("h-off", "off", "off.example.test");
        off.integration_enabled = false;
        let hosts = vec![prod, off];
        let detected = |line: &str| Detected {
            destination: parse(line).expect("linha interativa"),
            block_started_at_ms: Some(42),
        };

        let event = DetectedEvent::build("s1".into(), detected("ssh prod"), &hosts).expect("avisa");
        assert_eq!(event.host_id.as_deref(), Some("h-prod"));
        assert_eq!(event.destination, dest("prod"));
        assert_eq!(event.block_started_at_ms, Some(42));
        assert_eq!(event.session_id, "s1");

        let event = DetectedEvent::build("s1".into(), detected("ssh x@new.example.test"), &hosts)
            .expect("avisa");
        assert_eq!(event.host_id, None);
        assert_eq!(event.destination, with_user("new.example.test", "x"));

        assert_eq!(
            DetectedEvent::build("s1".into(), detected("ssh off"), &hosts),
            None
        );

        let wire = serde_json::to_value(
            DetectedEvent::build("s1".into(), detected("ssh -p 2 a@b"), &[]).expect("avisa"),
        )
        .expect("serializa");
        assert_eq!(
            wire,
            serde_json::json!({
                "session_id": "s1",
                "block_started_at_ms": 42,
                "destination": {
                    "host": "b", "user": "a", "port": 2,
                    "identity_file": null, "proxy_jump": null
                },
                "host_id": null
            })
        );
    }
}
