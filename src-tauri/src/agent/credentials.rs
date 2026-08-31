//! Entrega B — a credencial do agente sobrevive à jaula no Linux.
//!
//! Três peças, na ordem em que rodam no spawn (§6 do design):
//!
//! 1. `diagnose_state_writability` — diagnóstico sobre o ARGV do bwrap, puro,
//!    sem disco/bwrap/plataforma. Roda em microssegundos.
//! 2. `preflight_claude_dir_writable` — 3 syscalls reais no host, fora da
//!    jaula, mesmo uid do dono. Pega o que o argv não alcança (fs ro,
//!    permissão, disco cheio, chattr +i).
//! 3. `unclassified_claude_children` — o alarme de deriva (§2.5): todo nome no
//!    topo de `~/.claude` fora da tabela classificada gera aviso, uma vez por
//!    execução.
//!
//! Nenhuma das três recusa a sessão — "nunca falha silenciosa" é avisar,
//! nunca é bloquear (o mesmo raciocínio do "o teto não recusa" do ADR
//! 2026-08-22). Fail-closed é para o sandbox falhar; não gravar credencial não
//! é falha de segurança.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::{
    CLAUDE_STATE_TOP_LEVEL_NAMES, SCRIPT_EXTENSIONS, SENSITIVE_CLAUDE_FILES_IF_PRESENT,
    SENSITIVE_CLAUDE_FILES_MANDATORY, SENSITIVE_CLAUDE_READONLY_DIRS,
};

/// Core → webview (§1.2 do design): aviso sem ação, tone warning — distinto
/// do toast acionável de `OpenUrlPayload` (T2), que carrega decisão do dono.
pub const EVENT_SANDBOX_WARNING: &str = "agent://sandbox-warning";

#[derive(Clone, serde::Serialize)]
pub struct SandboxWarningPayload {
    pub session_id: crate::session::SessionId,
    pub kind: SandboxWarningKind,
    pub detail: Option<String>,
}

/// Espelha o enum do contrato core→webview (§1.2 do design). `Copy` porque é
/// pequeno e trafega por valor no payload do evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum SandboxWarningKind {
    CredencialPaiNaoEhRw,
    CredencialSombreadaDepois,
    CredencialHostNaoGrava,
    HomeRoClaudeJsonNaoPersiste,
    FilhoDesconhecidoEmClaude,
}

/// V5: `CLAUDE_CONFIG_DIR` sobrepõe `~/.claude` tanto para `.claude.json`
/// quanto para a credencial.
pub fn claude_config_dir(home: &Path, env: &HashMap<String, String>) -> PathBuf {
    env.get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".claude"))
}

pub fn credential_path(home: &Path, env: &HashMap<String, String>) -> PathBuf {
    claude_config_dir(home, env).join(".credentials.json")
}

fn is_ro_bind_op(op: &OsStr) -> bool {
    op == "--ro-bind" || op == "--ro-bind-try"
}

/// C1 (§6): três checagens sobre o argv já montado do bwrap, sem tocar disco.
/// `claude` é o diretório da credencial (ver `claude_config_dir`).
///
/// - o pai da credencial nunca apareceu como `--bind` ⇒
///   `CredencialPaiNaoEhRw`;
/// - apareceu, mas uma operação POSTERIOR cobre o próprio `claude` ou o
///   arquivo da credencial com `--ro-bind`/`--tmpfs` ⇒
///   `CredencialSombreadaDepois`;
/// - o `$HOME` apareceu como `--ro-bind`/`--ro-bind-try` (§3.7 — dono com
///   `read_allow = ["~"]`) ⇒ `HomeRoClaudeJsonNaoPersiste`.
pub fn diagnose_state_writability(
    argv: &[OsString],
    home: &Path,
    claude: &Path,
) -> Vec<SandboxWarningKind> {
    let credential = claude.join(".credentials.json");
    let mut pai_rw_at: Option<usize> = None;
    let mut home_ro = false;
    let mut shadow_after = false;

    let mut i = 0;
    while i < argv.len() {
        let op = argv[i].as_os_str();
        if op == "--bind" && i + 2 < argv.len() {
            if Path::new(&argv[i + 2]) == claude {
                pai_rw_at = Some(i);
            }
            i += 3;
            continue;
        }
        if is_ro_bind_op(op) && i + 2 < argv.len() {
            let dest = Path::new(&argv[i + 2]);
            if dest == home {
                home_ro = true;
            }
            if pai_rw_at.is_some_and(|at| i > at) && (dest == claude || dest == credential) {
                shadow_after = true;
            }
            i += 3;
            continue;
        }
        if op == "--tmpfs" && i + 1 < argv.len() {
            let dest = Path::new(&argv[i + 1]);
            if pai_rw_at.is_some_and(|at| i > at) && (dest == claude || dest == credential) {
                shadow_after = true;
            }
            i += 2;
            continue;
        }
        i += 1;
    }

    let mut warnings = Vec::new();
    if pai_rw_at.is_none() {
        warnings.push(SandboxWarningKind::CredencialPaiNaoEhRw);
    } else if shadow_after {
        warnings.push(SandboxWarningKind::CredencialSombreadaDepois);
    }
    if home_ro {
        warnings.push(SandboxWarningKind::HomeRoClaudeJsonNaoPersiste);
    }
    warnings
}

