# Frozen spec — ssh-conexao-confiavel

> Frozen at the end of `fase-arq`. Everything downstream is measured against this
> file: `impl-block` builds from it, `verifier` checks the diff against it. If it
> has to change, the change goes through `fase-arq` again — a spec that moves
> measures nothing.

Vault note: `[[tyba/features/ssh/tech-spec-04-conexao-confiavel]]` (promise and design)
ADR: `[[tyba/decisions/2026-09-16-ssh-conexao-confiavel]]`

Vocabulary (from the vault glossary): **Host** = the registered connection;
**SSH Session** = the live session that lives on the host (inside the managed
tmux); **Cano** = the local `ssh` process that reaches it; **login concluded** =
the moment the Cano's remote command starts running (sshd only runs it after
authentication); **connection failure** = the Cano ended before login concluded;
**drop** = the Cano ended after login concluded.

Code identifiers are English. Vault reason names map as: `auth_recusada` →
`auth_refused`, `host_key_mudou` → `host_key_changed`, `host_key_recusada` →
`host_key_rejected`, `host_nao_resolve` → `host_unresolved`, `sem_rota` →
`no_route`, `desconhecido` → `unknown`. UI strings are pt-BR through i18n.

## Promise

A registered Host connects from the connection manager in every case where typing
`ssh` by hand connects — and when it does not, TYBA says why instead of
reconnecting forever.

## Rules

### Host authentication

1. Every Host has an `auth_method`: `auto`, `agent`, `file` or `password`. Hosts
   stored before this delivery load as `auto`, and their `tyba.conf` block only
   changes by the keepalive lines (rule 9). An `auto` host that still carries a
   legacy `identity_file` keeps rendering `IdentityFile <path>` exactly as today.
2. `agent`: the Host stores an `agent_key` `{ public_key, name, fingerprint }`.
   The core recomputes `fingerprint` (SHA256, OpenSSH format) from `public_key`
   and rejects a key line that is not `<type> <base64>[ <comment>]` on one line.
   The rendered block has `IdentityFile <pub path>` + `IdentitiesOnly yes`, where
   the pub file lives at `~/.ssh/config.d/tyba-keys/<fp>.pub` (`<fp>` = the
   fingerprint without `SHA256:`, `+`→`-`, `/`→`_`, no `=`), file 0600, directory
   0700, content `<type> <base64> <name>\n`. Materialization writes every pub
   file a Host references and removes files in that directory no Host references.
3. `file`: `identity_file` is required; the block has `IdentityFile <path>`,
   `IdentitiesOnly yes`, `AddKeysToAgent yes`.
4. `password`: the block has `PubkeyAuthentication no` and
   `PreferredAuthentications keyboard-interactive,password`; `identity_file` and
   `agent_key` must be empty.
5. `auto`: no authentication option beyond the legacy `IdentityFile` of rule 1;
   `agent_key` must be empty. The form never produces `auto` with an
   `identity_file`: editing a legacy host presents it as `file`.
6. The core never stores, reads, transports or logs private key material,
   passwords or passphrases. It may spawn `ssh-keygen`/`ssh-add` on a key path;
   their stdout carrying key material is discarded, never parsed beyond
   fingerprints and public keys.
7. The rendered `tyba.conf` never contains `UseKeychain` or `IgnoreUnknown`.
8. A username containing an uppercase letter produces a form **warning**, never a
   block. `matchSshHost` keeps comparing usernames exactly.
9. Every Host block contains `ServerAliveInterval 15` and `ServerAliveCountMax 3`,
   before the multiplex lines. Order inside a block: `HostName`, `Port`, `User`,
   auth lines, `ProxyJump`, forwards, keepalive, multiplex.
10. Creating or updating a Host renders and validates (`ssh -G` on a staged file)
    the full host list **before** writing SQLite; an invalid render leaves neither
    the row nor `tyba.conf` changed. No database transaction is held while `ssh`
    runs. If installation fails after the row is written, the next mutation and
    the next boot regenerate the derived files (existing behaviour).

### The Cano lifecycle

