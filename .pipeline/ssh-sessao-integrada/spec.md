# Frozen spec — ssh-sessao-integrada

> Frozen at the end of `fase-arq`. Everything downstream is measured against this
> file: `impl-block` builds from it, `verifier` checks the diff against it. If it
> has to change, the change goes through `fase-arq` again — a spec that moves
> measures nothing.

Vault note: `[[tyba/features/ssh/tech-spec-05-sessao-integrada]]` (promise, rules, design)
ADR: `[[tyba/decisions/2026-09-17-ssh-sessao-integrada]]`
Depends on the delivery `ssh-conexao-confiavel` (PR #310): the login marker with
a per-spawn nonce is the channel this delivery extends.

Vocabulary (vault glossary): **Host**, **SSH Session** (lives on the host),
**Cano** (the local `ssh` process), **login concluded**. New here:
**integrated session** (an SSH Session whose remote shell runs TYBA's shell
integration), **control transport** (the tmux control-mode client inside the
core), **side channel** (a short command run over the existing multiplexed
connection, never through the PTY).

Code identifiers and comments follow the repo: identifiers in English, comments
in pt-BR for traps only. UI strings are pt-BR through i18n.

## Promise

An SSH Session behaves like TYBA's local shell — blocks per command, the TYBA
command line, history, completion and chips — all from the **server**, while
keeping the persistence the managed tmux already provides.

## Rules

### Transport

1. An integrated session's remote command is
   `tmux -C new-session -A -s <name> "<login shell with TYBA's rc>"`. The login
   marker of `ssh-conexao-confiavel` is printed **before** it, unchanged, and a
   second marker (`tyba-ctl=<nonce>`, same channel and nonce) announces the
   switch to the control protocol. Everything before that marker is raw bytes
   (ssh banners, password prompts); everything after is the control protocol.
2. `PtyPool` gains a transport per session: `Raw` (today) and `TmuxControl`.
   Callers do not change: `write`, `resize` and the reader path keep their
   signatures, and every existing call site (command line, broadcast, rich input,
   snippets) works unchanged.
3. `TmuxControl` decodes `%output <pane> <octal-escaped bytes>` into the byte
   stream that feeds the screen, the capture machine and the `OscParser` — those
   three are untouched. Other notifications (`%begin/%end/%error`, `%exit`,
   `%session-changed`, `%window-*`, `%layout-change`, `%client-detached`) are
   consumed by the transport, never forwarded to the screen.
4. Input is sent as `send-keys -H <hex bytes>` to the session's pane; a resize is
   `refresh-client -C <cols>x<rows>`. No keystroke is ever sent to tmux itself.
5. Reattaching an integrated session redraws from `capture-pane -p -e -J` (the
   visible screen plus at most 5000 scrollback lines, matching the tmux
   `history-limit` the wrap already sets). Measured on 2026-09-17: a control
   client that attaches receives no screen content on its own.
6. A command that the control client cannot parse (unknown notification, broken
   escape) is logged once per session and dropped; the session stays alive.
7. `%exit` and the death of the control client are the same drop the
   `CanoLifecycle` already handles — the reconnection rules of
   `ssh-conexao-confiavel` are unchanged.

### Remote shell integration

8. Supported remote shells: **bash** and **zsh** — the same the local
   integration supports, driven by the same rc scripts plus a remote prologue.
   Any other shell (fish, sh, ksh, BusyBox) opens as a plain terminal with a
   one-line explanation in the pane, and so does a host whose integration switch
   is off or that could not be detected.
9. The rc reaches the server inside the ssh command: it is decoded into a private
   temp directory (0700, under `${XDG_RUNTIME_DIR:-/tmp}`), read by the shell at
   startup, and **the directory is removed by the rc itself right after being
   sourced**. After the session is up, no TYBA file remains on the server.
10. The remote rc emits the same sequences the local one does (`133;A/B/C/D`,
    `633;E`, `633;P` for prompt mode, `OSC 7` for cwd), and nothing it emits ever
    grounds a security decision.
11. Every Host has `integration_enabled`, default true, editable in the host
    form.
12. An SSH Session that was already alive before this version keeps running as a
    plain terminal until it is closed; every new session is integrated. The pane
    says so in one line.
13. A host without tmux opens integrated without persistence, and the pane says
    so (the no-tmux fallback of the wrap keeps working).

### What the pane gains

14. Blocks per command, with exit code, under the same rule as local: blocks only
    exist with prompt mode on. They are persisted through the same path, with the
    same secret redaction.
15. The TYBA command line, with the same three keyboard states as local
    (alt-screen, running command, shell at prompt) — the `kind === "ssh"` gate
    that hides it today goes away for integrated sessions.
16. `SessionStatus` (running · idle · awaiting input) is driven by the remote
    markers, exactly as local.
17. Broadcast keeps working: it writes through the transport like any other
    writer.

### History

18. A command typed on the server is recorded with its `host_id`, after the
    existing redaction.
19. In a session of that Host, its history ranks first; in a local session, a
    command that only exists in a remote history is not suggested.

### Completion

20. The remote command names are read once per connection through the **side
    channel** — `files::remote::RemoteFs::exec`, which already runs commands over
    the multiplexed connection; no new ssh path is created — never through the
    PTY, cached per Host for the life of the connection, capped at 8000 names and
    512 KiB.
21. Remote path completion uses the existing SFTP layer (`files/remote`).
22. The Host's history is a source, per rule 19.

### Chips

23. In an SSH Session the chips show the **server's** cwd, branch and diff count,
    and never the local machine's.
24. The cwd comes from the remote `OSC 7`; branch and diff count come from the
    side channel (`RemoteFs::exec`, reusing the `-z --no-color
    -c core.quotePath=false` discipline of `files/remote/gitexec.rs`), at most
    once per 2 s per session and never more than one in-flight query per session.
25. Where the side channel is unavailable (no ControlMaster — Windows today), the
    remote chips and the remote command names degrade to absent; cwd and path
    completion still work.

### Agent on the server

26. When a remote command matches the agent matcher, the pane shows a banner
    saying the agent runs on the server without jail and without the approvals
    inbox. Nothing is installed, intercepted or blocked on the server.

### Boundaries kept

27. Hand-typed `ssh` in a local shell is not integrated; the current behaviour
    (host context, plain terminal) is unchanged.
28. The tmux stays invisible and is only ever driven by commands.

## Contract

### Rust

```rust
// pty/mod.rs
pub enum Transport { Raw, TmuxControl { pane: String, session: String, control_marker: Option<String> } }
impl PtyPool {
    pub fn spawn_with_transport(/* … existing args …, transport: Transport */) -> Result<(), PtyError>;
    pub fn redraw_from_capture(&self, id: PtyId) -> Result<(), PtyError>; // reattach
}

// Emenda de contrato — 2026-09-17, durante o bloco 01: `control_marker` carrega o
// marco `tyba-ctl=<nonce>` da regra 1. Sem ele na camada de PTY não há como
// separar a fase crua (banner, senha, marco de login) do protocolo, e o §7 do
// desenho já havia descartado a heurística "linha começa com %". `None` = o
// protocolo vale desde o primeiro byte.

// pty/tmux_control.rs (new, pure)
pub struct ControlDecoder { /* … */ }
pub enum ControlEvent { Output(Vec<u8>), Exit, Notification(String), Unparsed(String) }
impl ControlDecoder { pub fn feed(&mut self, bytes: &[u8]) -> Vec<ControlEvent>; }
pub fn send_keys(pane: &str, bytes: &[u8]) -> String;      // `send-keys -H …`
pub fn refresh_client(cols: u16, rows: u16) -> String;
pub fn capture_pane(pane: &str, lines: u32) -> String;

// ssh/remote_rc.rs (new)
pub enum RemoteShell { Bash, Zsh, Unsupported(String) }
pub fn remote_command(shell: RemoteShell, nonce: &str, tmux_name: &str, integrate: bool) -> String;

// ssh/query.rs (new — side channel)
pub struct HostQuery { /* alias, throttle, cache */ }
impl HostQuery {
    pub fn command_names(&self) -> Result<Vec<String>, AppError>;
    pub fn git_chips(&self, cwd: &str) -> Result<GitChips, AppError>;
}

// ssh/mod.rs — Host gains:
#[serde(default = "default_true")] pub integration_enabled: bool,
```

### IPC

| Surface | Change |
|---|---|
| `create_host` / `update_host` | accept `integration_enabled` |
| `session://status` | unchanged; integrated sessions now report real `SessionStatus` |
| `session://command`, `session://cwd`, `session://prompt-mode` | now also fire for SSH sessions |
| chips | a session-scoped chip payload carries remote values; the front stops reading local repo state for `kind === "ssh"` |
| new event `ssh://integration/<session>` | `{ state: "integrated" \| "plain", reason, persistence }` — what the pane's one-line explanation renders. **Emenda de contrato — 2026-09-17, durante o bloco 02:** `persistence` (`"persistent" \| "ephemeral" \| "unknown"`) entrou porque a regra 13 exige que o pane diga "integrada, sem persistência" quando o host não tem tmux, e o evento era o único caminho; `unknown` quando não deu para perguntar — o pane não afirma o que o core não sabe. |

### Schema — migration step 9

- `host.integration_enabled INTEGER NOT NULL DEFAULT 1`
- `command_history.host_id TEXT NULL` (+ index by `host_id`)
- Check that no other branch claimed step 9 before writing it.

## Invariants that apply

- **Declared state transitions** — the Cano machine is untouched; the transport
  adds no state of its own beyond "raw → control" per spawn.
- **No external call inside a DB transaction** — side-channel queries and
  `capture-pane` never run with a transaction open.
- **Injected clock** — throttling of the side channel is testable without sleeping.
- **PTY batching (CLAUDE.md #3)** — the control transport keeps the ~16 ms batch
  to the webview: decoding happens in the reader thread, never one IPC event per
  `%output`.
- **Bounded reads** — `%output` chunks are bounded by the existing PTY buffers;
  `capture-pane` at most 5000 lines; command names capped per rule 20;
  side-channel output capped at 512 KiB.
- **Secrets** — redaction runs before blocks and history are written, as today.
- **OSC grounds no security decision** — rule 10.

## Acceptance criteria

- [ ] The decoder turns `%output` (including octal escapes and a payload split across reads) into exactly the original bytes; notifications never reach the screen.
- [ ] `send-keys`/`refresh-client`/`capture-pane` command strings are exactly as specified (unit, argv).
- [ ] An unparsable line is dropped and logged once; the session survives.
- [ ] `PtyPool::write`/`resize` keep their signatures: every existing call site compiles untouched.
- [ ] The remote command is correct per shell (bash, zsh) and for the unsupported case, and the rc removes its temp directory after sourcing (unit on the generated script + real-host test).
- [ ] A new session on a bash host shows blocks with exit codes (real host).
- [ ] The same on a zsh host, or the test says the host has no zsh.
- [ ] The TYBA command line appears for an integrated session and respects the three keyboard states, including alt-screen apps.
- [ ] Reattaching redraws from `capture-pane` without re-running anything (real host).
- [ ] After the session is up, no TYBA file remains on the server (real host check).
- [ ] A host with the switch off, or with an unsupported shell, opens plain with the one-line explanation, and the event carries the reason.
- [ ] A session created before the upgrade keeps working and reports `plain` with the "from before" reason.
- [ ] Remote commands are recorded with `host_id`; a local session does not suggest a remote-only command (unit + store).
- [ ] Remote command names come from the side channel, capped, cached per connection, and never travel through the PTY (unit on the query + a guard that the PTY write path is not used for it).
- [ ] Remote path completion returns entries from the server (real host).
- [ ] Chips show the server's cwd, branch and diff count; local values never appear in an SSH session (unit + real host).
- [ ] The side channel runs at most one query per session at a time and no more than one per 2 s (unit, injected clock).
- [ ] Without a ControlMaster the remote chips and names are absent, and nothing blocks or times out the pane (unit).
- [ ] An agent started on the server raises the banner and nothing is blocked.
- [ ] Broadcast reaches integrated sessions through the transport (unit).
- [ ] A secret typed in an integrated session appears neither in history nor in the stored block.
- [ ] The owner's own tmux nested inside an integrated session keeps working (real host).
- [ ] Migration 8 → 9 adds both columns and preserves existing rows.
- [ ] E2E on the owner's VPS (bash): blocks, command line, chips, reattach, and a network drop.

## Out of scope

- Hand-typed `ssh` in a local shell (wrapping the user's `ssh` command).
- Agent on the remote with jail, gate or inbox.
- fish and other remote shells.
- Re-opening a live session as integrated.
- Host health (latency, availability on the card).
- Redacting the scrollback that lives in the server's tmux.
