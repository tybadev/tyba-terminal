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
}
