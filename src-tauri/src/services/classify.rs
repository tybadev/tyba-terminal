//! De onde vem cada processo (origem) e se o TYBA aceita encerrá-lo.
//!
//! Puro sobre fatos já colhidos — árvore de processos, donos, líderes de
//! sessão —, para testar sem processo de verdade. A recusa mora aqui e é o
//! core que a aplica: o botão desligado na UI é só reflexo.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::session::SessionId;

/// Teto de saltos subindo `ppid`: um ciclo no retrato (pid reciclado no meio
/// da varredura) nunca gira para sempre.
const MAX_HOPS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// Descende do shell de uma sessão do TYBA.
    Session { session_id: SessionId },
    /// O próprio TYBA, ou um filho dele fora de sessão (servidor de LSP...).
    Tyba,
    /// Docker Desktop / docker-proxy: a porta publicada de um container.
    Docker,
    /// Do usuário, fora do TYBA.
    Outside,
    /// Do sistema (root, ou binário do sistema operacional).
    System,
    /// De outro usuário.
    OtherUser,
}

/// O que se sabe do processo para classificá-lo.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub pid: u32,
    /// `None` = o `ps` não viu (sumiu, ou o retrato não tem).
    pub uid: Option<u32>,
    pub comm: &'a str,
    /// A linha de comando crua (só o primeiro token é olhado).
    pub args: &'a str,
}

/// O mundo em volta: árvore, sessões e quem é o TYBA.
#[derive(Debug, Clone, Default)]
pub struct World {
    pub parent: HashMap<u32, u32>,
    /// Líder (shell) de cada sessão viva.
    pub leaders: HashMap<u32, SessionId>,
    pub tyba_pid: u32,
    pub tyba_pgid: u32,
    pub my_uid: u32,
}

impl World {
    /// A sessão cujo shell é ancestral de `pid` (o próprio shell conta).
    pub fn session_of(&self, pid: u32) -> Option<SessionId> {
        self.ancestor_where(pid, |p| self.leaders.contains_key(&p))
            .and_then(|p| self.leaders.get(&p).copied())
    }

    /// `pid` descende do TYBA (ou é ele)?
    pub fn under_tyba(&self, pid: u32) -> bool {
        self.ancestor_where(pid, |p| p == self.tyba_pid).is_some()
    }

    fn ancestor_where(&self, pid: u32, hit: impl Fn(u32) -> bool) -> Option<u32> {
        let mut cur = pid;
        let mut seen = HashSet::new();
        for _ in 0..MAX_HOPS {
            if hit(cur) {
                return Some(cur);
            }
            if cur <= 1 || !seen.insert(cur) {
                return None;
            }
            cur = *self.parent.get(&cur)?;
        }
        None
    }
}

/// Nomes de processo do Docker que seguram porta publicada.
/// (`com.docker.*` do Docker Desktop entra pelo prefixo: o comm do macOS é
/// cortado em 15 caracteres, `com.docker.back`.)
const DOCKER_COMMS: &[&str] = &["vpnkit", "vpnkit-bridge", "docker-proxy", "rootlesskit"];

fn is_docker(facts: &Facts) -> bool {
    DOCKER_COMMS.contains(&facts.comm)
        || facts.comm.starts_with("com.docker.")
        || facts.args.contains("/Docker.app/")
        || first_token(facts.args).ends_with("/docker-proxy")
}

/// Binários do sistema operacional: nunca são "seus" para encerrar, mesmo
/// rodando com o seu usuário (o ControlCenter do macOS escuta na 5000/7000).
const SYSTEM_PREFIXES: &[&str] = &[
    "/System/",
    "/usr/libexec/",
    "/usr/sbin/",
    "/sbin/",
    "/Library/Apple/",
    "/lib/systemd/",
    "/usr/lib/systemd/",
];

fn first_token(args: &str) -> &str {
    args.split_whitespace().next().unwrap_or("")
}

fn is_system_binary(facts: &Facts) -> bool {
    let exe = first_token(facts.args);
    SYSTEM_PREFIXES.iter().any(|p| exe.starts_with(p))
}

pub fn classify(facts: &Facts, world: &World) -> Origin {
    if facts.pid == world.tyba_pid {
        return Origin::Tyba;
    }
    match facts.uid {
        Some(0) => return Origin::System,
        Some(uid) if uid != world.my_uid => return Origin::OtherUser,
        _ => {}
    }
    if let Some(session_id) = world.session_of(facts.pid) {
        return Origin::Session { session_id };
    }
    if world.under_tyba(facts.pid) {
        return Origin::Tyba;
    }
    if is_docker(facts) {
        return Origin::Docker;
    }
    if is_system_binary(facts) {
        return Origin::System;
    }
    Origin::Outside
}