11. Every Cano spawn gets a fresh nonce (32 lowercase hex chars). The remote
    command prints `ESC ] 633 ; P ; tyba-ssh-login=<nonce> BEL` before
    `exec tmux` (and before `exec $SHELL` in the no-tmux branch). A reader-thread
    watcher keeps the raw pre-login output (capped at 16 KiB, oldest bytes
    dropped) and detects the marker across read boundaries. A marker with any
    other nonce is ignored. After the marker, the watcher stops buffering.
12. Login concluded → connection `live`, `sessions.ssh_logged_in = 1`,
    `host.last_connected_at = now (UTC)`. Spawning never writes
    `last_connected_at`.
13. The Cano phase is a declared, pure state machine with an injected clock
    (`session/cano.rs`). Connection states: `connecting` (a Cano is spawned and
    has not logged in), `live`, `reconnecting` (waiting between attempts),
    `dropped` (gave up), `failed` (connection failure shown to the user).
    Allowed transitions: `connecting→live`, `connecting→failed`,
    `connecting→reconnecting` (only inside a drop, retryable reason),
    `connecting→dropped` (inside a drop, deadline passed), `live→reconnecting`,
    `live→(disposed)`, `reconnecting→connecting`, `reconnecting→dropped`,
    `reconnecting→(disposed)`, `failed→connecting` and `dropped→connecting`
    (user retry).
14. Exit before login outside a drop (first connect, user retry, boot resume of a
    session that never logged in is impossible — see 19) → `failed` with
    `{ reason, detail }`, no respawn, no `has-session` probe.
15. Reasons come from a pure classifier over the ANSI-stripped pre-login output:
    `auth_refused` (`Permission denied (` or `Too many authentication failures`),
    `host_key_changed` (`REMOTE HOST IDENTIFICATION HAS CHANGED`),
    `host_key_rejected` (`Host key verification failed` without the previous),
    `host_unresolved` (`Could not resolve hostname`),
    `no_route` (`Connection timed out`, `Operation timed out`,
    `Connection refused`, `No route to host`, `Network is unreachable`),
    `unknown` otherwise. `detail` is the last non-empty line of that output
    (≤ 300 chars). Precedence follows the list order.
16. Exit after login → drop: run the existing `has-session` arbiter. `Gone` or
    `NoTmux` → dispose exactly as today. `Alive` or `Unknown` → `reconnecting`,
    then respawn after 1, 2, 4, 8, 16, 30, 30… seconds; when 300 s have passed
    since the drop started → `dropped`. The drop clock starts at the first exit
    after login and is **not** reset by a later login unless that Cano stayed
    `live` for at least 30 s.
17. Exit before login inside a drop: `no_route` or `host_unresolved` → keep the
    backoff (rule 16 schedule and deadline); any other reason → `failed` with the
    reason, no further attempts.
18. A respawn under the same session id keeps the previous PTY's attachers and
    size; the pane keeps showing output (including password/passphrase prompts)
    and the PTY has the pane's current size.
19. Every connection-state change reaches the UI through the existing
    `session://status` event, carrying `connection` and `connection_failure`.
    On boot, an SSH Session with `ssh_logged_in = 0` is forgotten, not respawned;
    one with `ssh_logged_in = 1` is respawned as a drop that started at boot.
    `reconnect_ssh(id)` from `failed` or `dropped` performs one user-initiated
    attempt (rule 14 applies to it).

### Host key

20. TYBA never writes `known_hosts` and never renders `StrictHostKeyChecking`.
    A new host is confirmed at ssh's own prompt, in the pane.
21. `host_key_changed` renders a red explanation (attack or server reinstall) and
    a copyable `ssh-keygen -R <host>` command. No control removes or accepts a
    host key.

### Test connection

