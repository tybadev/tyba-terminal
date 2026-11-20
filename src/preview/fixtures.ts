// Dados do preview: todos sintéticos, copiados dos quadros (que já são dado
// fictício). Nada daqui vem da máquina de quem roda — nenhum caminho, host,
// usuário ou IP real.

import type {
  Block,
  Host,
  HostGroup,
  LayoutState,
  NotificationEntry,
  Session,
  Tab,
  Theme,
  TurnDigest,
  Workspace,
} from "../lib/ipc";

const NOW = Date.now();
const today = (h: number, m: number) => {
  const d = new Date(NOW);
  d.setHours(h, m, 0, 0);
  return d.getTime();
};
const ISO = new Date(NOW - 3_600_000).toISOString();

export const HOME = "~";

/** Ids em formato UUID: parte do front corta a sessão pelo tamanho (36). */
export const ID = {
  api: "00000000-0000-4000-8000-0000000000a1",
  core: "00000000-0000-4000-8000-0000000000a2",
  site: "00000000-0000-4000-8000-0000000000a3",
  notas: "00000000-0000-4000-8000-0000000000a4",
  tyba: "00000000-0000-4000-8000-0000000000a5",
  dev: "00000000-0000-4000-8000-0000000000a6",
  claudeApi: "00000000-0000-4000-8000-0000000000a7",
  server: "00000000-0000-4000-8000-0000000000a8",
  claudeSplit: "00000000-0000-4000-8000-0000000000a9",
} as const;

export const REPO = {
  api: `${HOME}/projetos/api`,
  core: `${HOME}/projetos/core`,
  site: `${HOME}/projetos/site`,
  notas: `${HOME}/projetos/notas`,
  tyba: `${HOME}/projetos/tyba-terminal`,
};

function session(over: Partial<Session> & Pick<Session, "id" | "kind" | "title">): Session {
  return {
    repo_root: null,
    worktree: null,
    status: { state: "idle", summary: null },
    attention: false,
    created_at: ISO,
    ...over,
  };
}

export const SESSIONS: Session[] = [
  session({
    id: ID.api,
    kind: { type: "agent", runner: "codex" },
    title: "api",
    repo_root: REPO.api,
    status: { state: "awaiting_input", hint: "bun run test", reason: "native_prompt" },
    attention: true,
  }),
  session({
    id: ID.core,
    kind: { type: "agent", runner: "claude_code" },
    title: "core",
    repo_root: REPO.core,
    status: { state: "running" },
    agent_model: "Opus 5.5",
    turn_open: true,
    turn_started_at_ms: NOW - 72_000,
    // O turno das 14:02 da Inbox (quadro: "Ver diff" nele).
    last_turn: {
      seq: 4,
      outcome: "completed",
      failure: null,
      started_at_ms: today(14, 2) - 95_000,
      ended_at_ms: today(14, 2),
      duration_ms: 95_000,
      prompt: "Corrige a ordem do desconto no billing",
      diff: "ok",
      files: [],
      files_total: 3,
      insertions: 42,
      deletions: 7,
      commits: 0,
      cost_usd: 0.62,
      tokens: null,
    },
  }),
  session({
    id: ID.site,
    kind: { type: "shell" },
    title: "site",
    repo_root: REPO.site,
  }),
  session({
    id: ID.notas,
    kind: { type: "agent", runner: "claude_code" },
    title: "notas",
    repo_root: REPO.notas,
    status: { state: "idle", summary: "atualizou v2.md (3 seções)" },
  }),
  session({
    id: ID.tyba,
    kind: { type: "agent", runner: "claude_code" },
    title: "tyba-terminal",
    repo_root: REPO.tyba,
    status: { state: "running" },
    turn_open: true,
    turn_started_at_ms: NOW - 134_000,
    subagents_working: false,
    agent_usage: { lines_added: 18, lines_removed: 4 },
  }),
  session({
    id: ID.dev,
    kind: { type: "shell" },
    title: "zsh",
    repo_root: REPO.api,
  }),
  session({
    id: ID.claudeApi,
    kind: { type: "agent", runner: "claude_code" },
    title: "api",
    repo_root: REPO.api,
    status: { state: "running" },
    agent_model: "Opus 5.5",
    turn_open: true,
    turn_started_at_ms: NOW - 29_000,
    subagents_working: true,
    agent_usage: { model: "Opus 5.5", context_pct: 7, tokens_in: 66_000, cost_usd: 1.05, duration_ms: 29_000 },
  }),
  session({
    id: ID.claudeSplit,
    kind: { type: "agent", runner: "claude_code" },
    title: "api",
    repo_root: REPO.api,
    status: { state: "running" },
    turn_open: true,
    turn_started_at_ms: NOW - 72_000,
  }),
  session({
    id: ID.server,
    kind: { type: "shell" },
    title: "zsh",
    repo_root: REPO.api,
    status: { state: "running" },
  }),
];

