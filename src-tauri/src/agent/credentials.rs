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

// Só consumido pelo alarme de deriva (linux-only, ver o bloco cfg mais
// abaixo) -- gated junto, senão vira `unused_imports` no mac/windows (os
// quatro nomes sem cfg em mod.rs continuam existindo lá, só ficam sem uso
// neste arquivo).
#[cfg(target_os = "linux")]
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
    /// Só populado para `FilhoDesconhecidoEmClaude` (review r1, v0.6.2): os
    /// mesmos nomes de `detail` (que é só pra exibição, `join(", ")`), mas
    /// estruturados — é o que o front devolve pro comando `ack_drift_warning`
    /// quando o dono dispensa o toast, pra `ack_drift_names` marcar durável
    /// SÓ o que foi de fato visto. Separado de `detail` pra não depender de
    /// fazer `split(", ")` num nome de arquivo que, em tese, pode conter
    /// vírgula.
    pub names: Option<Vec<String>>,
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
    // Path primeiro, causa entre parênteses (review r1, NIT): "detail" vira
    // um fragmento pronto pra encaixar no FIM de uma frase pt-BR
    // ("...é gravável ({detail})"), sem o errno cortando a sentença ao meio.
    let fail = |e: std::io::Error| format!("{} ({e})", claude.display());

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
///
/// `cfg(linux)`: a parte de DETECÇÃO do alarme de deriva (esta função +
/// `is_classified_claude_child` + `drift_toast_worthy` + `drift_alarm_names`)
/// só é alcançada por `emit_warnings`, chamado só em `session.rs` sob o
/// mesmo `#[cfg(target_os = "linux")]` — a jaula/credencial de B é Linux
/// (mac já tinha o Keychain corrigido em separado, #194). Sem o gate aqui,
/// esses símbolos ficam sem nenhum chamador em produção fora do Linux e o
/// clippy -D warnings do mac/windows reprova com dead_code (achado da CI do
/// PR #299) — eles são `pub(crate)`/privados, não `pub`, então não entram na
/// isenção que o lint dá pra API pública de uma lib crate. `load_warned` /
/// `save_warned` / `ack_drift_names` NÃO entram nesse gate (review r1,
/// v0.6.2): a ESCRITA do dedupe precisa existir em todo SO porque o comando
/// Tauri `ack_drift_warning` é registrado sem cfg (ver o comentário de
/// `DRIFT_WARNED_KEY`).
#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

/// Chave no `settings` (key/value) que guarda os nomes de filho
/// não-classificado JÁ avisados por toast (v0.6.2, item 1 do contrato de
/// cobertura de "polir o alarme de deriva"). Substitui o antigo
/// `WARNED_UNCLASSIFIED` in-process: aquele rearmava a cada reinício do TYBA
/// de propósito, mas isso virou o barulho que o dono relatou — o mesmo nome
/// (ex.: `vercel-plugin-device-id`) voltava a gritar toda vez que o app
/// abria. Não é `pref.*` (não é preferência editável pelo dono em UI, é
/// estado interno do alarme) — mesma convenção de `theme::KEY_MODE` /
/// `update::KEY_LATEST`, que também não passam pelo allowlist de
/// `get_pref`/`set_pref`.
///
/// Não é `cfg(linux)`: `ack_drift_names` (chamada pelo comando Tauri
/// `ack_drift_warning`, registrado em todo SO) precisa compilar em
/// mac/windows mesmo que `FilhoDesconhecidoEmClaude` nunca seja emitido lá —
/// senão o `invoke` do front não resolveria o comando fora do Linux (review
/// r1, v0.6.2).
const DRIFT_WARNED_KEY: &str = "drift.warned_unclassified_children";