/// Por que o TYBA não encerra este processo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Pid1,
    Tyba,
    System,
    OtherUser,
    /// O shell de uma sessão: só o que roda dentro dele.
    SessionShell,
    /// Porta publicada de container: para-se o container na tela do Docker.
    Docker,
    /// O dono não pôde ser conferido.
    Unknown,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Pid1 => "pid1",
            Refusal::Tyba => "tyba",
            Refusal::System => "system",
            Refusal::OtherUser => "other_user",
            Refusal::SessionShell => "session_shell",
            Refusal::Docker => "docker",
            Refusal::Unknown => "unknown",
        }
    }
}

/// `None` = pode encerrar (depois da confirmação na UI).
pub fn refusal(facts: &Facts, origin: &Origin, world: &World) -> Option<Refusal> {
    if facts.pid <= 1 {
        return Some(Refusal::Pid1);
    }
    if facts.pid == world.tyba_pid {
        return Some(Refusal::Tyba);
    }
    match facts.uid {
        None => return Some(Refusal::Unknown),
        Some(0) => return Some(Refusal::System),
        Some(uid) if uid != world.my_uid => return Some(Refusal::OtherUser),
        _ => {}
    }
    if world.leaders.contains_key(&facts.pid) {
        return Some(Refusal::SessionShell);
    }
    match origin {
        Origin::Tyba => Some(Refusal::Tyba),
        Origin::System => Some(Refusal::System),
        Origin::OtherUser => Some(Refusal::OtherUser),
        Origin::Docker => Some(Refusal::Docker),
        Origin::Session { .. } | Origin::Outside => None,
    }
}

/// Um membro do grupo do alvo, como o `ps` viu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub pid: u32,
    pub uid: u32,
}

/// Para onde vai o sinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalTarget {
    /// `killpg`: o grupo inteiro.
    Group { pgid: u32 },
    /// Só o processo: o grupo dele leva junto algo que não pode morrer.
    Process { pid: u32 },
}

/// O grupo só vai junto se nele não há o TYBA, um shell de sessão nem
/// processo de outro usuário. Senão, só o pid.
pub fn signal_target(pid: u32, pgid: u32, members: &[Member], world: &World) -> SignalTarget {
    let safe_group = pgid > 1
        && pgid != world.tyba_pgid
        && members.iter().all(|m| {
            m.uid == world.my_uid && m.pid != world.tyba_pid && !world.leaders.contains_key(&m.pid)
        });
    if safe_group {
        SignalTarget::Group { pgid }
    } else {
        SignalTarget::Process { pid }
    }
}