const EMPTY_SCREENS: Workspace["screens"] = {
  files: { section: "files", tabs: [], active: null, recent: [] },
  docker: { section: "containers", container: null, pane: "logs", project: null, hide_stopped: false },
  services: { section: "ports", only_mine: true, selected: null },
};

function tab(id: string, root: Tab["root"]): Tab {
  return {
    id,
    title: null,
    view: null,
    active_pane: root && root.type !== "split" ? root.id : root?.type === "split" ? root.first.id : null,
    root,
    created_at: ISO,
  };
}

const leaf = (id: string, session: string): Tab["root"] => ({ type: "leaf", id, session_id: session });

function workspace(name: string, sessionId: string, repo: string | null, extra: Tab[] = []): Workspace {
  return {
    id: `ws-${name}`,
    name,
    name_locked: false,
    repo_root: repo,
    color: null,
    group: null,
    kind: "user",
    launch_config_id: null,
    active_tab: `tab-${name}`,
    tabs: [tab(`tab-${name}`, leaf(`pane-${name}`, sessionId)), ...extra],
    side_view: null,
    side_ratio: 0.42,
    side_expanded: false,
    screens: structuredClone(EMPTY_SCREENS),
    created_at: ISO,
  };
}

export function initialLayout(): LayoutState {
  return {
    workspaces: [
      // As quatro abas "api" do quadro Header: codex, zsh, claude e o split.
      {
        ...workspace("api", ID.api, REPO.api, [
          tab("tab-api-shell", leaf("pane-api-shell", ID.dev)),
          tab("tab-api-claude", leaf("pane-api-claude", ID.claudeApi)),
          tab("tab-api-split", {
            type: "split",
            id: "split-api",
            split: "h",
            ratio: 0.5,
            first: { type: "leaf", id: "pane-split-a", session_id: ID.claudeSplit },
            second: { type: "leaf", id: "pane-split-b", session_id: ID.server },
          }),
        ]),
        active_tab: "tab-api-shell",
      },
      workspace("core", ID.core, REPO.core),
      workspace("site", ID.site, REPO.site),
      workspace("notas", ID.notas, REPO.notas),
      workspace("tyba-terminal", ID.tyba, REPO.tyba),
    ],
    active_workspace: "ws-api",
  };
}

export const HOST_GROUPS: HostGroup[] = [
  { id: "grp-clientes", name: "Clientes", color: null, notes: null, position: 0, created_at: ISO },
];

function host(id: string, alias: string, hostname: string, position: number, group: string | null): Host {
  return {
    id,
    alias,
    hostname,
    port: null,
    username: "deploy",
    identity_file: null,
    proxy_jump: null,
    group_id: group,
    color: null,
    notes: null,
    position,
    tunnels: [],
    created_at: ISO,
    last_connected_at: null,
  };
}