22. `test_host_connection` tests the **form values** without saving: a private
    temp directory (0700) holds a config with `Host tyba-test-<nonce>` rendered
    from the form (including a temp pub file for `agent`) followed by
    `Include <absolute ~/.ssh/config>`. ssh runs with `-F <temp config>`,
    `BatchMode=yes`, `ConnectTimeout=10`, `ConnectionAttempts=1`,
    `ControlMaster=no`, `ControlPath=none`, `UpdateHostKeys=no`, `-v`, and the
    remote command `true`. Total deadline 30 s (agent approval included); on
    deadline the process is killed. The temp directory is removed in every path.
    `tyba.conf` and `known_hosts` are never written.
23. Outcomes: `ok { user, elapsed_ms }`; `host_unknown { key_type, fingerprint }`
    (from `No … host key is known for` plus the `Server host key:` debug line);
    `passphrase_required`; `password_accepted { elapsed_ms }`;
    `password_not_offered { methods }`; `failed { reason, detail }`;
    `timed_out`. `user` is the form user or, when empty, the `user` that
    `ssh -G` resolves for the test alias.
24. `password` method: adds `PreferredAuthentications=none` and
    `PubkeyAuthentication=no`; the outcome comes from the
    `Permission denied (<methods>)` list — `password_accepted` when it contains
    `password` or `keyboard-interactive`, else `password_not_offered`. No key and
    no password is ever offered.
25. `agent`/`file` methods offer only the chosen key (`IdentitiesOnly yes`). A
    `file` test that fails with `auth_refused` reports `passphrase_required` when
    `ssh-keygen -y -P "" -f <path>` exits non-zero **and** the key's fingerprint
    (`ssh-keygen -l -f <path>`) is not in the agent listing.

### Environment of TYBA's ssh processes

26. Every `ssh` the core spawns — Cano, tmux probe/kill/ls, tunnels (`-O`),
    SFTP, `ssh -G`, test connection — and the `docker` process used with
    `DOCKER_HOST=ssh://` is built by `ssh::command` and receives `PATH` and
    `SSH_AUTH_SOCK` from the user's login shell, resolved once (same mechanism and
    3 s deadline as the agent PATH, one shell invocation for both). No other
    login-shell variable crosses. If resolution fails, the process env is used.
    `ssh-add -L` for agent listing uses the same builder. No other file may call
    `Command::new("ssh")` or `CommandBuilder::new("ssh")`.
27. For a `password` Host, SFTP, docker-over-ssh and session tunnels first run
    `ssh -O check <alias>`; if no master is alive they fail immediately with
    `ssh.password_needs_session`, without waiting for a timeout.

### Agent key listing

28. `list_agent_keys(alias?)` resolves the agent socket as the `identityagent`
    that `ssh -G` reports for the alias (or for a throwaway alias when none is
    given), `~` expanded; `none` or absent → the login-shell `SSH_AUTH_SOCK`.
    It runs `ssh-add -L` against that socket and returns
    `{ socket, keys: [{ public_key, name, fingerprint }] }`, at most 64 keys,
    reading at most 256 KiB. An agent with no identities returns an empty list,
    not an error. Listing never asks the agent to sign.

### Text fields

29. The base `Input` and `Textarea` components default to
    `autoCapitalize="off"`, `autoCorrect="off"`, `spellCheck={false}`; callers
    may override. Free-prose fields built on these components (the PR title in
    `OpenPrDialog`, host notes) opt back in explicitly. Fields built on raw
    `<input>`/`<textarea>` (agent composer, PR body, PR comments) keep the OS
    behaviour and are not touched.

## Contract

### Rust types (serde → IPC)