/// Puro — recebe o que já foi avisado e devolve só os nomes NOVOS. Separado
/// da leitura/escrita no store para ficar testável sem SQLite (a fronteira
/// de I/O é o que se isola, não a lista em si).
///
/// `cfg(linux)`: só `drift_alarm_names` chama esta função, e ela é
/// linux-only — mesmo motivo do gate em `unclassified_claude_children` logo
/// abaixo (achado do clippy --all-targets cross windows-gnu, v0.6.2).
#[cfg(target_os = "linux")]
pub(crate) fn names_not_yet_warned(
    candidates: &[String],
    already_warned: &std::collections::HashSet<String>,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|n| !already_warned.contains(n.as_str()))
        .cloned()
        .collect()
}

fn load_warned(store: &crate::session::store::Store) -> std::collections::HashSet<String> {
    store
        .get_setting(DRIFT_WARNED_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

fn save_warned(store: &crate::session::store::Store, warned: &std::collections::HashSet<String>) {
    // Ordenado antes de serializar: sem isso a ordem de um HashSet varia
    // entre execuções e o JSON persistido muda mesmo quando o conjunto é o
    // mesmo — ruído no diff, nada de comportamento.
    let mut list: Vec<&String> = warned.iter().collect();
    list.sort();
    if let Ok(json) = serde_json::to_string(&list) {
        let _ = store.set_setting(DRIFT_WARNED_KEY, &json);
    }
}

/// Review r1 (v0.6.2), MAJOR: o ÚNICO lugar que marca um nome como
/// "avisado" pra valer — chamado pelo comando Tauri `ack_drift_warning`
/// quando o FRONT confirma que o toast de `FilhoDesconhecidoEmClaude`
/// realmente apareceu na tela e foi dispensado (X, clique ou auto-dismiss
/// depois de renderizado — nunca antes disso). `drift_alarm_names` (abaixo)
/// só LÊ o que já foi confirmado; nunca persiste sozinho.
///
/// Por quê: `drift_alarm_names` persistia direto na DETECÇÃO, antes de
/// `emit_warnings` sequer tentar emitir o evento — e `app.emit` é
/// descartado (`let _ = `) se não houver listener no instante exato (a
/// mesma janela de corrida do `app://ready` documentada no listener de
/// `App.tsx`). Resultado: o toast se perdia E, por ser durável, nunca mais
/// reaparecia — um alarme de segurança podia ficar permanentemente mudo por
/// pura corrida de evento, sem o dono nunca ter visto nada. Marcar só no ack
/// do front devolve a rede segura que o dedupe in-process antigo tinha
/// (reaparecia a cada restart): agora reaparece a cada SPAWN de sessão até
/// realmente ser visto — o ônus é ficar re-oferecendo o toast até a
/// confirmação chegar, nunca o oposto.
pub(crate) fn ack_drift_names(store: &crate::session::store::Store, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let allowed = only_emitted_this_run(names);
    if allowed.is_empty() {
        return;
    }
    let mut warned = load_warned(store);
    warned.extend(allowed);
    save_warned(store, &warned);
}

/// Review de segurança r2 (v0.6.2), OPCIONAL — defesa em profundidade: o
/// webview é confiável (não é a fronteira de ameaça real; quem já
/// conseguisse rodar JS arbitrário ali comprometeu o app inteiro, não só
/// este comando), então isto não é obrigatório — mas é barato. Sem isto,
/// `ack_drift_warning` (comando Tauri) aceitaria QUALQUER `names` que o
/// front mandasse, sem checar que o core de fato ofereceu aquele nome como
/// toast. Com isto, só nomes que passaram por `record_emitted_drift_names`
/// (chamado de `emit_warnings`, junto com o emit do evento) podem virar
/// durável — um ack pra um nome nunca emitido é descartado em silêncio.
static EMITTED_DRIFT_NAMES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = std::sync::OnceLock::new();

fn emitted_drift_names_set() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    EMITTED_DRIFT_NAMES.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Chamado por `emit_warnings` (nunca pelo comando de ack): registra que o
/// core OFERECEU este nome como toast nesta execução do app. Em processo,
/// não durável de propósito — é só o allowlist do ack, não o dedupe (esse
/// já é o store, via `load_warned`/`save_warned`).
///
/// `cfg(linux)`: o único chamador é `drift_alarm_names`, linux-only (ver o
/// gate na definição dela) — sem este cfg aqui o clippy --all-targets do
/// mac/windows reprova com dead_code (mesmo achado do cross windows-gnu que
/// já pegou `names_not_yet_warned`). `only_emitted_this_run`/
/// `emitted_drift_names_set`/`EMITTED_DRIFT_NAMES` continuam SEM cfg: são
/// chamados também por `ack_drift_names`, que é cross-platform (o comando
/// Tauri precisa existir em todo SO).
#[cfg(target_os = "linux")]
fn record_emitted_drift_names(names: &[String]) {
    if names.is_empty() {
        return;
    }
    let mut guard = emitted_drift_names_set()
        .lock()
        .expect("emitted drift names lock");
    guard.extend(names.iter().cloned());
}

fn only_emitted_this_run(names: &[String]) -> Vec<String> {
    let guard = emitted_drift_names_set()
        .lock()
        .expect("emitted drift names lock");
    names
        .iter()
        .filter(|n| guard.contains(n.as_str()))
        .cloned()
        .collect()
}

/// Item 2/3 do contrato: o toast de deriva só dispara pra nome com "cara de
/// risco de execução" — diretório (pode carregar hook/script dentro) ou
/// arquivo com forma de script (mesma extensão/bit de execução que
/// `sensitive_claude_children` usa para achar o statusline do dono, V9).
/// Arquivo de estado sem essa forma (ex.: `vercel-plugin-device-id`) some do
/// toast mas continua em `unclassified_claude_children` — a detecção não
/// muda, só o que vira barulho.
#[cfg(target_os = "linux")]
fn drift_toast_worthy(name: &str, claude: &Path) -> bool {
    let path = claude.join(name);
    if path.is_dir() {
        return true;
    }
    if path.is_file() {
        let script_ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| SCRIPT_EXTENSIONS.contains(&ext))
            .unwrap_or(false);
        return script_ext || super::is_executable(&path);
    }
    false
}

/// Nomes de filho não-classificado que merecem TOAST agora: shape de risco
/// (`drift_toast_worthy`, item 2/3) E ainda não avisados (dedupe durável no
/// store, item 1). `unclassified_claude_children` continua a detecção
/// completa e sem filtro — só este passo decide o que vira barulho.
#[cfg(target_os = "linux")]
/// Review r1 (v0.6.2), MAJOR: só LÊ o store (`load_warned`) — nunca
/// escreve. A escrita é `ack_drift_names`, chamada só depois que o front
/// confirma que o dono viu o toast (ver o comentário lá). Um nome pode
/// voltar aqui em toda chamada até o ack chegar — é o comportamento
/// correto, não um bug: reoferece até ser realmente visto.
///
/// Review r1, MINOR: os nomes benignos filtrados por `drift_toast_worthy`
/// (item 2/3, sem cara de risco) não somem — `unclassified_claude_children`
/// já os detecta, e aqui cada um vira uma linha em stderr (mesma convenção
/// de diagnóstico do resto do binário, ex. `disk_observer.rs`,
/// `boot.rs` — não há `log`/`tracing` neste crate). Satisfaz "continua
/// detectado e REGISTRADO — nunca deixa de ser visto, só deixa de gritar"
/// (critério 2 da spec de deriva) sem inventar dependência de log nova pra
/// um MINOR.
#[cfg(target_os = "linux")]
pub(crate) fn drift_alarm_names(
    claude: &Path,
    store: &crate::session::store::Store,
) -> Vec<String> {
    let unknown = unclassified_claude_children(claude);
    let mut risky = Vec::new();
    for name in unknown {
        if drift_toast_worthy(&name, claude) {
            risky.push(name);
        } else {
            eprintln!(
                "[tyba] alarme de deriva: \"{name}\" é estado benigno em ~/.claude (sem cara \
                 de risco) — continua gravável, registrado, sem toast"
            );
        }
    }
    let warned = load_warned(store);
    let fresh = names_not_yet_warned(&risky, &warned);
    // Review de segurança r2, OPCIONAL: registra AQUI, não em
    // `emit_warnings` — é o ponto único onde "o core decidiu oferecer X
    // como toast" é verdade, e `drift_alarm_names` é chamada direto pelos
    // testes (sem passar por `emit_warnings`), então gravar aqui casa o
    // allowlist do ack com o que a função realmente devolve, produção ou
    // teste.
    record_emitted_drift_names(&fresh);
    fresh
}

/// Fio de produção (§6, chamado por `session::spawn_prepared` — Linux, só
/// Claude Code): roda C1 + C2 + o alarme de deriva e emite um
/// `agent://sandbox-warning` por achado. Nunca devolve erro, nunca bloqueia o
/// spawn — item 17 do contrato de cobertura: a sessão sobe mesmo com
/// preflight falho, só com aviso.
#[cfg(target_os = "linux")]
pub(crate) fn emit_warnings(
    app: &tauri::AppHandle,
    session_id: crate::session::SessionId,
    env: &HashMap<String, String>,
    spec: &crate::sandbox::SandboxSpec,
    store: &crate::session::store::Store,
) {
    use tauri::Emitter;

    let claude = claude_config_dir(&spec.home, env);
    // `names` só é `Some` pro achado de deriva (review r1, v0.6.2) — é o que
    // `ack_drift_names` precisa de volta do front pra marcar durável só o
    // que foi realmente visto (ver o comentário de `drift_alarm_names`).
    let mut findings: Vec<(SandboxWarningKind, Option<String>, Option<Vec<String>>)> = Vec::new();

    if let Ok(argv) = crate::sandbox::bwrap::build_args(spec) {
        findings.extend(
            diagnose_state_writability(&argv, &spec.home, &claude)
                .into_iter()
                .map(|kind| (kind, None, None)),
        );
    }

    if let Err(detail) = preflight_claude_dir_writable(&claude) {
        findings.push((
            SandboxWarningKind::CredencialHostNaoGrava,
            Some(detail),
            None,
        ));
    }

    let unknown = drift_alarm_names(&claude, store);
    if !unknown.is_empty() {
        findings.push((
            SandboxWarningKind::FilhoDesconhecidoEmClaude,
            Some(unknown.join(", ")),
            Some(unknown),
        ));
    }

    for (kind, detail, names) in findings {
        let _ = app.emit(
            EVENT_SANDBOX_WARNING,
            SandboxWarningPayload {
                session_id,
                kind,
                detail,
                names,
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

    /// NIT do review r1: path primeiro, causa entre parênteses -- pra
    /// "detail" render num FIM de frase pt-BR ("...é gravável ({detail})")
    /// sem o errno cortando a sentença no meio (era "{path}: {errno}", que
    /// interpolado no meio de "...confirmar que {{detail}} é gravável"
    /// lia "...confirmar que /home/x/.claude: Permission denied é
    /// gravável" -- a causa aparecia antes do verbo).
    #[test]
    fn preflight_error_puts_the_path_first_and_the_cause_in_parens() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("nao-existe/.claude");
        let err = preflight_claude_dir_writable(&claude).unwrap_err();
        let path = claude.display().to_string();
        assert!(
            err.starts_with(&path),
            "path precisa vir primeiro, causa depois: {err}"
        );
        let rest = &err[path.len()..];
        assert!(
            rest.trim_start().starts_with('('),
            "a causa precisa vir entre parênteses, separada do path: {err}"
        );
    }

    #[test]
    fn preflight_succeeds_on_a_writable_dir_and_leaves_no_probe_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        preflight_claude_dir_writable(&claude).unwrap();
        assert_eq!(std::fs::read_dir(&claude).unwrap().count(), 0);
    }

    /// Review r1 (v0.6.2), MAJOR — o teste que o review pediu: "nome
    /// detectado mas emit sem listener → NÃO fica marcado durável
    /// (re-avisaria)". `drift_alarm_names` sozinho (sem `ack_drift_names`)
    /// NUNCA persiste — chamar duas, três vezes seguidas continua
    /// oferecendo o mesmo nome, porque nenhuma delas é "o dono viu". Só
    /// depois do ack explícito é que some.
    ///
    /// Substitui `drift_alarm_names_fires_once_per_toast_worthy_name_then_
    /// dedupes` (que assumia dedupe na própria detecção): aquele
    /// comportamento tinha o furo que este review achou — `emit_warnings`
    /// descarta o evento (`let _ = app.emit(...)`) se não houver listener no
    /// instante exato (a mesma corrida do `app://ready` documentada no
    /// listener de `App.tsx`), e persistir ANTES de saber se o toast
    /// realmente chegou silenciava o alarme pra sempre sem o dono nunca ter
    /// visto nada.
    ///
    /// `cfg(linux)`: testa `drift_alarm_names`, linux-only (ver o gate na
    /// definição) — sem este cfg aqui o mac/windows nem compilam o `cargo
    /// test` (função inexistente).
    #[test]
    #[cfg(target_os = "linux")]
    fn drift_alarm_names_keeps_reoffering_until_acked() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        // Diretório, não arquivo `.sh`: um arquivo com cara de script no topo
        // já é classificado por `is_classified_claude_child` (mesma checagem
        // de forma que `sensitive_claude_children` usa pra sombrear, V9) —
        // ele NUNCA chega a "não classificado", então nunca alcançaria
        // `drift_alarm_names` por esse caminho. Diretório desconhecido é o
        // shape que sobrevive até aqui; ver `drift_toast_worthy_flags_dirs_
        // and_script_shaped_or_executable_files` para a checagem de forma em
        // isolamento, sem depender desse acaso do pipeline.
        let mystery = format!("mystery-dir-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(claude.join(&mystery)).unwrap();
        std::fs::create_dir_all(claude.join("backups")).unwrap();
        let store = crate::session::store::Store::open_in_memory().unwrap();

        let first = drift_alarm_names(&claude, &store);
        assert!(first.contains(&mystery), "{first:?}");
        assert!(
            !first.contains(&"backups".to_string()),
            "estado conhecido não dispara alarme: {first:?}"
        );

        // Simula o emit se perder (sem listener): NENHUM ack aconteceu.
        let second = drift_alarm_names(&claude, &store);
        assert!(
            second.contains(&mystery),
            "sem ack, o nome precisa continuar sendo oferecido — a detecção sozinha não pode \
             marcar durável, senão um evento perdido silencia o alarme pra sempre: {second:?}"
        );

        // Só agora o front "viu e dispensou" o toast.
        ack_drift_names(&store, std::slice::from_ref(&mystery));
        let third = drift_alarm_names(&claude, &store);
        assert!(
            !third.contains(&mystery),
            "depois do ack, o nome não pode reaparecer: {third:?}"
        );
    }

    /// v0.6.2, item 1 do contrato + review r1: o nome avisado sobrevive a um
    /// restart de verdade do app (store fechado e reaberto do zero, arquivo
    /// em disco, não `open_in_memory`) — mas SÓ depois de `ack_drift_names`,
    /// que é o único jeito de marcar durável desde o review r1 (MAJOR).
    #[test]
    #[cfg(target_os = "linux")]
    fn drift_alarm_dedupe_survives_a_restart_of_the_app_once_acked() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let db = tmp.path().join("tyba.db");
        std::fs::create_dir_all(&claude).unwrap();
        let mystery_dir = format!("mystery-dir-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(claude.join(&mystery_dir)).unwrap();

        {
            let store = crate::session::store::Store::open(&db).unwrap();
            let first = drift_alarm_names(&claude, &store);
            assert!(first.contains(&mystery_dir), "{first:?}");
            // "O dono viu e dispensou o toast" — sem isto, nada persiste.
            ack_drift_names(&store, &first);
        }

        // "Reinicia o app": Store novo, do zero, sobre o MESMO arquivo —
        // nada em memória sobrevive além do que foi persistido.
        let store = crate::session::store::Store::open(&db).unwrap();
        let after_restart = drift_alarm_names(&claude, &store);
        assert!(
            !after_restart.contains(&mystery_dir),
            "nome já ACKED antes do restart não pode reavisar depois: {after_restart:?}"
        );
    }

    /// v0.6.2, item 3 do contrato: `names_not_yet_warned` é a peça pura do
    /// dedupe — testável sem SQLite.
    ///
    /// `cfg(linux)`: `names_not_yet_warned` é linux-only (ver o gate na
    /// definição) — sem este cfg aqui o mac/windows nem compilam o `cargo
    /// test` (função inexistente).
    #[test]
    #[cfg(target_os = "linux")]
    fn names_not_yet_warned_filters_out_already_warned() {
        let mut warned = std::collections::HashSet::new();
        warned.insert("a".to_string());
        let out = names_not_yet_warned(&["a".to_string(), "b".to_string()], &warned);
        assert_eq!(out, vec!["b".to_string()]);
    }

    /// Review r1 (v0.6.2), MAJOR: `ack_drift_names` é a ÚNICA escrita real
    /// do dedupe — round-trip puro (sem `drift_alarm_names` no meio), prova
    /// que persiste e que `load_warned` enxerga depois. `ack_drift_names`
    /// em si NÃO é linux-only (comando Tauri precisa existir em todo SO —
    /// ver o comentário de `DRIFT_WARNED_KEY`), mas este teste PRECISA do
    /// gate porque usa `record_emitted_drift_names` (o allowlist do review
    /// de segurança r2, OPCIONAL), que é linux-only — ver o comentário na
    /// definição dela.
    ///
    /// `record_emitted_drift_names` primeiro: sem isso, `ack_drift_names`
    /// descartaria os dois nomes por nunca terem sido "oferecidos" nesta
    /// execução — ver `ack_drift_names_ignores_names_the_core_never_emitted`
    /// pra prova dedicada desse allowlist. UUID nos nomes:
    /// `EMITTED_DRIFT_NAMES` é `static` global, compartilhado entre testes
    /// rodando em paralelo no mesmo binário — sem nome único, um teste
    /// poderia "herdar" o registro de outro.
    #[test]
    #[cfg(target_os = "linux")]
    fn ack_drift_names_persists_and_load_warned_sees_it() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        assert!(load_warned(&store).is_empty());
        let um = format!("um-{}", uuid::Uuid::new_v4());
        let dois = format!("dois-{}", uuid::Uuid::new_v4());
        record_emitted_drift_names(&[um.clone(), dois.clone()]);

        ack_drift_names(&store, &[um.clone(), dois.clone()]);
        let warned = load_warned(&store);
        assert!(warned.contains(&um));
        assert!(warned.contains(&dois));
    }

    /// Review r1: lista vazia é no-op — não grava um `[]` por cima do que já
    /// existia (defesa contra um ack tardio/duplicado limpar o store).
    /// `cfg(linux)`: usa `record_emitted_drift_names`, linux-only.
    #[test]
    #[cfg(target_os = "linux")]
    fn ack_drift_names_with_empty_list_does_not_touch_the_store() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        let um = format!("um-{}", uuid::Uuid::new_v4());
        record_emitted_drift_names(std::slice::from_ref(&um));
        ack_drift_names(&store, std::slice::from_ref(&um));
        ack_drift_names(&store, &[]);
        let warned = load_warned(&store);
        assert!(warned.contains(&um), "{warned:?}");
    }

    /// Review de segurança r2 (v0.6.2), OPCIONAL: o ack só marca durável
    /// nomes que o core de fato registrou como emitidos nesta execução —
    /// uma string arbitrária que o webview mandasse sem o core ter
    /// oferecido aquele nome é descartada em silêncio, não vira durável.
    /// `cfg(linux)`: usa `record_emitted_drift_names`, linux-only.
    #[test]
    #[cfg(target_os = "linux")]
    fn ack_drift_names_ignores_names_the_core_never_emitted() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        let emitted = format!("emitted-{}", uuid::Uuid::new_v4());
        let forged = format!("forged-{}", uuid::Uuid::new_v4());
        record_emitted_drift_names(std::slice::from_ref(&emitted));

        ack_drift_names(&store, &[emitted.clone(), forged.clone()]);
        let warned = load_warned(&store);
        assert!(warned.contains(&emitted), "{warned:?}");
        assert!(
            !warned.contains(&forged),
            "nome nunca emitido pelo core não pode virar durável: {warned:?}"
        );
    }

    /// v0.6.2, item 2/5 do contrato: o toast só dispara pra shape de risco
    /// (diretório ou script) — um arquivo de estado benigno tipo
    /// `vercel-plugin-device-id` (sem extensão de script, sem bit de
    /// execução) continua DETECTADO por `unclassified_claude_children`
    /// (nunca some da visibilidade), mas não vira barulho.
    #[test]
    #[cfg(target_os = "linux")]
    fn drift_alarm_only_toasts_dir_or_script_shaped_unclassified_names() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("vercel-plugin-device-id"), "abc123").unwrap();
        let risky_dir = format!("plugin-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(claude.join(&risky_dir)).unwrap();

        let unknown = unclassified_claude_children(&claude);
        assert!(
            unknown.contains(&"vercel-plugin-device-id".to_string()),
            "estado benigno precisa continuar detectado, mesmo sem gritar: {unknown:?}"
        );
        assert!(unknown.contains(&risky_dir));

        let store = crate::session::store::Store::open_in_memory().unwrap();
        let toasted = drift_alarm_names(&claude, &store);
        assert!(
            !toasted.contains(&"vercel-plugin-device-id".to_string()),
            "arquivo de estado benigno não pode virar toast: {toasted:?}"
        );
        assert!(toasted.contains(&risky_dir), "{toasted:?}");
    }

    /// v0.6.2: `drift_toast_worthy` em isolamento, sem depender de o
    /// pipeline de `unclassified_claude_children` conseguir produzir um
    /// arquivo de forma script (hoje não consegue -- ver o comentário de
    /// `drift_alarm_names_fires_once_per_toast_worthy_name_then_dedupes`).
    /// Trava a semântica da função mesmo que essa coincidência do pipeline
    /// mude no futuro.
    #[test]
    #[cfg(target_os = "linux")]
    fn drift_toast_worthy_flags_dirs_and_script_shaped_or_executable_files() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(claude.join("some-dir")).unwrap();
        std::fs::write(claude.join("script.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(claude.join("state-file"), "abc").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(claude.join("exec-no-ext"), "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(
                claude.join("exec-no-ext"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            assert!(drift_toast_worthy("exec-no-ext", &claude));
        }

        assert!(drift_toast_worthy("some-dir", &claude));
        assert!(drift_toast_worthy("script.sh", &claude));
        assert!(!drift_toast_worthy("state-file", &claude));
        assert!(!drift_toast_worthy("does-not-exist", &claude));
    }

    /// `cfg(linux)`: testa `unclassified_claude_children`, linux-only —
    /// mesmo motivo do teste anterior.
    #[test]
    #[cfg(target_os = "linux")]
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