/// C2 (§6): preflight real, no host, fora da jaula, mesmo uid do dono. Três
/// syscalls — create, rename, unlink — o mesmo padrão tmp+rename que o
/// próprio Claude Code usa para gravar a credencial (M2), então o que passa
/// aqui é exatamente o que o binário vai tentar fazer depois. NUNCA recusa a
/// sessão: o chamador decide o que fazer com o `Err` (§6 — avisa e sobe).
pub fn preflight_claude_dir_writable(claude: &Path) -> Result<(), String> {
    let probe_tmp = claude.join(format!(".tyba-write-probe.tmp.{}", std::process::id()));
    let probe = claude.join(".tyba-write-probe");
    let fail = |e: std::io::Error| format!("{}: {e}", claude.display());

    std::fs::write(&probe_tmp, b"").map_err(fail)?;
    if let Err(e) = std::fs::rename(&probe_tmp, &probe) {
        let _ = std::fs::remove_file(&probe_tmp);
        return Err(fail(e));
    }
    std::fs::remove_file(&probe).map_err(fail)?;
    Ok(())
}

/// §2.5 — o alarme de deriva. Tudo que está no topo de `claude` e não bate com
/// a tabela classificada (sombreados §2.2/§2.3 + estado conhecido §2.4 + a
/// mesma detecção por forma que `sensitive_claude_children` usa para scripts
/// sem nome fixo, V9) é "novo e não classificado" — continua gravável (é
/// estado, por default), mas o dono passa a saber que existe.
pub(crate) fn unclassified_claude_children(claude: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(claude) else {
        return Vec::new();
    };
    let mut unknown: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_classified_claude_child(&name, &entry.path()) {
                None
            } else {
                Some(name)
            }
        })
        .collect();
    unknown.sort();
    unknown.dedup();
    unknown
}

fn is_classified_claude_child(name: &str, path: &Path) -> bool {
    if name == "projects"
        || SENSITIVE_CLAUDE_READONLY_DIRS.contains(&name)
        || SENSITIVE_CLAUDE_FILES_MANDATORY.contains(&name)
        || SENSITIVE_CLAUDE_FILES_IF_PRESENT.contains(&name)
        || CLAUDE_STATE_TOP_LEVEL_NAMES.contains(&name)
    {
        return true;
    }
    if path.is_file() {
        let script_ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| SCRIPT_EXTENSIONS.contains(&ext))
            .unwrap_or(false);
        if script_ext || super::is_executable(path) {
            return true;
        }
    }
    false
}

/// Dedupe em processo: uma vez por nome por execução do TYBA (§6). Reiniciar
/// o app rearma o aviso de propósito — é o certo para item de segurança não
/// classificado, ao contrário de um toast comum.
static WARNED_UNCLASSIFIED: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = std::sync::OnceLock::new();

pub(crate) fn drift_alarm_names(claude: &Path) -> Vec<String> {
    let unknown = unclassified_claude_children(claude);
    let seen =
        WARNED_UNCLASSIFIED.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut guard = seen.lock().expect("drift alarm lock");
    unknown
        .into_iter()
        .filter(|n| guard.insert(n.clone()))
        .collect()
}