```rust
// ssh/mod.rs
#[serde(rename_all = "snake_case")]
pub enum AuthMethod { #[default] Auto, Agent, File, Password }

pub struct AgentKey { pub public_key: String, pub name: String, pub fingerprint: String }

// Host and HostInput gain:
#[serde(default)] pub auth_method: AuthMethod,
#[serde(default)] pub agent_key: Option<AgentKey>,

// ssh/classify.rs
#[serde(rename_all = "snake_case")]
pub enum FailureReason { AuthRefused, HostKeyChanged, HostKeyRejected, HostUnresolved, NoRoute, Unknown }
pub struct CanoFailure { pub reason: FailureReason, pub detail: String }

// ssh/test_conn.rs
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ConnectionTest {
    Ok { user: String, elapsed_ms: u64 },
    HostUnknown { key_type: String, fingerprint: String },
    PassphraseRequired,
    PasswordAccepted { elapsed_ms: u64 },
    PasswordNotOffered { methods: Vec<String> },
    Failed { reason: FailureReason, detail: String },
    TimedOut,
}

// ssh/agent_keys.rs
pub struct AgentKeyListing { pub socket: Option<String>, pub keys: Vec<AgentKey> }

// session/mod.rs
#[serde(rename_all = "snake_case")]
pub enum ConnectionState { #[default] Live, Connecting, Reconnecting, Dropped, Failed }
// Session gains:
#[serde(default)] pub connection_failure: Option<CanoFailure>,
```

### Commands

| Command | Args | Returns | Errors |
|---|---|---|---|
| `list_agent_keys` | `alias: Option<String>` | `AgentKeyListing` | `ssh.agent_unreachable { detail }` |
| `test_host_connection` | `input: HostInput` | `ConnectionTest` | `ssh.alias_invalid`, `ssh.field_invalid`, `ssh.agent_key_invalid`, `ssh.identity_file_required` |
| `create_host` / `update_host` | unchanged shape, new fields | `Host` | adds `ssh.agent_key_invalid`, `ssh.identity_file_required`, `ssh.auth_fields_conflict` |
| `reconnect_ssh` | `id` | `()` | unchanged; now valid from `failed` too |

Background error added: `ssh.password_needs_session { alias }`.

### Events

`session://status` (existing, full `Session`): the front merges `connection` and
`connection_failure` in addition to `status`, `attention`, `observed`.

### Schema — migration step 8 (`SCHEMA_VERSION = 8`)

- `host.auth_method TEXT NOT NULL DEFAULT 'auto'`
- `host.agent_key TEXT` (JSON of `AgentKey`)
- `sessions.ssh_logged_in INTEGER NOT NULL DEFAULT 0`, backfilled to `1` for every
  existing SSH session row (they were live before this delivery; without the
  backfill an upgrade would silently drop them at boot).
- The base `SCHEMA` creates the same columns for fresh databases.
- Store API: `mark_ssh_logged_in(id)`, `ssh_logged_in(id) -> bool`;
  `upsert_host`/`load_hosts` carry the two host columns.

### Public seams (used by `src-tauri/tests/ssh_real_host.rs`)

- `tyba_lib::ssh::command::std_command() -> std::process::Command` (program `ssh`
  resolved in the login-shell PATH, env applied).
- `tyba_lib::ssh::tmux::wrap_command_with_nonce(name, nonce) -> String` and
  `login_marker(nonce) -> Vec<u8>`.
- `tyba_lib::ssh::classify::classify(&str) -> CanoFailure`.
- `tyba_lib::ssh::test_conn::run(&HostInput, deadline) -> ConnectionTest`.
- `tyba_lib::session::cano::CanoWatch::new(nonce)` with `feed(&[u8]) -> bool`
  (true once, when the marker is seen) and `finish(self) -> CanoOutcome`
  (`LoggedIn` or `NotLoggedIn { tail: Vec<u8> }`).
- The guard for rule 26 lives in `ssh/command.rs` tests and scans
  `src-tauri/src` from `CARGO_MANIFEST_DIR`.

### Remote command

`sh -c 'printf "\033]633;P;tyba-ssh-login=%s\007" <nonce>; <existing tmux wrap>'`
— the existing no-single-quote assertion and the csh test keep passing.

## Invariants that apply

- **Declared state transitions** — rule 13, one table in `session/cano.rs`; no
  direct assignment of `connection` outside the manager method that applies a
  machine decision.
- **Compensation** — rule 10: validate → persist → install; derived files
  regenerate on the next mutation and at boot.
- **No external call inside a DB transaction** — `ssh -G`, `ssh-add`,
  `ssh-keygen` and the test run outside any transaction.
- **Injected clock, UTC** — the Cano machine takes `now` as a parameter;
  `last_connected_at` is UTC.
