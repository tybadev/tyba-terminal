//! O handler do core para o canal shim↔core (shim v2, passo 2, tech-spec
//! §4): quem decide se um `host_agent` vira sessão hospedada, e em qual
//! sessão de shell e cwd. Agente livre (2026-10-15): o claude digitado sobe
//! sem jaula; a allowlist de jaula (§1 / ADR 2026-09-02) saiu deste caminho,
//! e só [`visible_strict_descendant`] continua, para o `cd` do retomar.

use std::path::{Component, Path, PathBuf};

use crate::agent::process_probe::{find_owning_session, ProcRow};
use crate::hook_ipc::channel::{ChannelRequest, ChannelResponse, RefusedReason};
use crate::session::SessionId;

const HOST_AGENT_AGENT: &str = "claude";

/// O que a resolução decidiu, antes de `prepare_hosted_agent` (Track B, em
/// `agent::session`) montar o `argv`/`env` de verdade: qual sessão hospeda e
/// em que cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHost {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    /// Os args do `claude`, já reclassificados aqui como sessão hospedável.
    pub args: Vec<String>,
}

/// As dependências da resolução, injetadas — o mesmo motivo de sempre:
/// `handle` (produção) fecha sobre `/proc` de verdade; o teste fecha sobre
/// dados sintéticos, sem precisar de processo nenhum.
pub struct ResolveDeps<'a> {
    pub shells: &'a [(SessionId, u32)],
    pub rows: &'a [ProcRow],
    pub starttime: &'a dyn Fn(u32) -> Option<u64>,
    pub cwd_of: &'a dyn Fn(u32) -> Option<PathBuf>,
}

/// Review round 1 (correção/contrato), achado MINOR: `/proc/<pid>/cwd` de um
/// diretório APAGADO ainda faz `readlink` suceder — o kernel devolve o path
/// original com o sufixo `" (deleted)"`, então `Option::Some` sozinho nunca
/// distingue "cwd viva" de "cwd sumiu debaixo do processo" (§4.4: cwd
/// inexistente/(deleted) → `refused:no_cwd`). Sem este check, um cwd morto
/// passava batido e chegava a `prepare_hosted_agent`, que BINDARIA um
/// `HookServer` órfão — a pré-condição do achado bloqueante de deadlock
/// corrigido acima. `is_dir()` cobre o segundo caso do achado (path que
/// simplesmente não existe mais no disco, sem o sufixo do kernel — ex.: um
/// bind-mount desmontado).
fn cwd_is_live(cwd: &Path) -> bool {
    !cwd.as_os_str().to_string_lossy().ends_with(" (deleted)") && cwd.is_dir()
}

#[cfg(test)]
mod cwd_is_live_tests {
    use super::*;

    #[test]
    fn a_path_ending_in_the_kernel_deleted_suffix_is_not_live() {
        assert!(!cwd_is_live(Path::new(
            "/home/dono/projetos/tyba (deleted)"
        )));
    }

    #[test]
    fn a_path_that_does_not_exist_on_disk_is_not_live() {
        assert!(!cwd_is_live(Path::new(
            "/definitely/does/not/exist/on/this/machine"
        )));
    }

    #[test]
    fn a_real_directory_on_disk_is_live() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cwd_is_live(dir.path()));
    }

    /// Prova viva (não só a checagem de string): apaga um diretório
    /// DEBAIXO de um processo vivo — no Linux isso é permitido, o kernel
    /// mantém a referência — e confere que `/proc/<pid>/cwd` de verdade
    /// devolve o path com o sufixo `" (deleted)"`, e que `cwd_is_live`
    /// rejeita esse valor real, não só um literal montado à mão.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_cwd_deleted_out_from_under_a_live_process_is_rejected_for_real() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .current_dir(dir.path())
            .spawn()
            .expect("spawn sleep com cwd de teste");
        let pid = child.id();

        std::fs::remove_dir(dir.path()).expect("apaga o cwd debaixo do processo vivo");

        let real_cwd = crate::repo::process_cwd(pid);
        let _ = child.kill();
        let _ = child.wait();

        let cwd = real_cwd.expect("/proc/<pid>/cwd ainda resolve, com o sufixo do kernel");
        assert!(
            cwd.to_string_lossy().ends_with(" (deleted)"),
            "esperava o sufixo do kernel para cwd apagada: {cwd:?}"
        );
        assert!(
            !cwd_is_live(&cwd),
            "uma cwd apagada nunca pode contar como viva: {cwd:?}"
        );
    }
}