/// Fio de produção (§6, chamado por `session::spawn_prepared` — Linux, só
/// Claude Code): roda C1 + C2 + o alarme de deriva e emite um
/// `agent://sandbox-warning` por achado. Nunca devolve erro, nunca bloqueia o
/// spawn — item 17 do contrato de cobertura: a sessão sobe mesmo com
/// preflight falho, só com aviso.
pub(crate) fn emit_warnings(
    app: &tauri::AppHandle,
    session_id: crate::session::SessionId,
    env: &HashMap<String, String>,
    spec: &crate::sandbox::SandboxSpec,
) {
    use tauri::Emitter;

    let claude = claude_config_dir(&spec.home, env);
    let mut findings: Vec<(SandboxWarningKind, Option<String>)> = Vec::new();

    if let Ok(argv) = crate::sandbox::bwrap::build_args(spec) {
        findings.extend(
            diagnose_state_writability(&argv, &spec.home, &claude)
                .into_iter()
                .map(|kind| (kind, None)),
        );
    }

    if let Err(detail) = preflight_claude_dir_writable(&claude) {
        findings.push((SandboxWarningKind::CredencialHostNaoGrava, Some(detail)));
    }

    let unknown = drift_alarm_names(&claude);
    if !unknown.is_empty() {
        findings.push((
            SandboxWarningKind::FilhoDesconhecidoEmClaude,
            Some(unknown.join(", ")),
        ));
    }

    for (kind, detail) in findings {
        let _ = app.emit(
            EVENT_SANDBOX_WARNING,
            SandboxWarningPayload {
                session_id,
                kind,
                detail,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn accuses_pai_nao_bindado_when_claude_never_appears_as_bind() {
        let home = Path::new("/home/x");
        let claude = home.join(".claude");
        let argv = os(&["--ro-bind", "/usr", "/usr", "--tmpfs", "/tmp"]);
        let warnings = diagnose_state_writability(&argv, home, &claude);
        assert_eq!(warnings, vec![SandboxWarningKind::CredencialPaiNaoEhRw]);
    }

    #[test]
    fn accuses_pai_coberto_depois_when_a_later_tmpfs_shadows_the_whole_dir() {
        let home = Path::new("/home/x");
        let claude = home.join(".claude");
        let claude_s = claude.to_string_lossy().into_owned();
        let argv = os(&["--bind", &claude_s, &claude_s, "--tmpfs", &claude_s]);
        let warnings = diagnose_state_writability(&argv, home, &claude);
        assert_eq!(
            warnings,
            vec![SandboxWarningKind::CredencialSombreadaDepois]
        );
    }

    #[test]
    fn accuses_arquivo_mascarado_depois_when_only_the_credential_file_is_shadowed_later() {
        let home = Path::new("/home/x");
        let claude = home.join(".claude");
        let claude_s = claude.to_string_lossy().into_owned();
        let cred = claude
            .join(".credentials.json")
            .to_string_lossy()
            .into_owned();
        let argv = os(&["--bind", &claude_s, &claude_s, "--ro-bind", &cred, &cred]);
        let warnings = diagnose_state_writability(&argv, home, &claude);
        assert_eq!(
            warnings,
            vec![SandboxWarningKind::CredencialSombreadaDepois]
        );
    }

    #[test]
    fn does_not_accuse_shadow_when_the_ro_bind_of_the_credential_comes_before_the_rw_bind() {
        // sombra ANTES do bind rw é a ordem correta (write allow enterra a
        // sombra por cima, como o resto da entrega B monta de propósito) — o
        // diagnóstico não pode confundir isso com "sombreado depois".
        let home = Path::new("/home/x");
        let claude = home.join(".claude");
        let claude_s = claude.to_string_lossy().into_owned();
        let cred = claude
            .join(".credentials.json")
            .to_string_lossy()
            .into_owned();
        let argv = os(&["--ro-bind", &cred, &cred, "--bind", &claude_s, &claude_s]);
        let warnings = diagnose_state_writability(&argv, home, &claude);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn accuses_home_ro_bindado_via_ro_bind_try_the_mechanism_read_allow_extra_uses() {
        let home = Path::new("/home/x");
        let claude = home.join(".claude");
        let home_s = home.to_string_lossy().into_owned();
        let claude_s = claude.to_string_lossy().into_owned();
        let argv = os(&[
            "--ro-bind-try",
            &home_s,
            &home_s,
            "--bind",
            &claude_s,
            &claude_s,
        ]);
        let warnings = diagnose_state_writability(&argv, home, &claude);
        assert!(warnings.contains(&SandboxWarningKind::HomeRoClaudeJsonNaoPersiste));
    }

    /// Item 15 do contrato: aceita o argv REAL da política de produção sem
    /// falso positivo — não uma paráfrase do argv, o `build_args` de verdade.
    #[test]
    fn real_argv_from_production_policy_raises_no_false_positive() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let repo = root.join("repo");
        let wt = root.join("wt-a");
        let runtime = root.join("tyba-rt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(repo.join(".git/worktrees/wt-a")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../repo/.git/worktrees/wt-a").unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        let exe = root.join("bin/tyba");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();

        use crate::agent::{AgentRunner, ClaudeCodeRunner};
        let agent = ClaudeCodeRunner.sandbox_access(&home, &wt);
        let spec = crate::sandbox::SandboxSpec {
            writable_root: wt.clone(),
            readable_root: repo.clone(),
            allow_network: true,
            repo_git_dir: repo.join(".git"),
            worktree_git_dir: repo.join(".git/worktrees/wt-a"),
            runtime_dir: runtime.clone(),
            hook_socket: runtime.join("hook.sock"),
            tyba_exe: exe,
            tyba_data_dir: home.join(".local/share/dev.tyba.app"),
            home: home.clone(),
            tmpdir: None,
            exec_path_dirs: vec![],
            agent,
            read_allow_extra: vec![],
            data_dir_reads: vec![],
        };

        let argv = crate::sandbox::bwrap::build_args(&spec).unwrap();
        let claude = home.join(".claude");
        let warnings = diagnose_state_writability(&argv, &home, &claude);
        assert!(
            warnings.is_empty(),
            "a política de produção não pode disparar falso positivo: {warnings:?}"
        );
    }

    #[test]
    fn credential_path_honors_claude_config_dir_override() {
        let home = Path::new("/home/x");
        let mut env = HashMap::new();
        assert_eq!(
            credential_path(home, &env),
            home.join(".claude/.credentials.json")
        );
        env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            "/mnt/claude-cfg".to_string(),
        );
        assert_eq!(
            credential_path(home, &env),
            PathBuf::from("/mnt/claude-cfg/.credentials.json")
        );
    }

    /// Item 16: preflight acusa dir sem escrita com o path no detalhe. ENOENT
    /// (pai ausente) falha independente de uid — inclusive rodando como root,
    /// ao contrário de um `chmod` (root ignora bits DAC). O `detail` viaja
    /// cru até o webview (`SandboxWarningPayload`) — quem monta a frase em
    /// pt-BR é o front (T4, `sandboxWarning.ts`), o mesmo desenho do
    /// `OpenUrlPayload`, cuja cópia também é decidida do lado do webview.
    #[test]
    fn preflight_error_carries_the_path_for_the_frontend_to_render() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("nao-existe/.claude");
        let err = preflight_claude_dir_writable(&claude).unwrap_err();
        assert!(err.contains(&claude.display().to_string()), "{err}");
    }

    #[test]
    fn preflight_succeeds_on_a_writable_dir_and_leaves_no_probe_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        preflight_claude_dir_writable(&claude).unwrap();
        assert_eq!(std::fs::read_dir(&claude).unwrap().count(), 0);
    }

    /// Item 18: dispara para nome não classificado, e só uma vez por nome por
    /// execução — nomes com UUID pra não colidir com outro teste do mesmo
    /// binário (o dedupe é global-por-processo, de propósito, §6).
    #[test]
    fn drift_alarm_fires_once_per_unclassified_name_per_process() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let mystery = format!("mystery-{}", uuid::Uuid::new_v4());
        std::fs::write(claude.join(&mystery), "x").unwrap();
        std::fs::create_dir_all(claude.join("backups")).unwrap();

        let first = drift_alarm_names(&claude);
        assert!(first.contains(&mystery), "{first:?}");
        assert!(
            !first.contains(&"backups".to_string()),
            "estado conhecido não dispara alarme: {first:?}"
        );

        let second = drift_alarm_names(&claude);
        assert!(
            !second.contains(&mystery),
            "o mesmo nome não pode reavisar na mesma execução: {second:?}"
        );
    }

    #[test]
    fn unclassified_claude_children_ignores_script_shaped_files_by_shape_not_name() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("statusline-command.sh"), "#!/bin/sh\n").unwrap();
        let unknown = unclassified_claude_children(&claude);
        assert!(
            !unknown.contains(&"statusline-command.sh".to_string()),
            "script já classificado por sensitive_claude_children não pode reaparecer como \
             deriva: {unknown:?}"
        );
    }
}