export const HOSTS: Host[] = [
  host("host-prod", "prod-01", "prod-01.exemplo.dev", 0, null),
  host("host-staging", "staging", "staging.exemplo.dev", 1, null),
  host("host-cli-a", "cliente-a", "cliente-a.exemplo.dev", 2, "grp-clientes"),
];

const LIGHT_TERMINAL = {
  background: "#ffffff",
  foreground: "#17171c",
  cursor: "#1152d2",
  cursorAccent: "#ffffff",
  selectionBackground: "#1152d24d",
  ansi: [
    "#2a2a31", "#d6402e", "#4e9622", "#c47c14",
    "#3b66d6", "#d33d84", "#0f9d8c", "#e9e9ee",
    "#6d6d78", "#e05a48", "#6fae2e", "#d69a2b",
    "#5d84e0", "#7a4ce8", "#16b3a0", "#ffffff",
  ],
};

const DARK_TERMINAL = {
  background: "#000000",
  foreground: "#f5f5f6",
  cursor: "#7cc544",
  cursorAccent: "#000000",
  selectionBackground: "#7cc5444d",
  ansi: [
    "#0f0f10", "#f0503c", "#7cc544", "#f5a93b",
    "#4c7df0", "#ec4899", "#2dd4bf", "#f5f5f6",
    "#5f6066", "#f4715f", "#a8e05f", "#f5c93b",
    "#7da3f5", "#a78bfa", "#5eead4", "#ffffff",
  ],
};

export const THEMES: Theme[] = [
  { id: "tyba-dark", name: "TYBA Escuro", base: "dark", builtin: true, ui: {}, terminal: DARK_TERMINAL },
  { id: "tyba-light", name: "TYBA Claro", base: "light", builtin: true, ui: {}, terminal: LIGHT_TERMINAL },
];

function note(over: Partial<NotificationEntry> & Pick<NotificationEntry, "id" | "kind" | "category">): NotificationEntry {
  return {
    at_ms: NOW,
    session_id: ID.api,
    session_title: "api",
    agent: "codex",
    approval_id: null,
    pending: false,
    decision: null,
    risk: null,
    command: null,
    detail: null,
    usage: null,
    turn: null,
    read: false,
    ...over,
  };
}

export const NOTIFICATIONS: NotificationEntry[] = [
  note({
    id: 5,
    kind: "native_approval",
    category: "request",
    at_ms: NOW - 40_000,
    pending: true,
    command: "bun run test",
    read: true,
  }),
  note({
    id: 4,
    kind: "agent_finished",
    category: "done",
    at_ms: today(14, 2),
    session_id: ID.core,
    session_title: "core",
    agent: "claude",
    detail: "Corrige a ordem do desconto no billing",
    turn: { seq: 4, files: 3, insertions: 42, deletions: 7, duration_ms: 95_000, cost_usd: 0.62 },
  }),
  note({
    id: 3,
    kind: "agent_failed",
    category: "error",
    at_ms: today(13, 40),
    detail: "O processo saiu com código 1 no meio do turno.",
  }),
  note({
    id: 2,
    kind: "usage_limit",
    category: "info",
    at_ms: today(12, 15),
    session_id: ID.core,
    session_title: "core",
    agent: "claude",
    usage: { window: "five_hour", pct: 85, resets_at: Math.floor(today(16, 0) / 1000) },
  }),
  note({
    id: 1,
    kind: "agent_finished",
    category: "done",
    at_ms: today(22, 10) - 86_400_000,
    session_id: ID.site,
    session_title: "site",
    agent: "claude",
    turn: { seq: 1, files: 1, insertions: 3, deletions: 1, duration_ms: 20_000, cost_usd: 0.08 },
    read: true,
  }),
];