/// As portas de cada sessão, para o chip da sidebar: ordenadas, sem repetição.
pub fn ports_by_session(
    listeners: &[super::listen::Listener],
    world: &World,
) -> HashMap<SessionId, Vec<u16>> {
    let mut out: HashMap<SessionId, Vec<u16>> = HashMap::new();
    for l in listeners {
        if let Some(session) = world.session_of(l.pid) {
            out.entry(session).or_default().push(l.port);
        }
    }
    for ports in out.values_mut() {
        ports.sort_unstable();
        ports.dedup();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::listen::Listener;

    const ME: u32 = 501;
    const TYBA: u32 = 900;

    fn sid(n: u128) -> SessionId {
        uuid::Uuid::from_u128(n)
    }

    /// launchd(1) → tyba(900) → zsh(1000, sessão A) → bun(1010) → vite(1020)
    ///           → node(2000) fora do TYBA
    ///           tyba(900) → lsp(950)
    fn world() -> World {
        World {
            parent: HashMap::from([
                (TYBA, 1),
                (950, TYBA),
                (1000, TYBA),
                (1010, 1000),
                (1020, 1010),
                (2000, 1),
            ]),
            leaders: HashMap::from([(1000, sid(1))]),
            tyba_pid: TYBA,
            tyba_pgid: TYBA,
            my_uid: ME,
        }
    }

    fn facts<'a>(pid: u32, uid: Option<u32>, comm: &'a str, args: &'a str) -> Facts<'a> {
        Facts {
            pid,
            uid,
            comm,
            args,
        }
    }

    #[test]
    fn descendant_of_a_session_shell_belongs_to_that_session() {
        let w = world();
        let o = classify(&facts(1020, Some(ME), "node", "vite"), &w);
        assert_eq!(o, Origin::Session { session_id: sid(1) });
        // O próprio shell também é da sessão.
        let shell = classify(&facts(1000, Some(ME), "zsh", "-zsh"), &w);
        assert_eq!(shell, Origin::Session { session_id: sid(1) });
    }

    #[test]
    fn tyba_itself_and_its_non_session_children_are_tyba() {
        let w = world();
        assert_eq!(
            classify(&facts(TYBA, Some(ME), "tyba", ""), &w),
            Origin::Tyba
        );
        assert_eq!(
            classify(&facts(950, Some(ME), "rust-analyzer", ""), &w),
            Origin::Tyba
        );
    }

    #[test]
    fn root_and_other_users_are_never_mine() {
        let w = world();
        assert_eq!(
            classify(&facts(2000, Some(0), "sshd", "/usr/sbin/sshd"), &w),
            Origin::System
        );
        assert_eq!(
            classify(&facts(2000, Some(502), "node", "node x"), &w),
            Origin::OtherUser
        );
    }

    #[test]
    fn docker_and_system_binaries_are_recognized_outside_sessions() {
        let w = world();
        assert_eq!(
            classify(&facts(2000, Some(ME), "com.docker.back", ""), &w),
            Origin::Docker
        );
        assert_eq!(
            classify(
                &facts(
                    2000,
                    Some(ME),
                    "docker-proxy",
                    "/usr/bin/docker-proxy -proto tcp"
                ),
                &w
            ),
            Origin::Docker
        );
        assert_eq!(
            classify(
                &facts(
                    2000,
                    Some(ME),
                    "ControlCenter",
                    "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter"
                ),
                &w
            ),
            Origin::System
        );
        assert_eq!(
            classify(&facts(2000, Some(ME), "node", "node vite"), &w),
            Origin::Outside
        );
    }

    #[test]
    fn a_ppid_cycle_does_not_hang_and_falls_outside() {
        let mut w = world();
        w.parent.insert(3000, 3001);
        w.parent.insert(3001, 3000);
        assert_eq!(
            classify(&facts(3000, Some(ME), "x", "x"), &w),
            Origin::Outside
        );
    }

    #[test]
    fn refusal_rules() {
        let w = world();
        let check = |pid: u32, uid: Option<u32>, comm: &str, args: &str| {
            let f = facts(pid, uid, comm, args);
            refusal(&f, &classify(&f, &w), &w)
        };
        assert_eq!(
            check(1, Some(0), "launchd", "/sbin/launchd"),
            Some(Refusal::Pid1)
        );
        assert_eq!(check(TYBA, Some(ME), "tyba", ""), Some(Refusal::Tyba));
        assert_eq!(check(950, Some(ME), "lsp", ""), Some(Refusal::Tyba));
        assert_eq!(check(2000, Some(0), "x", ""), Some(Refusal::System));
        assert_eq!(check(2000, Some(502), "x", ""), Some(Refusal::OtherUser));
        assert_eq!(check(2000, None, "x", ""), Some(Refusal::Unknown));
        assert_eq!(
            check(1000, Some(ME), "zsh", "-zsh"),
            Some(Refusal::SessionShell)
        );
        assert_eq!(
            check(2000, Some(ME), "com.docker.backend", ""),
            Some(Refusal::Docker)
        );
        assert_eq!(
            check(2000, Some(ME), "ControlCenter", "/System/Library/x"),
            Some(Refusal::System)
        );
        // O que roda dentro do shell, e o que é do usuário fora do TYBA: pode.
        assert_eq!(check(1020, Some(ME), "node", "vite"), None);
        assert_eq!(check(2000, Some(ME), "node", "node x"), None);
    }

    #[test]
    fn group_goes_along_only_when_it_is_safe() {
        let w = world();
        let me = |pid| Member { pid, uid: ME };
        assert_eq!(
            signal_target(1020, 1010, &[me(1010), me(1020)], &w),
            SignalTarget::Group { pgid: 1010 }
        );
        // Shell sem job control: o grupo é o do shell.
        assert_eq!(
            signal_target(1020, 1000, &[me(1000), me(1020)], &w),
            SignalTarget::Process { pid: 1020 }
        );
        // Grupo do próprio TYBA.
        assert_eq!(
            signal_target(950, TYBA, &[me(950)], &w),
            SignalTarget::Process { pid: 950 }
        );
        // Alguém de outro usuário no grupo.
        assert_eq!(
            signal_target(1020, 1010, &[me(1020), Member { pid: 1030, uid: 0 }], &w),
            SignalTarget::Process { pid: 1020 }
        );
        assert_eq!(
            signal_target(1020, 1, &[me(1020)], &w),
            SignalTarget::Process { pid: 1020 }
        );
        assert_eq!(
            signal_target(1020, 0, &[me(1020)], &w),
            SignalTarget::Process { pid: 1020 }
        );
    }

    #[test]
    fn ports_are_grouped_by_owning_session() {
        let mut w = world();
        w.leaders.insert(4000, sid(2));
        w.parent.insert(4000, TYBA);
        w.parent.insert(4010, 4000);
        let ls = |pid, port| Listener {
            pid,
            port,
            address: "*".into(),
        };
        let by = ports_by_session(
            &[
                ls(1020, 5432),
                ls(1020, 3000),
                ls(1020, 3000),
                ls(4010, 8080),
                ls(2000, 9000),
            ],
            &w,
        );
        assert_eq!(by.get(&sid(1)), Some(&vec![3000, 5432]));
        assert_eq!(by.get(&sid(2)), Some(&vec![8080]));
        assert_eq!(by.len(), 2, "porta fora do TYBA não entra no chip");
    }
}
