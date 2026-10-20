//! Push para main/master por agente: recusado pelo hook `pre-push` do TYBA
//! (princípio 5, decisão do dono de 2026-10-19).
//!
//! O destino de um push só se sabe de verdade dentro do git: `push.default`,
//! `remote.*.push`, upstream, alias, script, `cd` em subshell — o texto do
//! comando não dá conta. Então toda sessão de agente (claude e codex,
//! digitado e hospedado, ou aberto pelo TYBA; e, por herança de env, o que o
//! agente abrir) recebe por env (`GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/
//! `GIT_CONFIG_VALUE_n`, somados ao que já existir) `core.hooksPath` apontando
//! para a pasta de hooks do TYBA. Config por env vence a dos arquivos; só
//! `git -c` vence o env, e o guarda-corpo de texto recusa isso.
//!
//! O `pre-push` lê as linhas `<ref local> <sha local> <ref remota> <sha
//! remoto>` da entrada e recusa se alguma ref remota for `refs/heads/main` ou
//! `refs/heads/master`. Depois roda o `pre-push` do usuário.
//!
//! `core.hooksPath` troca a pasta de TODOS os hooks, então a pasta do TYBA tem
//! um hook de passagem para cada hook do git: ele acha a pasta de hooks do
//! usuário (o `core.hooksPath` dele, ou `$GIT_DIR/hooks`) perguntando ao git
//! com o env de config de antes do TYBA (`TYBA_GIT_CONFIG_BASE`), e roda o
//! hook dele com os mesmos argumentos e a mesma entrada. Nada do usuário deixa
//! de rodar.
//!
//! Os scripts são `sh` portátil: o Git for Windows roda hook com o `sh` dele.

use std::collections::HashMap;
use std::path::Path;

/// Quantas entradas de `GIT_CONFIG_*` havia antes do TYBA: o hook de passagem
/// usa para achar o `core.hooksPath` do usuário sem o nosso.
pub const BASE_ENV: &str = "TYBA_GIT_CONFIG_BASE";
/// No plano do claude hospedado: a pasta de hooks. O shim soma o
/// `core.hooksPath` ao env do pane ([`with_hooks_os`]), que só ele conhece.
pub const HOOKS_DIR_ENV: &str = "TYBA_GIT_HOOKS";

/// Todo hook que o git procura na pasta de hooks (githooks(5)), menos:
/// - `fsmonitor-watchman`, que o git nunca procura lá (o `core.fsmonitor`
///   aponta o caminho do script);
/// - `reference-transaction` e `post-index-change` (decisão do dono,
///   2026-10-19, "hook enxuto"): disparam várias vezes por comando e eram
///   ~90% do custo (rebase de 200 commits de 1,3 s para 60 s). **Hooks do
///   usuário desses dois tipos não rodam em sessão de agente.**
pub const HOOK_NAMES: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
];

/// Acha o hook do usuário com o nome deste script. Deixa em `user_hook` e
/// devolve 0; sem hook (ou se ele é este mesmo arquivo), devolve 1. O mais
/// barato possível: um `git rev-parse` e nada de subshell além dele.
const FIND_USER_HOOK: &str = r#"tyba_user_hook() {
  dir=$(GIT_CONFIG_COUNT="${TYBA_GIT_CONFIG_BASE:-0}" git rev-parse --git-path hooks 2>/dev/null) || return 1
  case "$dir" in
    /*|[A-Za-z]:*) ;;
    *) dir="$PWD/$dir" ;;
  esac
  user_hook="$dir/${0##*/}"
  [ -f "$user_hook" ] && [ -x "$user_hook" ] && ! [ "$user_hook" -ef "$0" ]
}
"#;

const PASSTHROUGH_TAIL: &str = r#"tyba_user_hook || exit 0
exec "$user_hook" "$@"
"#;

/// `push-to-checkout`: com o arquivo presente o git deixa de fazer o padrão
/// (`updateInstead`); sem hook do usuário, este faz o padrão.
const PUSH_TO_CHECKOUT_TAIL: &str = r#"tyba_user_hook && exec "$user_hook" "$@"
git update-index -q --ignore-submodules --refresh || exit 1
if ! git diff-files --quiet --ignore-submodules -- ||
   ! git diff-index --quiet --cached --ignore-submodules HEAD --; then
  echo "Working directory has unstaged changes" >&2
  exit 1