/** O turno do quadro Handoff ("turno das 14:46"). */
export const TURN_HANDOFF: TurnDigest = {
  seq: 3,
  outcome: "completed",
  failure: null,
  started_at_ms: today(14, 46),
  ended_at_ms: today(14, 46) + 3_000,
  duration_ms: 3_000,
  prompt: "roda os três subagentes e me diz se deu erro",
  diff: "ok",
  files: [],
  files_total: 0,
  insertions: 0,
  deletions: 0,
  commits: 0,
  cost_usd: 0.04,
  tokens: 3_200,
};

// --- terminal --------------------------------------------------------------

const plain = (text: string) => ({ text, runs: [] });

function block(id: number, sessionId: string, command: string, exitCode: number, ms: number, lines: string[]): Block {
  const finished = NOW - 60_000 * (5 - id);
  return {
    id,
    sessionId,
    command,
    exitCode,
    startedAtMs: finished - ms,
    finishedAtMs: finished,
    lines: lines.map(plain),
    truncated: 0,
    cwd: REPO.api,
    altScreen: false,
  };
}

/** Blocos do quadro Shell (zsh em api) e do Shell dividido (o servidor). */
export const BLOCKS: Record<string, Block[]> = {
  [ID.dev]: [
    block(1, ID.dev, "git status --short", 0, 100, [
      " M src/billing/billing.service.ts",
      "A  src/billing/invoice.test.ts",
    ]),
    block(2, ID.dev, "bun run test", 1, 4_200, [
      "bun test v1.2 · 42 testes",
      "✗ invoice › calcula o total com desconto",
      "  esperado 90, recebido 100",
      "41 passaram · 1 falhou",
    ]),
  ],
  [ID.server]: [
    block(3, ID.server, "curl localhost:3000/billing", 1, 200, ["error: discount_after_tax"]),
  ],
};

const ESC = "\x1b[";
const dim = (t: string) => `${ESC}2m${t}${ESC}0m`;
const bold = (t: string) => `${ESC}1m${t}${ESC}0m`;
const orange = (t: string) => `${ESC}38;2;217;119;87m${t}${ESC}0m`;
const nl = (lines: string[]) => lines.join("\r\n") + "\r\n";

/** O que a tela do agente mostra (o PTY), copiado dos quadros. */
export const SCREENS: Record<string, string> = {
  [ID.claudeApi]: nl([
    dim("❯ rode subagentes com ls -la e for 1 até 10 e ai sleep de 2 no final"),
    "",
    `${orange("●")} 3 background agents launched`,
    dim("  ├ Run ls -la"),
    dim("  ├ Run for loop 1 to 10"),
    dim("  └ Run sleep 2"),
    "",
    `${orange("●")} Iniciei 3 subagentes em paralelo, um para cada comando.`,
    "",
    orange("✻ Waiting for 3 background agents to finish"),
  ]),
  [ID.claudeSplit]: nl([
    `${orange("●")} O desconto é aplicado depois do imposto. Vou corrigir a ordem.`,
    dim("  ⎿ Editou billing.service.ts +6 −2"),
    dim("  ⎿ bun run test invoice"),
    "",
    dim("❯ Peça algo ao claude…"),
  ]),
  [ID.api]: nl([
    bold("codex quer rodar um comando"),
    "",
    "  $ bun run test",
    "",
    "  1. Sim, rodar",
    "  2. Sim, e não perguntar de novo",
    "  3. Não, dizer ao codex o que fazer",
  ]),
  [ID.tyba]: nl([
    `${orange("●")} Editando handoff.rs`,
    dim("  ⎿ +18 −4"),
    dim("  ⎿ Explore: achar o TurnPanel"),
  ]),
  [ID.notas]: nl([`${orange("●")} Atualizei v2.md (3 seções).`]),
  [ID.server]: nl([
    `${ESC}32m❯${ESC}0m bun run dev`,
    "Listening on http://localhost:3000",
    dim("GET /health 200 · 2 ms"),
    dim("GET /invoices 200 · 14 ms"),
  ]),
};
SCREENS[ID.core] = SCREENS[ID.claudeSplit];
