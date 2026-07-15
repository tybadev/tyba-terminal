use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
pub const LOCAL_TIMEOUT: Duration = Duration::from_secs(10);

const POLL: Duration = Duration::from_millis(20);

pub const FORGE_ENV_EXTRA: [&str; 12] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "GH_HOST",
    "GH_CONFIG_DIR",
    "GLAB_TOKEN",
    "GITLAB_TOKEN",
    "GLAB_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

pub const FORGE_ENV_PROXY: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

const FORGE_ENV_FORCED: [(&str, &str); 5] = [
    ("NO_COLOR", "1"),
    ("CLICOLOR", "0"),
    ("GH_NO_UPDATE_NOTIFIER", "1"),
    ("GH_PROMPT_DISABLED", "1"),
    ("GIT_TERMINAL_PROMPT", "0"),
];

fn allowed(key: &str) -> bool {
    crate::repo_config::AGENT_ENV_BASELINE.contains(&key)
        || FORGE_ENV_EXTRA.contains(&key)
        || FORGE_ENV_PROXY.contains(&key)
}

fn absolute_path_only(path: &str) -> String {
    std::env::split_paths(path)
        .filter(|p| p.is_absolute())
        .collect::<Vec<_>>()
        .iter()
        .filter_map(|p| p.to_str())
        .collect::<Vec<_>>()
        .join(":")
}

pub fn filter_env(
    vars: impl Iterator<Item = (String, String)>,
    login_path: &str,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vars
        .filter(|(k, _)| k.as_str() != "PATH" && allowed(k))
        .collect();
    env.push(("PATH".into(), absolute_path_only(login_path)));
    for (k, v) in FORGE_ENV_FORCED {
        env.retain(|(key, _)| key != k);
        env.push((k.into(), v.into()));
    }
    env
}

#[derive(Debug)]
pub struct Output {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn stderr_message(&self) -> String {
        let text = self.stderr.trim();
        if text.is_empty() {
            "sem detalhes na saída de erro".into()
        } else {
            let joined = text.lines().take(6).collect::<Vec<_>>().join(" / ");
            crate::session::redact::redact(&joined).into_owned()
        }
    }
}

pub fn run(
    program: &str,
    args: &[&str],
    cwd: &Path,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in filter_env(std::env::vars(), &crate::shell_path::agent_path()) {
        cmd.env(k, v);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    crate::repo::no_console_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        // ENOENT aqui significa "a CLI não está instalada" — e "arquivo ou diretório
        // inexistente" é a pior mensagem possível pra isso: não diz QUAL arquivo, e
        // parece defeito do Tyba. Quem chama decide se degrada ou avisa.
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("`{program}` não está instalado")
        } else {
            format!("falha ao executar `{program}`: {e}")
        }
    })?;

    let stdin_handle = stdin.map(|bytes| {
        let bytes = bytes.to_vec();
        let mut pipe = child.stdin.take();
        std::thread::spawn(move || {
            if let Some(pipe) = pipe.as_mut() {
                let _ = pipe.write_all(&bytes);
            }
        })
    });

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = out_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = err_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => return Err(format!("falha ao aguardar `{program}`: {e}")),
        }
        if started.elapsed() >= timeout {
            kill_group(&mut child);
            break None;
        }
        std::thread::sleep(POLL);
    };

    if status.is_some() {
        reap_group(&child);
    }

    if let Some(handle) = stdin_handle {
        let _ = handle.join();
    }
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_handle.join().unwrap_or_default()).into_owned();

    let Some(status) = status else {
        return Err(format!(
            "`{program}` excedeu o tempo limite de {}s e foi encerrado",
            timeout.as_secs()
        ));
    };

    Ok(Output {
        ok: status.success(),
        stdout,
        stderr,
    })
}

fn reap_group(child: &std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child;
    }
}

fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_filter_keeps_only_the_allowlist() {
        let vars = [
            ("PATH", "/usr/bin"),
            ("HOME", "/Users/dev"),
            ("GH_TOKEN", "t"),
            ("GLAB_TOKEN", "t"),
            ("AWS_SECRET_ACCESS_KEY", "leak"),
            ("OPENAI_API_KEY", "leak"),
        ];
        let env = filter_env(
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())),
            "/usr/bin",
        );
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();

        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"HOME"));
        assert!(keys.contains(&"GH_TOKEN"));
        assert!(keys.contains(&"GLAB_TOKEN"));
        assert!(!keys.contains(&"AWS_SECRET_ACCESS_KEY"));
        assert!(!keys.contains(&"OPENAI_API_KEY"));
    }

    #[test]
    fn env_filter_forces_non_interactive_values() {
        let vars = [("NO_COLOR", "0"), ("PATH", "/usr/bin")];
        let env = filter_env(
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())),
            "/usr/bin",
        );

        let no_color: Vec<&String> = env
            .iter()
            .filter(|(k, _)| k == "NO_COLOR")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(no_color, vec!["1"]);
        assert!(env
            .iter()
            .any(|(k, v)| k == "GIT_TERMINAL_PROMPT" && v == "0"));
    }

    #[test]
    fn env_filter_strips_relative_path_components() {
        let env = filter_env(std::iter::empty(), "/usr/bin:.:node_modules/.bin:/opt/bin");
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(path, "/usr/bin:/opt/bin");
        assert!(!path.contains('.') || path.starts_with('/'));
    }

    #[test]
    fn env_filter_takes_path_from_the_login_shell_not_the_process() {
        let vars = [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"), ("HOME", "/u")];
        let env = filter_env(
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())),
            "/opt/homebrew/bin:/usr/bin",
        );
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(path, "/opt/homebrew/bin:/usr/bin");
    }

    #[test]
    fn stderr_message_redacts_secrets() {
        let out = Output {
            ok: false,
            stdout: Vec::new(),
            stderr: "fatal: https://x-access-token:ghp_abcdefghijklmnopqrstuvwxyz0123456789@github.com/x/y".into(),
        };
        let msg = out.stderr_message();
        assert!(
            !msg.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
            "{msg}"
        );
    }

    #[test]
    fn timeout_kills_the_child_and_reports_an_error() {
        let dir = std::env::temp_dir();
        let err = run("sleep", &["30"], &dir, None, Duration::from_millis(300))
            .expect_err("deveria estourar o timeout");
        assert!(err.contains("tempo limite"), "{err}");
    }

    /// O bug do QA no Linux: sem o `gh`, o painel de git mostrava "falha ao executar
    /// gh: arquivo ou diretório inexistente". A mensagem não diz QUAL arquivo e
    /// parece defeito do Tyba — quando na verdade é só uma CLI que a maioria das
    /// pessoas não instala.
    #[test]
    fn a_missing_cli_says_it_is_not_installed_not_enoent() {
        let err = run(
            "tyba-cli-que-nao-existe",
            &["--version"],
            Path::new("/"),
            None,
            LOCAL_TIMEOUT,
        )
        .expect_err("binário inexistente");

        assert!(err.contains("não está instalado"), "{err}");
        assert!(
            !err.to_lowercase().contains("no such file")
                && !err.to_lowercase().contains("inexistente\""),
            "a mensagem crua do SO não pode vazar pro usuário: {err}"
        );
    }

    #[test]
    fn run_captures_stdout_and_exit_status() {
        let dir = std::env::temp_dir();
        let out = run("echo", &["oi"], &dir, None, LOCAL_TIMEOUT).expect("echo");
        assert!(out.ok);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "oi");
    }
}