fi
exec git read-tree -u -m HEAD "$1"
"#;

/// Lê a entrada direto do stdin, sem heredoc nem arquivo temporário: com o
/// `/tmp` só leitura (sandbox, bash 3.2 do macOS) o heredoc falhava e o hook
/// passava. Linha: `<ref local> <sha local> <ref remota> <sha remoto>`.
const PRE_PUSH_TAIL: &str = r#"input=""
refused=""
while IFS= read -r line || [ -n "$line" ]; do
  input="$input$line
"
  remote_ref=${line#* * }
  remote_ref=${remote_ref%% *}
  case "$remote_ref" in
    refs/heads/main|refs/heads/master) refused=${remote_ref#refs/heads/} ;;
  esac
done
if [ -n "$refused" ]; then
  echo "TYBA: push para $refused recusado (guarda-corpo fixo do TYBA): agente não empurra para main/master. Faça o push numa branch própria e abra um PR; o merge na main fica com o humano." >&2
  exit 1
fi
tyba_user_hook || exit 0
printf '%s' "$input" | "$user_hook" "$@"
"#;

pub fn script(name: &str) -> String {
    let tail = match name {
        "pre-push" => PRE_PUSH_TAIL,
        "push-to-checkout" => PUSH_TO_CHECKOUT_TAIL,
        _ => PASSTHROUGH_TAIL,
    };
    format!(
        "#!/bin/sh\n# Hook do TYBA (agente livre): {name}. Ver agent/git_hooks.rs.\n{FIND_USER_HOOK}{tail}"
    )
}

/// Permissão da pasta e dos scripts depois de gravados: só leitura e
/// execução — instalador de hook (lefthook, git-lfs) não grava nela.
#[cfg(unix)]
const READ_ONLY: u32 = 0o500;

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// A pasta está como o binário atual a gravaria: todo script com o conteúdo
/// certo e, no unix, pasta e scripts só leitura.
pub fn hooks_current(dir: &Path) -> bool {
    #[cfg(unix)]
    let mode_ok = |p: &Path| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o777 == READ_ONLY)
    };
    #[cfg(not(unix))]
    let mode_ok = |_: &Path| true;
    mode_ok(dir)
        && HOOK_NAMES.iter().all(|name| {
            let path = dir.join(name);
            mode_ok(&path) && std::fs::read_to_string(&path).is_ok_and(|s| s == script(name))
        })
}

/// Grava a pasta de hooks com o conteúdo do binário atual: abre a permissão
/// (pasta e scripts que já existem), grava, tira scripts que não são mais
/// nossos e fecha tudo em só leitura.
pub fn write_hooks(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        set_mode(dir, 0o700)?;
        for entry in std::fs::read_dir(dir)?.flatten() {
            let _ = set_mode(&entry.path(), 0o700);
        }
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name();
        // Temporário de gravação (`.nome.pid.tmp`) não é apagado aqui.
        let temp = name.to_string_lossy().starts_with('.');
        if !temp && !HOOK_NAMES.iter().any(|n| name.as_os_str() == *n) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    for name in HOOK_NAMES {
        crate::session::write_private(dir, name, &script(name))?;
        #[cfg(unix)]
        set_mode(&dir.join(name), READ_ONLY)?;
    }
    #[cfg(unix)]
    set_mode(dir, READ_ONLY)?;
    Ok(())
}

/// Trava entre quem confere e regrava a pasta (sessões do app e `tyba
/// _codex` de vários panes): um arquivo criado com `create_new` ao lado da
/// pasta. Trava velha (processo que morreu segurando) expira.
struct HooksLock(std::path::PathBuf);

const LOCK_STALE: std::time::Duration = std::time::Duration::from_secs(10);
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