/// O núcleo puro do handler (tech-spec §4, passos 2 a 7 — o passo 1, peer
/// cred via `SO_PEERCRED`, já aconteceu em `hook_ipc::channel::dispatch`
/// antes de `peer_pid` chegar aqui). Ordem que importa:
///
/// 1. agente conhecido — recusa cedo, antes de qualquer leitura de `/proc`.
/// 2. starttime do peer ANTES do trabalho pesado (TOCTOU, FIX C1).
/// 3. sessão dona (`find_owning_session`) — sobe a árvore até um líder de
///    shell conhecido.
/// 4. cwd do líder — nunca da requisição.
/// 5. RECHECK do starttime: se o pid foi reciclado durante os passos 3-4,
///    diverge daqui e a resposta é `peer_unresolved` — pior caso de corrida
///    perdida é hospedar na cwd da PRÓPRIA sessão do atacante, sem escalação.
pub fn resolve_host_request(
    peer_pid: u32,
    request: &ChannelRequest,
    deps: &ResolveDeps,
) -> Result<ResolvedHost, RefusedReason> {
    if request.agent != HOST_AGENT_AGENT {
        return Err(RefusedReason::UnknownAgent);
    }
    // M6-A: o core refaz a classificação do cliente sobre os args que
    // chegaram. Só sessão de agente é hospedada; flag que tiraria os hooks do
    // TYBA (ou qualquer coisa que o cliente não devia ter mandado) é recusada.
    if crate::agent::claude_cli::route(&request.args) != crate::agent::claude_cli::ClaudeRoute::Host
    {
        return Err(RefusedReason::ForbiddenArgs);
    }
    let starttime_before = (deps.starttime)(peer_pid).ok_or(RefusedReason::PeerUnresolved)?;

    let session_id =
        find_owning_session(peer_pid, deps.shells, deps.rows).ok_or(RefusedReason::NoSession)?;
    let leader_pid = deps
        .shells
        .iter()
        .find(|(id, _)| *id == session_id)
        .map(|&(_, pid)| pid)
        .ok_or(RefusedReason::NoSession)?;
    let cwd = (deps.cwd_of)(leader_pid).ok_or(RefusedReason::NoCwd)?;
    if !cwd_is_live(&cwd) {
        return Err(RefusedReason::NoCwd);
    }

    if (deps.starttime)(peer_pid) != Some(starttime_before) {
        return Err(RefusedReason::PeerUnresolved);
    }

    Ok(ResolvedHost {
        session_id,
        cwd,
        args: request.args.clone(),
    })
}

/// O ponto de entrada de produção — o `ChannelHandler` que `lib.rs` liga ao
/// `ChannelServer::bind`. Fecha as dependências reais (`/proc`) e delega a
/// `resolve_host_request`; em caso de acerto,
/// chama `agent::session::prepare_hosted_agent` (Track B) para montar o
/// `argv`/`env`/`hooks.json`/`HookServer` de verdade e escrever o plano.
pub fn handle(
    ctx: &super::session::AgentSessionCtx,
    peer_pid: u32,
    request: ChannelRequest,
) -> ChannelResponse {
    let shells: Vec<(SessionId, u32)> = ctx
        .sessions
        .shell_ids()
        .into_iter()
        .filter_map(|id| ctx.pty_pool.leader_pid(id).map(|pid| (id, pid)))
        .collect();
    let rows = crate::agent::process_probe::snapshot();
    let deps = ResolveDeps {
        shells: &shells,
        rows: &rows,
        starttime: &crate::repo::process_start_time,
        cwd_of: &crate::repo::process_cwd,
    };
    match resolve_host_request(peer_pid, &request, &deps) {
        Err(reason) => ChannelResponse::Refused { reason },
        Ok(resolved) => {
            match super::session::prepare_hosted_agent(
                ctx,
                resolved.session_id,
                resolved.cwd,
                &resolved.args,
            ) {
                // Agente livre: nunca jaulado.
                Ok(plan_path) => ChannelResponse::Host {
                    plan_path: plan_path.to_string_lossy().into_owned(),
                    jailed: false,
                },
                // Falhou preparar (binário ausente, fs quebrado, git indisponível
                // no repo alvo): degrada para a MESMA resposta de "não posso
                // ajudar agora" que o cliente já trata — sem 5º motivo de recusa
                // só para isto (mesmo raciocínio do `dispatch` sobrecarregado em
                // `hook_ipc::channel`).
                Err(_) => ChannelResponse::Refused {
                    reason: RefusedReason::NoSession,
                },
            }
        }
    }
}