- **Bounded reads** — pre-login buffer 16 KiB; agent listing 64 keys / 256 KiB;
  test stderr capped at 256 KiB.

## Acceptance criteria

- [ ] Render per method matches rules 2–5, including legacy `auto` + `identity_file`.
- [ ] No input combination renders `UseKeychain` or `IgnoreUnknown`.
- [ ] Every block renders keepalive lines in the order of rule 9.
- [ ] Pub files: written 0600 in a 0700 dir, name derived from the fingerprint, unreferenced files removed.
- [ ] A forged `agent_key` (fingerprint not matching, multi-line key) is rejected.
- [ ] A database at version 7 migrates to 8: hosts load as `auto`, existing SSH sessions get `ssh_logged_in = 1`, other sessions `0`.
- [ ] Create/update with an invalid render writes neither the row nor `tyba.conf`.
- [ ] Classifier returns each reason of rule 15 from hand-written samples, with precedence and `detail` truncation.
- [ ] Watcher: marker split across reads is detected; wrong nonce ignored; buffer capped at 16 KiB; buffering stops after the marker.
- [ ] `wrap_command` emits the marker before tmux and before the no-tmux shell; csh test still passes.
- [ ] Machine: exit before login outside a drop → `failed`, no probe, no respawn.
- [ ] Machine: backoff sequence 1/2/4/8/16/30/30 and `dropped` at 300 s, with an injected clock.
- [ ] Machine: login that lasts < 30 s does not reset the drop clock; ≥ 30 s does.
- [ ] Machine: inside a drop, `no_route`/`host_unresolved` keep retrying; other reasons → `failed`.
- [ ] Machine: every transition not in rule 13 is impossible (table-driven test).
- [ ] PTY respawn under the same id keeps attachers and size.
- [ ] Login marker sets `live`, `ssh_logged_in`, `last_connected_at`; spawn does not touch `last_connected_at`.
- [ ] Boot forgets SSH sessions with `ssh_logged_in = 0` and respawns the others as a drop.
- [ ] `mergeSessionUpdate` propagates `connection` and `connection_failure`.
- [ ] Test connection argv contains every option of rule 22 (plus rule 24 for `password`); temp dir removed on success, failure and timeout; `tyba.conf` untouched.
- [ ] Test connection outcome parsing covers every variant of rule 23 from hand-written samples.
- [ ] `passphrase_required` logic of rule 25 (unit, with a throwaway encrypted key generated in the test).
- [ ] Agent listing parses `ssh-add -L` output, caps at 64 keys, and works against a disposable `ssh-agent` started by the test.
- [ ] Login-shell env resolution returns PATH and SSH_AUTH_SOCK from one marked invocation; failure falls back to the process env.
- [ ] Guard: no `new("ssh")` outside `src-tauri/src/ssh/command.rs`.
- [ ] `password` host without a live master: SFTP/docker/tunnel fail with `ssh.password_needs_session` immediately.
- [ ] `Input`/`Textarea` render `autocapitalize="off"`, `autocorrect="off"`, `spellcheck="false"` by default; listed prose fields opt in.
- [ ] Uppercase-username warning shows and does not block saving.
- [ ] Failure banner shows reason text and actions; `host_key_changed` shows the copyable command and no accept/remove control.
- [ ] E2E on the owner's VPS: `root` + `agent` (the owner's key in the agent) connects from the manager; `Root` shows `auth_refused` in the pane and no reconnect happens; killing the network after login shows `reconnecting` and the pane keeps output after it comes back.

## Out of scope

- Integrated SSH Session (blocks, TYBA command line, remote chips) — `tech-spec-05`.
- Importing hosts from `~/.ssh/config` — backlog (alias shadowing trap).
- Permanent health/status on host cards — health slice.
- Passphrase in Keychain (`UseKeychain`) — refused by rule 7.
- Accepting/replacing host keys by click — refused by rules 20–21.
- Auto-correcting `Root` → `root` on existing hosts.
- ControlMaster on Windows.
- Agent running on the remote host.
