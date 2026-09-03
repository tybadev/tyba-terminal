//! O handler do core para o canal shim↔core (shim v2, passo 2, tech-spec
//! §4): quem decide se um `host_agent` vira sessão gerenciada, e a allowlist
//! positiva de jaula (§1 / ADR 2026-09-02) que substitui o piso denylist que
//! o design-review furou em bwrap real.
//!
//! A allowlist é a decisão central de segurança desta entrega: `jail_target`
//! só devolve `Some` quando o toplevel git da cwd é um descendente ESTRITO e
//! VISÍVEL de `$HOME` — nunca a home em si, nunca sob um dot-dir, nunca fora
//! dela. Structurally disjunto de toda superfície de segredo do Linux
//! (dot-dirs, dot-files, fora da home), então nada precisa ser lembrado numa
//! lista de exclusão — é isso que corrige o furo medido (§XDG_RUNTIME_DIR,
//! ~/.config/~/.local/~/.cache, /var/tmp, shadow_swallows_worktree).

use std::path::{Component, Path, PathBuf};

use crate::agent::process_probe::{find_owning_session, ProcRow};
use crate::hook_ipc::channel::{ChannelRequest, ChannelResponse, RefusedReason};
use crate::session::SessionId;

const HOST_AGENT_AGENT: &str = "claude";

/// O que a resolução decidiu, antes de `prepare_hosted_agent` (Track B, em
/// `agent::session`) montar o `argv`/`env` de verdade: qual sessão hospeda,
/// em que cwd, e se a jaula entra ou não.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHost {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub jail_target: Option<PathBuf>,
}

