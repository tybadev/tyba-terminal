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

pub fn filter_env(vars: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vars
        .filter(|(k, _)| allowed(k))
        .map(|(k, v)| {
            if k == "PATH" {
                (k, absolute_path_only(&v))
            } else {
                (k, v)
            }
        })
        .collect();
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

    for (k, v) in filter_env(std::env::vars()) {
        cmd.env(k, v);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("falha ao executar `{program}`: {e}"))?;

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
        let env = filter_env(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
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
        let env = filter_env(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));

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
        let vars = [("PATH", "/usr/bin:.:node_modules/.bin:/opt/bin")];
        let env = filter_env(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(path, "/usr/bin:/opt/bin");
        assert!(!path.contains('.') || path.starts_with('/'));
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

    #[test]
    fn run_captures_stdout_and_exit_status() {
        let dir = std::env::temp_dir();
        let out = run("echo", &["oi"], &dir, None, LOCAL_TIMEOUT).expect("echo");
        assert!(out.ok);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "oi");
    }
}
