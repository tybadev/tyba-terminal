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

/// Todo hook que o git procura na pasta de hooks (githooks(5)), menos o
/// `fsmonitor-watchman`, que o git nunca procura lá (o `core.fsmonitor`
/// aponta o caminho do script).
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
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];

/// Acha o hook do usuário com o nome deste script. Deixa em `user_hook` e
/// devolve 0; sem hook (ou se a pasta do usuário é a nossa), devolve 1.
const FIND_USER_HOOK: &str = r#"tyba_user_hook() {
  name=${0##*/}
  dir=$(GIT_CONFIG_COUNT="${TYBA_GIT_CONFIG_BASE:-0}" git rev-parse --git-path hooks 2>/dev/null) || return 1
  case "$dir" in
    /*|[A-Za-z]:*) ;;
    *) dir="$(pwd)/$dir" ;;
  esac
  self=$(cd "${0%/*}" 2>/dev/null && pwd -P) || return 1
  real=$(cd "$dir" 2>/dev/null && pwd -P) || return 1
  [ "$real" = "$self" ] && return 1
  [ -f "$real/$name" ] && [ -x "$real/$name" ] || return 1
  user_hook="$real/$name"
  return 0
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

const PRE_PUSH_TAIL: &str = r#"input=$(cat; printf x)
input=${input%x}
refused=""
while read -r local_ref local_sha remote_ref remote_sha; do
  case "$remote_ref" in
    refs/heads/main|refs/heads/master) refused=${remote_ref#refs/heads/} ;;
  esac
done <<TYBA_EOF
$input
TYBA_EOF
if [ -n "$refused" ]; then
  echo "TYBA: push para $refused recusado (guarda-corpo fixo do TYBA): agente não empurra para main/master. Faça o push numa branch própria e abra um PR; o merge na main fica com o humano." >&2
  exit 1
fi
tyba_user_hook || exit 0
printf '%s' "$input" | "$user_hook" "$@"
"#;

fn script(name: &str) -> String {
    let tail = match name {
        "pre-push" => PRE_PUSH_TAIL,
        "push-to-checkout" => PUSH_TO_CHECKOUT_TAIL,
        _ => PASSTHROUGH_TAIL,
    };
    format!(
        "#!/bin/sh\n# Hook do TYBA (agente livre): {name}. Ver agent/git_hooks.rs.\n{FIND_USER_HOOK}{tail}"
    )
}

/// Escreve a pasta de hooks (reescreve sempre: o conteúdo é do binário atual).
pub fn write_hooks(dir: &Path) -> std::io::Result<()> {
    for name in HOOK_NAMES {
        crate::session::write_private(dir, name, &script(name))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
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
pub fn with_hooks(env: &mut HashMap<String, String>, dir: &Path) {
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
    let mut env: Vec<_> = env
        .into_iter()
        .filter(|(k, _)| !add.iter().any(|(a, _)| k.as_os_str() == a.as_str()))
        .collect();
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