impl HooksLock {
    fn acquire(dir: &Path) -> std::io::Result<Self> {
        let name = format!(
            ".{}.lock",
            dir.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("git-hooks")
        );
        let parent = dir.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let path = parent.join(name);
        let deadline = std::time::Instant::now() + LOCK_WAIT;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "outra sessão segura a trava da pasta de hooks",
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for HooksLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Confere a pasta a cada sessão de agente e regrava se ela sumiu, foi
/// mexida ou é de outra versão. Conferir e regravar ficam sob a mesma trava:
/// duas sessões juntas não deixam a pasta pela metade.
pub fn ensure_hooks(dir: &Path) -> std::io::Result<()> {
    if hooks_current(dir) {
        return Ok(());
    }
    let _lock = HooksLock::acquire(dir)?;
    if hooks_current(dir) {
        return Ok(());
    }
    write_hooks(dir)
}

/// Tira de um `GIT_CONFIG_PARAMETERS` (o `git -c` herdado) as entradas de
/// `core.hookspath`: elas venceriam o `core.hooksPath` do TYBA. Formato do
/// git: `'chave'='valor'` (ou `'chave'`) separados por espaço, com `'` como
/// `'\''`. `None` se não sobrou nada.
pub fn strip_hookspath_params(params: &str) -> Option<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in params.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                current.push(c);
            }
            ' ' if !quoted => {
                if !current.is_empty() {
                    entries.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        entries.push(current);
    }
    let key_of = |entry: &str| {
        let mut key = String::new();
        let mut quoted = false;
        for c in entry.chars() {
            match c {
                '\'' => quoted = !quoted,
                '=' if !quoted => break,
                _ => key.push(c),
            }
        }
        key.trim().to_ascii_lowercase()
    };
    let kept: Vec<String> = entries
        .into_iter()
        .filter(|e| key_of(e) != "core.hookspath")
        .collect();
    (!kept.is_empty()).then(|| kept.join(" "))
}

/// As variáveis do hook no env (para o Codex repassar pelo
/// `shell_environment_policy.set`, que ele aplica por último — com uma
/// política restrita, o env sozinho não chega aos comandos dele).
pub fn hook_vars(get: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let Some(_) = get(BASE_ENV) else {
        return Vec::new();
    };
    let count: usize = get("GIT_CONFIG_COUNT")
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    let mut vars = vec![("GIT_CONFIG_COUNT".to_string(), count.to_string())];
    for i in 0..count {
        for key in [
            format!("GIT_CONFIG_KEY_{i}"),
            format!("GIT_CONFIG_VALUE_{i}"),
        ] {
            if let Some(v) = get(&key) {
                vars.push((key, v));
            }
        }
    }
    if let Some(base) = get(BASE_ENV) {
        vars.push((BASE_ENV.to_string(), base));
    }
    vars
}

/// As entradas de env que somam `core.hooksPath=<dir>` ao `GIT_CONFIG_*` que
/// já existe (`get` lê o env atual). Vazio se o nosso já está lá — o agente
/// aninhado herda o env e não ganha uma segunda entrada.
pub fn config_env(get: impl Fn(&str) -> Option<String>, dir: &str) -> Vec<(String, String)> {
    let base: usize = get("GIT_CONFIG_COUNT")
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or(0);
    let ours = (0..base).any(|i| {
        get(&format!("GIT_CONFIG_KEY_{i}"))
            .is_some_and(|k| k.eq_ignore_ascii_case("core.hooksPath"))
            && get(&format!("GIT_CONFIG_VALUE_{i}")).as_deref() == Some(dir)
    });
    if ours {
        return Vec::new();
    }
    vec![
        (format!("GIT_CONFIG_KEY_{base}"), "core.hooksPath".into()),
        (format!("GIT_CONFIG_VALUE_{base}"), dir.into()),
        ("GIT_CONFIG_COUNT".into(), (base + 1).to_string()),
        (BASE_ENV.into(), base.to_string()),
    ]
}

/// [`config_env`] aplicado a um env em mapa (o do agente aberto pelo TYBA).
/// Também tira `core.hookspath` de um `GIT_CONFIG_PARAMETERS` herdado.
pub fn with_hooks(env: &mut HashMap<String, String>, dir: &Path) {
    if let Some(params) = env.get("GIT_CONFIG_PARAMETERS").cloned() {
        match strip_hookspath_params(&params) {
            Some(kept) => env.insert("GIT_CONFIG_PARAMETERS".into(), kept),
            None => env.remove("GIT_CONFIG_PARAMETERS"),
        };
    }
    let dir = dir.to_string_lossy();
    let add = config_env(|k| env.get(k).cloned(), &dir);
    env.extend(add);
}

/// O env final do claude hospedado: se o plano trouxe [`HOOKS_DIR_ENV`],
/// soma o `core.hooksPath` ao `GIT_CONFIG_*` do pane.
pub fn with_hooks_os(
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let get = |key: &str| {
        env.iter()
            .rev()
            .find(|(k, _)| k.as_os_str() == key)
            .map(|(_, v)| v.to_string_lossy().into_owned())
    };
    let Some(dir) = get(HOOKS_DIR_ENV) else {
        return env;
    };
    let add = config_env(get, &dir);
    let params = get("GIT_CONFIG_PARAMETERS").map(|p| strip_hookspath_params(&p));
    let mut env: Vec<_> = env
        .into_iter()
        .filter(|(k, _)| !add.iter().any(|(a, _)| k.as_os_str() == a.as_str()))
        .filter(|(k, _)| params.is_none() || k.as_os_str() != "GIT_CONFIG_PARAMETERS")
        .collect();
    if let Some(Some(kept)) = params {
        env.push(("GIT_CONFIG_PARAMETERS".into(), kept.into()));
    }
    env.extend(add.into_iter().map(|(k, v)| (k.into(), v.into())));
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_env_appends_to_the_existing_git_config() {
        let existing: HashMap<String, String> = [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "user.name"),
            ("GIT_CONFIG_VALUE_0", "dono"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let mut env = existing.clone();
        with_hooks(&mut env, Path::new("/h"));
        assert_eq!(
            env["GIT_CONFIG_KEY_0"], "user.name",
            "não apaga o do usuário"
        );
        assert_eq!(env["GIT_CONFIG_KEY_1"], "core.hooksPath");
        assert_eq!(env["GIT_CONFIG_VALUE_1"], "/h");
        assert_eq!(env["GIT_CONFIG_COUNT"], "2");
        assert_eq!(env[BASE_ENV], "1");
        // Aninhado: herda e não duplica.
        let before = env.clone();
        with_hooks(&mut env, Path::new("/h"));
        assert_eq!(env, before);

        let mut empty = HashMap::new();
        with_hooks(&mut empty, Path::new("/h"));
        assert_eq!(empty["GIT_CONFIG_COUNT"], "1");
        assert_eq!(empty[BASE_ENV], "0");
    }

    #[test]
    fn hosted_env_gets_the_hooks_on_top_of_the_pane_config() {
        let env = vec![
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            ("GIT_CONFIG_KEY_0".into(), "x.y".into()),
            ("GIT_CONFIG_VALUE_0".into(), "z".into()),
            (HOOKS_DIR_ENV.into(), "/h".into()),
        ];
        let out = with_hooks_os(env);
        let get = |k: &str| {
            out.iter()
                .find(|(key, _)| key.as_os_str() == k)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert_eq!(get("GIT_CONFIG_COUNT").as_deref(), Some("2"));
        assert_eq!(get("GIT_CONFIG_KEY_1").as_deref(), Some("core.hooksPath"));
        assert_eq!(get("GIT_CONFIG_KEY_0").as_deref(), Some("x.y"));
        assert_eq!(
            out.iter()
                .filter(|(k, _)| k.as_os_str() == "GIT_CONFIG_COUNT")
                .count(),
            1
        );
    }

    #[test]
    fn inherited_git_c_hookspath_is_dropped() {
        assert_eq!(strip_hookspath_params("'core.hooksPath'='/x'"), None);
        assert_eq!(
            strip_hookspath_params("'user.name'='a b' 'CORE.HOOKSPATH'='/x' 'x.y'"),
            Some("'user.name'='a b' 'x.y'".to_string())
        );
        assert_eq!(
            strip_hookspath_params("'a.b'='it'\\''s'"),
            Some("'a.b'='it'\\''s'".to_string())
        );
        let mut env: HashMap<String, String> = [(
            "GIT_CONFIG_PARAMETERS".to_string(),
            "'core.hookspath'='/dev/null' 'color.ui'='never'".to_string(),
        )]
        .into_iter()
        .collect();
        with_hooks(&mut env, Path::new("/h"));
        assert_eq!(env["GIT_CONFIG_PARAMETERS"], "'color.ui'='never'");
        let os = with_hooks_os(vec![
            ("GIT_CONFIG_PARAMETERS".into(), "'core.hooksPath'=''".into()),
            (HOOKS_DIR_ENV.into(), "/h".into()),
        ]);
        assert!(!os
            .iter()
            .any(|(k, _)| k.as_os_str() == "GIT_CONFIG_PARAMETERS"));
    }

    #[test]
    fn hook_vars_carry_the_whole_git_config_for_codex() {
        let mut env = HashMap::new();
        env.insert("GIT_CONFIG_COUNT".to_string(), "1".to_string());
        env.insert("GIT_CONFIG_KEY_0".to_string(), "user.name".to_string());
        env.insert("GIT_CONFIG_VALUE_0".to_string(), "dono".to_string());
        assert!(
            hook_vars(|k| env.get(k).cloned()).is_empty(),
            "sem o hook do TYBA, nada"
        );
        with_hooks(&mut env, Path::new("/h"));
        let vars: HashMap<_, _> = hook_vars(|k| env.get(k).cloned()).into_iter().collect();
        assert_eq!(vars["GIT_CONFIG_COUNT"], "2");
        assert_eq!(vars["GIT_CONFIG_KEY_0"], "user.name");
        assert_eq!(vars["GIT_CONFIG_KEY_1"], "core.hooksPath");
        assert_eq!(vars["GIT_CONFIG_VALUE_1"], "/h");
        assert_eq!(vars[BASE_ENV], "1");
    }

    #[cfg(unix)]
    #[test]
    fn hooks_folder_is_read_only_and_kept_current() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("git-hooks");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        ensure_hooks(&dir).unwrap();
        assert_eq!(mode(&dir), 0o500);
        assert_eq!(mode(&dir.join("pre-push")), 0o500);
        assert!(hooks_current(&dir));
        assert!(!dir.join("reference-transaction").exists());
        // Um instalador não consegue gravar nela.
        assert!(std::fs::write(dir.join("pre-commit"), "x").is_err());
        assert!(std::fs::write(dir.join("novo"), "x").is_err());
        // Mexida (por fora) ou apagada: a próxima sessão regrava.
        set_mode(&dir.join("pre-push"), 0o700).unwrap();
        std::fs::write(dir.join("pre-push"), "velho").unwrap();
        assert!(!hooks_current(&dir));
        ensure_hooks(&dir).unwrap();
        assert!(hooks_current(&dir));
        set_mode(&dir, 0o700).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        ensure_hooks(&dir).unwrap();
        assert!(hooks_current(&dir));
        set_mode(&dir, 0o700).unwrap();
    }

    /// Duas sessões regravando juntas: as duas terminam com a pasta inteira.
    #[cfg(unix)]
    #[test]
    fn two_sessions_rewriting_at_once_both_end_with_the_hooks() {
        for _ in 0..20 {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("git-hooks");
            let run = || {
                let dir = dir.clone();
                std::thread::spawn(move || ensure_hooks(&dir))
            };
            let (a, b) = (run(), run());
            a.join().unwrap().unwrap();
            b.join().unwrap().unwrap();
            assert!(hooks_current(&dir));
            assert!(!tmp.path().join(".git-hooks.lock").exists(), "trava solta");
            set_mode(&dir, 0o700).unwrap();
        }
    }

    /// e2e com git de verdade (repos sintéticos num tempdir, remoto bare).
    #[cfg(unix)]
    mod e2e {
        use super::*;
        use std::path::PathBuf;
        use std::process::{Command, Output};

        struct Lab {
            _tmp: tempfile::TempDir,
            work: PathBuf,
            remote: PathBuf,
            hooks: PathBuf,
        }

        /// A pasta de hooks é só leitura: abre antes de o tempdir ser apagado.
        impl Drop for Lab {
            fn drop(&mut self) {
                let _ = set_mode(&self.hooks, 0o700);
            }
        }

        /// Git sem a config da máquina (nada de assinatura, hooks globais).
        fn git_cmd(dir: &Path) -> Command {
            let mut cmd = Command::new("git");
            cmd.current_dir(dir)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE");
            cmd
        }

        fn git(dir: &Path, args: &[&str]) {
            let out = git_cmd(dir).args(args).output().unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        fn lab() -> Lab {
            let tmp = tempfile::tempdir().unwrap();
            let remote = tmp.path().join("remote.git");
            let work = tmp.path().join("work");
            let hooks = tmp.path().join("tyba-hooks");
            std::fs::create_dir_all(&work).unwrap();
            std::fs::create_dir_all(&hooks).unwrap();
            git(
                tmp.path(),
                &[
                    "init",
                    "-q",
                    "--bare",
                    "-b",
                    "main",
                    remote.to_str().unwrap(),
                ],
            );
            git(&work, &["init", "-q", "-b", "main"]);
            std::fs::create_dir_all(work.join("sub")).unwrap();
            std::fs::write(work.join("sub/a.txt"), "1").unwrap();
            git(&work, &["add", "."]);
            git(&work, &["commit", "-q", "-m", "init"]);
            git(
                &work,
                &["remote", "add", "origin", remote.to_str().unwrap()],
            );
            git(&work, &["push", "-q", "-u", "origin", "main"]);
            git(&work, &["checkout", "-q", "-b", "feat"]);
            std::fs::write(work.join("sub/a.txt"), "2").unwrap();
            git(&work, &["commit", "-q", "-am", "feat"]);
            git(&work, &["checkout", "-q", "main"]);
            git(&work, &["merge", "-q", "--ff-only", "feat"]);
            write_hooks(&hooks).unwrap();
            Lab {
                _tmp: tmp,
                work,
                remote,
                hooks,
            }
        }

        impl Lab {
            /// Roda `script` em bash com o env de uma sessão de agente.
            fn agent(&self, script: &str) -> Output {
                let mut sh = Command::new("bash");
                sh.current_dir(&self.work)
                    .arg("-c")
                    .arg(script)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .env("REMOTE", &self.remote)
                    .env("WORK", &self.work)
                    .env_remove("GIT_CONFIG_COUNT");
                let mut env = HashMap::new();
                with_hooks(&mut env, &self.hooks);
                sh.envs(env);
                sh.output().unwrap()
            }

            fn remote_main(&self) -> String {
                let out = git_cmd(&self.remote)
                    .args(["rev-parse", "main"])
                    .output()
                    .unwrap();
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
        }

        impl Lab {
            /// Como [`Lab::agent`], com o `TMPDIR` apontando para onde não
            /// dá para gravar (sandbox só leitura).
            fn agent_without_tmp(&self, script: &str) -> Output {
                let mut sh = Command::new("bash");
                sh.current_dir(&self.work)
                    .arg("-c")
                    .arg(script)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("TMPDIR", "/nao/existe/tmp")
                    .env_remove("GIT_CONFIG_COUNT");
                let mut env = HashMap::new();
                with_hooks(&mut env, &self.hooks);
                sh.envs(env);
                sh.output().unwrap()
            }
        }

        #[test]
        fn read_only_tmp_does_not_open_the_hook() {
            let lab = lab();
            let out = lab.agent_without_tmp("git push origin main");
            assert!(refused(&out), "{}", String::from_utf8_lossy(&out.stderr));
            let out = lab.agent_without_tmp("git push origin feat");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        fn refused(out: &Output) -> bool {
            !out.status.success()
                && String::from_utf8_lossy(&out.stderr).contains("TYBA: push para main recusado")
        }

        #[test]
        fn every_way_of_pushing_to_main_hits_the_hook() {
            let lab = lab();
            let before = lab.remote_main();
            for script in [
                "git push origin main",
                "git push",
                "git push origin HEAD",
                "git push origin HEAD:main",
                "git push origin feat:main",
                "git push origin heads/main",
                "git push origin refs/heads/main",
                "git push origin :main",
                "git -c push.default=matching push origin",
                "git -c push.default=current push origin",
                "git -c push.default=upstream push",
                "git -c remote.origin.push=refs/heads/main:refs/heads/main push origin",
                "(cd sub && git push origin main); exit $?",
                "builtin cd sub; git push origin main",
                "git -C \"$WORK\" push origin main",
                "cd / && git -C \"$WORK\" push origin main",
                "git -c alias.P=push P origin main",
                "git -c alias.x='!git push origin main' x",
                "echo 'git push origin main' | bash",
                "b=main; git push origin \"$b\"",
                "git push \"$REMOTE\" main",
            ] {
                let out = lab.agent(script);
                assert!(
                    refused(&out),
                    "{script}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            // `env -C` só no env do GNU.
            if Command::new("env")
                .args(["-C", "/", "true"])
                .status()
                .is_ok_and(|s| s.success())
            {
                let out = lab.agent("env -C sub git push origin main");
                assert!(refused(&out), "env -C");
            }
            // HEAD versionado na raiz do repo não confunde nada.
            let out = lab.agent(
                "echo x > HEAD && git add HEAD && git commit -q -m h && git push origin main",
            );
            assert!(refused(&out), "HEAD na raiz");
            assert_eq!(lab.remote_main(), before, "a main do remoto não andou");
        }

        /// Medida, não verificação: `cargo test --lib medir_custo -- --ignored
        /// --nocapture`. Rebase de 200 commits e um `git commit`, com e sem o
        /// env do TYBA.
        #[test]
        #[ignore = "medida de tempo, roda à mão"]
        fn medir_custo_dos_hooks() {
            let lab = lab();
            lab.agent(
                "git checkout -q -b longa && for i in $(seq 1 200); do \
                 echo $i > n.txt && git add n.txt && git commit -q -m c$i; done && \
                 git checkout -q -b base main~1 && echo b > b.txt && git add b.txt && \
                 git commit -q -m b && git checkout -q longa",
            );
            let time = |with: bool, script: &str| {
                let start = std::time::Instant::now();
                let mut sh = Command::new("bash");
                sh.current_dir(&lab.work)
                    .arg("-c")
                    .arg(script)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .env_remove("GIT_CONFIG_COUNT");
                if with {
                    let mut env = HashMap::new();
                    with_hooks(&mut env, &lab.hooks);
                    sh.envs(env);
                }
                let out = sh.output().unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                start.elapsed()
            };
            let rebase = "git checkout -q -B r longa && git rebase -q base";
            let commit = "git commit -q --allow-empty -m x";
            let sem_rebase = time(false, rebase);
            let com_rebase = time(true, rebase);
            let sem_commit = time(false, commit);
            let com_commit = time(true, commit);
            eprintln!(
                "rebase 200: sem {sem_rebase:?}, com TYBA {com_rebase:?}; commit: sem {sem_commit:?}, com TYBA {com_commit:?}"
            );
        }

        #[test]
        fn feature_branches_and_tags_still_push() {
            let lab = lab();
            for script in [
                "git push origin feat",
                "git push origin main:feat2",
                "git tag v1 && git push origin --tags",
            ] {
                let out = lab.agent(script);
                assert!(
                    out.status.success(),
                    "{script}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }

        #[test]
        fn the_user_hooks_keep_running() {
            let lab = lab();
            let hooks = lab.work.join(".git/hooks");
            std::fs::write(
                hooks.join("pre-commit"),
                "#!/bin/sh\necho ok > \"$WORK/.pre-commit-ran\"\n",
            )
            .unwrap();
            std::fs::write(
                hooks.join("pre-push"),
                "#!/bin/sh\ncat > \"$WORK/.pre-push-input\"\necho \"$1 $2\" > \"$WORK/.pre-push-args\"\nexit 3\n",
            )
            .unwrap();
            for name in ["pre-commit", "pre-push"] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(hooks.join(name), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            let out = lab.agent("git commit -q --allow-empty -m x");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                lab.work.join(".pre-commit-ran").exists(),
                "pre-commit do usuário rodou"
            );

            // O pre-push do usuário recebe a mesma entrada e os args, e o
            // código dele volta.
            let out = lab.agent("git push origin feat");
            assert!(!out.status.success(), "o exit 3 do usuário vale");
            let input = std::fs::read_to_string(lab.work.join(".pre-push-input")).unwrap();
            assert!(input.contains("refs/heads/feat"), "{input}");
            let args = std::fs::read_to_string(lab.work.join(".pre-push-args")).unwrap();
            assert!(args.starts_with("origin "), "{args}");

            // `core.hooksPath` próprio do usuário (config do repo) também roda.
            let own = lab.work.join("meus-hooks");
            std::fs::create_dir_all(&own).unwrap();
            std::fs::write(
                own.join("post-commit"),
                "#!/bin/sh\necho ok > \"$WORK/.post-commit-ran\"\n",
            )
            .unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    own.join("post-commit"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            git(&lab.work, &["config", "core.hooksPath", "meus-hooks"]);
            let out = lab.agent("git commit -q --allow-empty -m y");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                lab.work.join(".post-commit-ran").exists(),
                "hooksPath do usuário rodou"
            );
            // E mesmo assim o do TYBA manda no push para a main.
            let out = lab.agent("git push origin main");
            assert!(refused(&out), "{}", String::from_utf8_lossy(&out.stderr));
        }
    }
}