/// As dependências da resolução, injetadas — o mesmo motivo de sempre:
/// `handle` (produção) fecha sobre `/proc` de verdade; o teste fecha sobre
/// dados sintéticos, sem precisar de processo nenhum.
pub struct ResolveDeps<'a> {
    pub shells: &'a [(SessionId, u32)],
    pub rows: &'a [ProcRow],
    pub starttime: &'a dyn Fn(u32) -> Option<u64>,
    pub cwd_of: &'a dyn Fn(u32) -> Option<PathBuf>,
    pub home: &'a Path,
    pub toplevel: &'a dyn Fn(&Path) -> Option<PathBuf>,
    pub userns_ok: bool,
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
///    perdida é gate na cwd da PRÓPRIA sessão do atacante, sem escalação.
/// 6. a allowlist decide a jaula.
pub fn resolve_host_request(
    peer_pid: u32,
    request: &ChannelRequest,
    deps: &ResolveDeps,
) -> Result<ResolvedHost, RefusedReason> {
    if request.agent != HOST_AGENT_AGENT {
        return Err(RefusedReason::UnknownAgent);
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

    let target = jail_target(&cwd, deps.home, deps.toplevel);
    let jail_target = decide_jail(target, deps.userns_ok);
    Ok(ResolvedHost {
        session_id,
        cwd,
        jail_target,
    })
}

/// O ponto de entrada de produção — o `ChannelHandler` que `lib.rs` liga ao
/// `ChannelServer::bind`. Fecha as dependências reais (`/proc`, git,
/// `userns_usable`) e delega a `resolve_host_request`; em caso de acerto,
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
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return ChannelResponse::Refused {
            reason: RefusedReason::NoCwd,
        };
    };
    let deps = ResolveDeps {
        shells: &shells,
        rows: &rows,
        starttime: &crate::repo::process_start_time,
        cwd_of: &crate::repo::process_cwd,
        home: &home,
        toplevel: &crate::repo::toplevel,
        userns_ok: userns_usable(),
    };
    match resolve_host_request(peer_pid, &request, &deps) {
        Err(reason) => ChannelResponse::Refused { reason },
        Ok(resolved) => {
            match super::session::prepare_hosted_agent(
                ctx,
                resolved.session_id,
                resolved.cwd,
                resolved.jail_target,
            ) {
                Ok((plan_path, jailed)) => ChannelResponse::Host {
                    plan_path: plan_path.to_string_lossy().into_owned(),
                    jailed,
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

#[cfg(target_os = "linux")]
fn userns_usable() -> bool {
    crate::sandbox::bwrap::userns_usable()
}

#[cfg(not(target_os = "linux"))]
fn userns_usable() -> bool {
    false
}

/// O componente que faz `T` deixar de ser um descendente ESTRITO e VISÍVEL de
/// `home`: `T == home` (não é descendente, é a própria home) OU algum
/// componente do caminho relativo começa com `.` (dot-dir/dot-file no
/// caminho). Pura — nenhuma chamada a disco, opera só sobre os dois `Path`
/// já canonicalizados por quem chama.
fn visible_strict_descendant(candidate: &Path, home: &Path) -> bool {
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

/// A allowlist positiva (§1): `Some(T)` só quando `T = toplevel(cwd)` é um
/// repositório git cujo toplevel é um descendente estrito e visível de
/// `home`. Todo o resto — cwd fora de repo, repo na própria home, repo sob um
/// dot-dir, repo fora da home — devolve `None`, e `None` aqui é
/// "gate-sem-jaula", nunca uma recusa (§3: não existe `no_repo` como motivo
/// de `refused`).
///
/// `toplevel` é injetado pelo mesmo motivo de `find_agent::kind_of`: a
/// resolução real faz `git rev-parse` (`crate::repo::toplevel`), e o teste
/// prova a decisão sem precisar de um repositório de verdade no disco.
pub fn jail_target(
    cwd: &Path,
    home: &Path,
    toplevel: impl Fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let target = crate::repo::canonicalize_or(&toplevel(cwd)?);
    let home = crate::repo::canonicalize_or(home);
    visible_strict_descendant(&target, &home).then_some(target)
}

/// O segundo eixo da decisão de jaula (§4.7): mesmo com um `jail_target`
/// positivo, sem user namespace utilizável a sessão degrada para
/// gate-sem-jaula — nunca recusa, e nunca tenta montar bwrap sem checar
/// antes (`platform_sandbox()` fail-closed é para o caminho de HOJE, não
/// para este). Pura sobre o resultado já calculado de `userns_usable()`, pelo
/// mesmo motivo dos outros: o teste não precisa de bwrap de verdade.
pub fn decide_jail(target: Option<PathBuf>, userns_ok: bool) -> Option<PathBuf> {
    target.filter(|_| userns_ok)
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn req() -> ChannelRequest {
        ChannelRequest {
            v: crate::hook_ipc::channel::CHANNEL_PROTOCOL_VERSION,
            op: "host_agent".into(),
            agent: "claude".into(),
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
    /// da sessão `s`, starttime estável, cwd sob um repo visível na home.
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
        home: PathBuf,
        cwd: PathBuf,
        cwd_of: Box<dyn Fn(u32) -> Option<PathBuf>>,
        toplevel: Box<dyn Fn(&Path) -> Option<PathBuf>>,
    }

    fn fixture() -> Fixture {
        let session = SessionId::new_v4();
        let home_dir = tempfile::tempdir().expect("tempdir da home de teste");
        let home = home_dir.path().to_path_buf();
        let cwd = home.join("projetos").join("tyba");
        std::fs::create_dir_all(&cwd).expect("cria o repo de teste no disco");
        let cwd_for_leader = cwd.clone();
        let cwd_for_toplevel = cwd.clone();
        Fixture {
            _home_dir: home_dir,
            session,
            shells: vec![(session, 100)],
            rows: vec![row(100, 1), row(200, 100)],
            home,
            cwd,
            cwd_of: Box::new(move |_leader| Some(cwd_for_leader.clone())),
            toplevel: Box::new(move |_cwd| Some(cwd_for_toplevel.clone())),
        }
    }

    fn stable_starttime(_pid: u32) -> Option<u64> {
        Some(999)
    }

    fn deps_from(f: &Fixture, userns_ok: bool) -> ResolveDeps<'_> {
        ResolveDeps {
            shells: &f.shells,
            rows: &f.rows,
            starttime: &stable_starttime,
            cwd_of: &*f.cwd_of,
            home: &f.home,
            toplevel: &*f.toplevel,
            userns_ok,
        }
    }

    #[test]
    fn an_unknown_agent_is_refused_before_touching_proc() {
        let f = fixture();
        let deps = deps_from(&f, true);
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
        let deps = deps_from(&f, true);
        // 777 não é filho de líder nenhum conhecido.
        assert_eq!(
            resolve_host_request(777, &req(), &deps),
            Err(RefusedReason::NoSession)
        );
    }

    #[test]
    fn a_leader_without_a_resolvable_cwd_is_no_cwd() {
        let f = fixture();
        let mut deps = deps_from(&f, true);
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
        let mut deps = deps_from(&f, true);
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
        let mut deps = deps_from(&f, true);
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
        let mut deps = deps_from(&f, true);
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
    fn a_visible_repo_resolves_to_a_jailed_host_request() {
        let f = fixture();
        let deps = deps_from(&f, true);
        let resolved = resolve_host_request(200, &req(), &deps).expect("deveria resolver");
        assert_eq!(resolved.session_id, f.session);
        assert_eq!(resolved.cwd, f.cwd);
        assert_eq!(resolved.jail_target, Some(f.cwd.clone()));
    }

    #[test]
    fn no_userns_resolves_to_a_gate_only_host_request_not_a_refusal() {
        let f = fixture();
        let deps = deps_from(&f, false);
        let resolved = resolve_host_request(200, &req(), &deps).expect("gate sem jaula não recusa");
        assert_eq!(resolved.jail_target, None);
    }

    #[test]
    fn a_non_git_cwd_resolves_to_a_gate_only_host_request_not_a_refusal() {
        let f = fixture();
        let mut deps = deps_from(&f, true);
        let no_repo = |_cwd: &Path| None;
        deps.toplevel = &no_repo;
        let resolved = resolve_host_request(200, &req(), &deps).expect("repo faltando degrada");
        assert_eq!(resolved.jail_target, None);
        assert_eq!(
            resolved.cwd, f.cwd,
            "cwd continua sendo a cwd real, mesmo sem jaula"
        );
    }
}

#[cfg(test)]
mod allowlist_tests {
    use super::*;

    fn git_repo_at(path: &str) -> impl Fn(&Path) -> Option<PathBuf> {
        let repo = PathBuf::from(path);
        move |_cwd: &Path| Some(repo.clone())
    }

    fn no_repo(_cwd: &Path) -> Option<PathBuf> {
        None
    }

    #[test]
    fn a_visible_repo_under_home_is_jailed() {
        let home = Path::new("/home/dono");
        let cwd = Path::new("/home/dono/projetos/tyba");
        let target = jail_target(cwd, home, git_repo_at("/home/dono/projetos/tyba"));
        assert_eq!(target, Some(PathBuf::from("/home/dono/projetos/tyba")));
    }

    #[test]
    fn a_non_git_cwd_degrades_to_gate_only() {
        let home = Path::new("/home/dono");
        let cwd = Path::new("/home/dono/scratch");
        assert_eq!(jail_target(cwd, home, no_repo), None);
    }

    #[test]
    fn a_repo_that_is_the_home_itself_degrades_to_gate_only() {
        let home = Path::new("/home/dono");
        let cwd = Path::new("/home/dono");
        assert_eq!(jail_target(cwd, home, git_repo_at("/home/dono")), None);
    }

    #[test]
    fn a_repo_under_a_dot_dir_degrades_to_gate_only() {
        let home = Path::new("/home/dono");
        let cwd = Path::new("/home/dono/.config/myrepo");
        let target = jail_target(cwd, home, git_repo_at("/home/dono/.config/myrepo"));
        assert_eq!(
            target, None,
            "~/.config carrega segredo — nunca vira raiz gravável"
        );
    }

    #[test]
    fn a_repo_outside_home_degrades_to_gate_only() {
        let home = Path::new("/home/dono");
        let cwd = Path::new("/var/tmp/repo");
        assert_eq!(jail_target(cwd, home, git_repo_at("/var/tmp/repo")), None);
    }

    #[test]
    fn a_repo_under_a_deep_dot_ancestor_degrades_to_gate_only() {
        // O dot-dir pode estar no MEIO do caminho, não só logo abaixo da home.
        let home = Path::new("/home/dono");
        let cwd = Path::new("/home/dono/projetos/.oculto/repo");
        let target = jail_target(cwd, home, git_repo_at("/home/dono/projetos/.oculto/repo"));
        assert_eq!(target, None);
    }

    #[test]
    fn no_userns_degrades_a_positive_target_to_gate_only() {
        let target = Some(PathBuf::from("/home/dono/projetos/tyba"));
        assert_eq!(decide_jail(target, false), None);
    }

    #[test]
    fn userns_usable_keeps_a_positive_target_jailed() {
        let target = Some(PathBuf::from("/home/dono/projetos/tyba"));
        assert_eq!(
            decide_jail(target.clone(), true),
            target,
            "userns disponível não muda a decisão da allowlist"
        );
    }
}