/// O componente que faz `T` deixar de ser um descendente ESTRITO e VISÍVEL de
/// `home`: `T == home` (não é descendente, é a própria home) OU algum
/// componente do caminho relativo começa com `.` (dot-dir/dot-file no
/// caminho). Pura — nenhuma chamada a disco, opera só sobre os dois `Path`
/// já canonicalizados por quem chama.
pub(crate) fn visible_strict_descendant(candidate: &Path, home: &Path) -> bool {
    let Ok(rel) = candidate.strip_prefix(home) else {
        return false;
    };
    let mut components = rel.components().peekable();
    if components.peek().is_none() {
        // T == home: a própria home não é um descendente dela mesma.
        return false;
    }
    components.all(|c| match c {
        Component::Normal(part) => !part.to_string_lossy().starts_with('.'),
        // Um caminho já canonicalizado (`repo::canonicalize_or`) nunca traz
        // `..`/`.`; `RootDir`/`Prefix` não aparecem depois de um
        // `strip_prefix` bem sucedido. Rejeita mesmo assim — defesa em
        // profundidade sobre uma entrada que não deveria existir.
        _ => false,
    })
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn req() -> ChannelRequest {
        ChannelRequest {
            v: crate::hook_ipc::channel::CHANNEL_PROTOCOL_VERSION,
            op: "host_agent".into(),
            agent: "claude".into(),
            args: Vec::new(),
            token: None,
            event: None,
            tool: None,
            summary: None,
        }
    }

    fn row(pid: u32, ppid: u32) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            comm: "x".into(),
            start_ms: 0,
        }
    }

    /// Deps de um caso feliz de fábrica: peer(200) filho do shell líder(100)
    /// da sessão `s`, starttime estável, cwd viva no disco.
    ///
    /// Review round 1, achado MINOR: `home`/`cwd` moram num `tempfile::
    /// TempDir` de verdade, criado no disco — desde o fix de `cwd_is_live`
    /// (que faz `is_dir()`), um caminho sintético que nunca existiu (o que a
    /// fixture usava antes) reprova a checagem de "cwd viva" e faz TODO
    /// teste que espera `Ok(...)` quebrar por um motivo que não é o dele. O
    /// `TempDir` guard fica dentro da struct só para não ser apagado cedo
    /// demais — o teste nunca lê o campo diretamente.
    #[allow(clippy::type_complexity)]
    struct Fixture {
        _home_dir: tempfile::TempDir,
        session: SessionId,
        shells: Vec<(SessionId, u32)>,
        rows: Vec<ProcRow>,
        cwd: PathBuf,
        cwd_of: Box<dyn Fn(u32) -> Option<PathBuf>>,
    }

    fn fixture() -> Fixture {
        let session = SessionId::new_v4();
        let home_dir = tempfile::tempdir().expect("tempdir da home de teste");
        let home = crate::repo::canonicalize_or(home_dir.path());
        let cwd = home.join("projetos").join("tyba");
        std::fs::create_dir_all(&cwd).expect("cria o repo de teste no disco");
        let cwd_for_leader = cwd.clone();
        Fixture {
            _home_dir: home_dir,
            session,
            shells: vec![(session, 100)],
            rows: vec![row(100, 1), row(200, 100)],
            cwd,
            cwd_of: Box::new(move |_leader| Some(cwd_for_leader.clone())),
        }
    }

    fn stable_starttime(_pid: u32) -> Option<u64> {
        Some(999)
    }

    fn deps_from(f: &Fixture) -> ResolveDeps<'_> {
        ResolveDeps {
            shells: &f.shells,
            rows: &f.rows,
            starttime: &stable_starttime,
            cwd_of: &*f.cwd_of,
        }
    }

    #[test]
    fn an_unknown_agent_is_refused_before_touching_proc() {
        let f = fixture();
        let deps = deps_from(&f);
        let mut request = req();
        request.agent = "vim".into();
        assert_eq!(
            resolve_host_request(200, &request, &deps),
            Err(RefusedReason::UnknownAgent)
        );
    }

    #[test]
    fn a_peer_unrelated_to_any_shell_is_no_session() {
        let f = fixture();
        let deps = deps_from(&f);
        // 777 não é filho de líder nenhum conhecido.
        assert_eq!(
            resolve_host_request(777, &req(), &deps),
            Err(RefusedReason::NoSession)
        );
    }

    #[test]
    fn a_leader_without_a_resolvable_cwd_is_no_cwd() {
        let f = fixture();
        let mut deps = deps_from(&f);
        let none_cwd = |_leader: u32| None;
        deps.cwd_of = &none_cwd;
        assert_eq!(
            resolve_host_request(200, &req(), &deps),
            Err(RefusedReason::NoCwd),
            "cwd sumiu ((deleted)) ou /proc/<leader>/cwd ilegível"
        );
    }

    /// Review round 1, achado MINOR: `process_cwd` de uma cwd apagada NUNCA
    /// devolve `None` (o `readlink` do kernel sucede, só que com o sufixo
    /// `" (deleted)"` no valor) — então o caminho de `a_leader_without_a_
    /// resolvable_cwd_is_no_cwd` acima (mock devolvendo `None`) não cobre o
    /// caso real. Este cobre o valor exato que `/proc/<pid>/cwd` produz de
    /// verdade para um diretório apagado debaixo do processo.
    #[test]
    fn a_deleted_cwd_from_the_kernel_suffix_is_no_cwd_not_a_dangling_bind() {
        let f = fixture();
        let mut deps = deps_from(&f);
        let deleted_path = PathBuf::from(format!("{} (deleted)", f.cwd.display()));
        let deleted_cwd = move |_leader: u32| Some(deleted_path.clone());
        deps.cwd_of = &deleted_cwd;
        assert_eq!(
            resolve_host_request(200, &req(), &deps),
            Err(RefusedReason::NoCwd),
            "cwd com o sufixo do kernel para diretório apagado precisa virar \
             NoCwd — passar batido bindaria um HookServer órfão numa pasta \
             que não existe mais"
        );
    }

    #[test]
    fn a_peer_that_vanishes_before_starttime_can_be_read_is_peer_unresolved() {
        let f = fixture();
        let mut deps = deps_from(&f);
        let no_starttime = |_pid: u32| None;
        deps.starttime = &no_starttime;
        assert_eq!(
            resolve_host_request(200, &req(), &deps),
            Err(RefusedReason::PeerUnresolved)
        );
    }

    #[test]
    fn a_recycled_peer_pid_fails_the_toctou_recheck() {
        let f = fixture();
        let mut deps = deps_from(&f);
        // Primeira leitura de starttime (antes do trabalho) bate; a segunda
        // (depois de resolver sessão+cwd) diverge — pid foi reciclado no
        // meio da corrida.
        let call = std::sync::atomic::AtomicU32::new(0);
        let flaky = move |_pid: u32| {
            let n = call.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(if n == 0 { 999 } else { 1000 })
        };
        deps.starttime = &flaky;
        assert_eq!(
            resolve_host_request(200, &req(), &deps),
            Err(RefusedReason::PeerUnresolved),
            "starttime mudou entre o registro e o recheck — pior caso é recusar, nunca gate na sessão errada"
        );
    }

    #[test]
    fn a_live_cwd_resolves_to_a_host_request_in_the_owning_shell() {
        let f = fixture();
        let deps = deps_from(&f);
        let resolved = resolve_host_request(200, &req(), &deps).expect("deveria resolver");
        assert_eq!(resolved.session_id, f.session);
        assert_eq!(resolved.cwd, f.cwd);
    }

    /// M6-A: o core refaz a classificação dos args — nunca confia no `_jail`.
    #[test]
    fn args_that_would_drop_tybas_hooks_are_refused_by_the_core() {
        let f = fixture();
        let deps = deps_from(&f);
        for args in [
            vec!["--bare"],
            vec!["--settings", "/tmp/sem-hooks.json"],
            vec!["--resume", "não é um id"],
            // Passthrough nunca devia chegar ao canal: também recusa.
            vec!["--version"],
        ] {
            let mut request = req();
            request.args = args.iter().map(|a| a.to_string()).collect();
            assert_eq!(
                resolve_host_request(200, &request, &deps),
                Err(RefusedReason::ForbiddenArgs),
                "{args:?}"
            );
        }
    }

    /// Agente livre: pular a permissão do próprio Claude é de quem digita —
    /// o core hospeda (com os hooks e o guarda-corpo) em vez de recusar.
    #[test]
    fn skipping_claudes_own_permissions_is_hosted_by_the_core() {
        let f = fixture();
        let deps = deps_from(&f);
        for args in [
            vec!["--dangerously-skip-permissions"],
            vec!["--permission-mode", "bypassPermissions"],
        ] {
            let mut request = req();
            request.args = args.iter().map(|a| a.to_string()).collect();
            let resolved = resolve_host_request(200, &request, &deps).expect("hospeda");
            assert_eq!(resolved.args, request.args);
        }
    }

    #[test]
    fn session_args_ride_along_untouched() {
        let f = fixture();
        let deps = deps_from(&f);
        let mut request = req();
        request.args = vec![
            "--resume".into(),
            "11111111-2222-3333-4444-555555555555".into(),
            "-p".into(),
            "a; rm -rf ~ $(x) `y`".into(),
        ];
        let resolved = resolve_host_request(200, &request, &deps).expect("hospeda");
        assert_eq!(resolved.args, request.args);
    }
}

#[cfg(test)]
mod visible_strict_descendant_tests {
    use super::*;

    #[test]
    fn only_a_strict_visible_descendant_of_home_counts() {
        let home = Path::new("/home/dono");
        for (dir, expected) in [
            ("/home/dono/projetos/tyba", true),
            ("/home/dono/scratch", true),
            ("/home/dono", false),
            ("/home/dono/.config/x", false),
            ("/home/dono/projetos/.oculto/repo", false),
            ("/var/tmp/repo", false),
        ] {
            assert_eq!(
                visible_strict_descendant(Path::new(dir), home),
                expected,
                "{dir}"
            );
        }
    }
}
