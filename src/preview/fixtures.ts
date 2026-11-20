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
  prod: "00000000-0000-4000-8000-0000000000b1",
  staging: "00000000-0000-4000-8000-0000000000b2",
  zsh3: "00000000-0000-4000-8000-0000000000b3",
  zsh4: "00000000-0000-4000-8000-0000000000b4",
  zsh5: "00000000-0000-4000-8000-0000000000b5",
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
    // "Pronto · há 12 min" do quadro Sidebar.
    last_turn: {
      seq: 2,
      outcome: "completed",
      failure: null,
      started_at_ms: NOW - 780_000,
      ended_at_ms: NOW - 720_000,
      duration_ms: 60_000,
      prompt: null,
      diff: "ok",
      files: [],
      files_total: 1,
      insertions: 12,
      deletions: 3,
      commits: 0,
      cost_usd: 0.1,
      tokens: 2_000,
    },
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
    status_note: "editando handoff.rs",
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
    // Quadro Agents: "em andamento" e a aba "Turno · 3" (o turno anterior).
    subagents_working: false,
    last_turn: {
      seq: 2,
      outcome: "completed",
      failure: null,
      started_at_ms: NOW - 400_000,
      ended_at_ms: NOW - 300_000,
      duration_ms: 100_000,
      prompt: null,
      diff: "ok",
      files: [],
      files_total: 3,
      insertions: 42,
      deletions: 7,
      commits: 0,
      cost_usd: 0.4,
      tokens: 12_000,
    },
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
  // As SSH do quadro Sidebar (prod-01 e staging, conectadas).
  session({ id: ID.prod, kind: { type: "ssh", host_id: "host-prod" }, title: "prod-01", connection: "live" }),
  session({ id: ID.staging, kind: { type: "ssh", host_id: "host-staging" }, title: "staging", connection: "live" }),
  // As três abas a mais do `+3` do quadro Header.
  ...[ID.zsh3, ID.zsh4, ID.zsh5].map((id) =>
    session({ id, kind: { type: "shell" }, title: "zsh", repo_root: REPO.api }),
  ),
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
      // As abas "api" do quadro Header: quatro à vista (codex, claude, o
      // split e o zsh, ativo) e três no `+3`.
      {
        ...workspace("api", ID.api, REPO.api, [
          tab("tab-api-claude", leaf("pane-api-claude", ID.claudeApi)),
          tab("tab-api-split", {
            type: "split",
            id: "split-api",
            split: "v",
            ratio: 0.5,
            first: { type: "leaf", id: "pane-split-a", session_id: ID.claudeSplit },
            second: { type: "leaf", id: "pane-split-b", session_id: ID.server },
          }),
          tab("tab-api-shell", leaf("pane-api-shell", ID.dev)),
          tab("tab-api-z3", leaf("pane-api-z3", ID.zsh3)),
          tab("tab-api-z4", leaf("pane-api-z4", ID.zsh4)),
          tab("tab-api-z5", leaf("pane-api-z5", ID.zsh5)),
        ]),
        active_tab: "tab-api-shell",
      },
      workspace("core", ID.core, REPO.core),
      workspace("site", ID.site, REPO.site),
      workspace("prod-01", ID.prod, null),
      workspace("staging", ID.staging, null),
      // Os agentes notas e tyba-terminal do quadro moram no grupo Clientes
      // (fechado na barra), para "Este computador" ficar com api, core e site.
      { ...workspace("notas", ID.notas, REPO.notas), group: "Clientes" },
      { ...workspace("tyba-terminal", ID.tyba, REPO.tyba), group: "Clientes" },
    ],
    active_workspace: "ws-api",
  };
}

export const HOST_GROUPS: HostGroup[] = [
  { id: "grp-clientes", name: "Clientes", color: "amber", notes: null, position: 0, created_at: ISO },
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

/** Esquema embutido do core (`theme/schemes.rs`), só a paleta do terminal. */
function scheme(id: string, name: string, base: "dark" | "light", bg: string, fg: string, cursor: string, ansi: string): Theme {
  return { id, name, base, builtin: true, ui: {}, terminal: { background: bg, foreground: fg, cursor, ansi: ansi.split(" ") } };
}

// A lista do core: os dois TYBA e os esquemas embutidos, com os nomes de lá.
export const THEMES: Theme[] = [
  { id: "tyba-dark", name: "TYBA Dark", base: "dark", builtin: true, ui: {}, terminal: DARK_TERMINAL },
  { id: "tyba-light", name: "TYBA Light", base: "light", builtin: true, ui: {}, terminal: LIGHT_TERMINAL },
  scheme("solarized-dark", "Solarized Dark", "dark", "#002b36", "#93a1a1", "#859900", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #93a1a1 #586e75 #e25754 #a3bf00 #c29e2e #4da0da #6c71c4 #50b2ab #ffffff"),
  scheme("solarized-light", "Solarized Light", "light", "#f5efdc", "#073642", "#859900", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5 #93a1a1 #e25754 #98ad10 #c29e2e #4da0da #6c71c4 #50b2ab #ffffff"),
  scheme("dracula", "Dracula", "dark", "#282a36", "#f8f8f2", "#50fa7b", "#2f313f #ff5555 #50fa7b #ffb86c #7b9eff #ff79c6 #8be9fd #f8f8f2 #6272a4 #ff7474 #7dfc9d #ffc586 #93afff #bd93f9 #a0edfd #ffffff"),
  scheme("dracula-light", "Dracula Light (Alucard)", "light", "#f8f3de", "#1f1f1f", "#14710a", "#1f1f1f #cb3a2a #14710a #a34d14 #2f5fc9 #a3144d #036a96 #efead5 #918c73 #d45d50 #3d8f2a #b46d3e #547cd3 #644ac9 #3085a9 #ffffff"),
  scheme("gruvbox-dark", "Gruvbox Dark", "dark", "#282828", "#ebdbb2", "#b8bb26", "#32302f #fb4934 #b8bb26 #fabd2f #83a598 #d3869b #8ec07c #ebdbb2 #928374 #fc6a59 #d5dc3a #fbc954 #99b5ab #b16286 #a2cb94 #ffffff"),
  scheme("gruvbox-light", "Gruvbox Light", "light", "#f2e5bc", "#3c3836", "#79740e", "#3c3836 #9d0006 #79740e #b57614 #076678 #b16286 #427b58 #ebdbb2 #928374 #af2e33 #98971a #c28f3e #348290 #8f3f71 #649376 #ffffff"),
  scheme("github-dark", "GitHub Dark", "dark", "#0d1117", "#e6edf3", "#3fb950", "#161b22 #f85149 #3fb950 #d29922 #58a6ff #db61a2 #39c5cf #e6edf3 #6e7681 #f9706a #56d364 #daab4a #76b6ff #a371f7 #5dcfd8 #ffffff"),
  scheme("github-light", "GitHub Light", "light", "#f6f8fa", "#1f2328", "#1a7f37", "#1f2328 #cf222e #1a7f37 #9a6700 #0969da #bf3989 #1b7c83 #eaeef2 #8c959f #d84a54 #2da44e #ac822e #3584e1 #8250df #449499 #ffffff"),
  scheme("monokai", "Monokai", "dark", "#272822", "#f8f8f2", "#a6e22e", "#2e2f28 #fc4c3f #a6e22e #fd971f #66d9ef #f92672 #a1efe4 #f8f8f2 #75715e #fd6c62 #c4f04a #fdaa47 #82e0f2 #ae81ff #b2f2e9 #ffffff"),
  scheme("monokai-pro", "Monokai Pro", "dark", "#2d2a2e", "#fcfcfa", "#a9dc76", "#353236 #f4485c #a9dc76 #fc9867 #78dce8 #ff6188 #7ce8cd #fcfcfa #939293 #f66979 #c0e392 #fdab82 #90e2ec #ab9df2 #94ecd6 #ffffff"),
  scheme("monokai-machine", "Monokai Pro Machine", "dark", "#273136", "#f2fffc", "#a2e57b", "#2d3a40 #f4536a #a2e57b #ffb270 #7cd5f1 #ff6d7e #7fe0ce #f2fffc #6b7678 #f67285 #baf090 #ffc08a #94ddf4 #baa0f8 #96e6d7 #ffffff"),
  scheme("monokai-octagon", "Monokai Pro Octagon", "dark", "#282a3a", "#eaf2f1", "#bad761", "#2f3142 #f4506b #bad761 #ff9b5e #9cd1bb #ff657a #8ce0d8 #eaf2f1 #696d77 #f67086 #d2e87e #ffad7b #aed9c7 #c39ac9 #a1e6df #ffffff"),
  scheme("monokai-ristretto", "Monokai Pro Ristretto", "dark", "#2c2525", "#fff1f3", "#adda78", "#342c2c #f04a60 #adda78 #f38d70 #85dacc #fd6883 #7fe4c8 #fff1f3 #72696a #f36b7d #c5ea90 #f5a28a #9be1d5 #a8a9eb #96e9d2 #ffffff"),
  scheme("monokai-spectrum", "Monokai Pro Spectrum", "dark", "#222222", "#f7f1ff", "#7bd88f", "#2a2a2b #f0485e #7bd88f #fd9353 #5ad4e6 #fc618d #62e2cc #f7f1ff #69676c #f3697b #97e5a6 #fda672 #78dcea #948ae3 #7ee7d5 #ffffff"),
  scheme("monokai-light", "Monokai Pro Light", "light", "#faf4f2", "#29242a", "#269d69", "#29242a #d0342c #269d69 #cc7a0a #1c8ca8 #e14775 #1da08a #ede7e5 #a59fa0 #d85952 #4db07e #d59236 #45a1b8 #7058be #46b19f #ffffff"),
  scheme("mono-dark", "Mono Dark", "dark", "#151313", "#eeffff", "#ffd867", "#1a1616 #ff5370 #c3e88d #ffcb6b #82aaff #f07178 #89ddff #eeffff #5c595f #ff728a #d7f2ab #ffd486 #98b9ff #c792ea #9ee3ff #ffffff"),
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

type Seg = string | [string, number];

/** Linha da saída com as cores ANSI que o programa pintou (`[texto, índice]`). */
function line(segs: Seg | Seg[]): Block["lines"][number] {
  const list = Array.isArray(segs) && typeof segs[1] !== "number" ? (segs as Seg[]) : [segs as Seg];
  let text = "";
  const runs: Block["lines"][number]["runs"] = [];
  for (const seg of list) {
    const [part, idx] = typeof seg === "string" ? [seg, null] : seg;
    if (idx !== null) {
      runs.push({
        start: text.length,
        end: text.length + part.length,
        fg: { kind: "idx", value: idx },
        bg: { kind: "default" },
        bold: false,
        italic: false,
        underline: false,
      });
    }
    text += part;
  }
  return { text, runs };
}

const GREEN = 2;
const RED = 1;
const GRAY = 8;

function block(id: number, sessionId: string, command: string, exitCode: number, ms: number, lines: (Seg | Seg[])[]): Block {
  const finished = NOW - 60_000 * (5 - id);
  return {
    id,
    sessionId,
    command,
    exitCode,
    startedAtMs: finished - ms,
    finishedAtMs: finished,
    lines: lines.map(line),
    truncated: 0,
    cwd: REPO.api,
    altScreen: false,
  };
}

/** Blocos do quadro Shell (zsh em api) e do Shell dividido (o servidor). */
export const BLOCKS: Record<string, Block[]> = {
  [ID.dev]: [
    // As cores do quadro Shell: M/A em verde, o resumo em cinza, falha em
    // vermelho.
    block(1, ID.dev, "git status --short", 0, 100, [
      [["M", GREEN], "  src/billing/billing.service.ts"],
      [["A", GREEN], "  src/billing/invoice.test.ts"],
    ]),
    block(2, ID.dev, "bun run test", 1, 4_200, [
      ["bun test v1.2 · 42 testes", GRAY],
      [["✗", RED], " invoice › calcula o total com desconto"],
      ["  esperado 90, recebido 100", GRAY],
      [["41 passaram", GREEN], " · ", ["1 falhou", RED]],
    ]),
  ],
  [ID.server]: [
    block(3, ID.server, "curl localhost:3000/billing", 1, 200, ["error: discount_after_tax"]),
  ],
  // O Terminal · api do quadro Orbit.
  "orbit-term": [
    block(4, "orbit-term", "ls -la", 0, 600, [
      "drwxr-xr-x   8 dev  staff   256 Oct 14 docs",
      "drwxr-xr-x 128 dev  staff  4096 Oct 15 drizzle",
      "-rw-r--r--   1 dev  staff  1786 Oct  5 package.json",
      "drwxr-xr-x   5 dev  staff   160 Oct  5 src",
      "drwxr-xr-x   6 dev  staff   192 Oct  5 test",
    ]),
  ],
};

const ESC = "\x1b[";
const dim = (t: string) => `${ESC}2m${t}${ESC}0m`;
const bold = (t: string) => `${ESC}1m${t}${ESC}0m`;
const gray = (t: string) => `${ESC}90m${t}${ESC}0m`;
const orange = (t: string) => `${ESC}38;2;217;119;87m${t}${ESC}0m`;
const nl = (lines: string[]) => lines.join("\r\n") + "\r\n";

/** O que a tela do agente mostra (o PTY), copiado dos quadros. */
export const SCREENS: Record<string, string> = {
  // As cores do quadro Agents: o ● da ferramenta azul, o texto em negrito e
  // a árvore e a espera em cinza.
  [ID.claudeApi]: nl([
    "❯ rode subagentes com ls -la e for 1 até 10 e ai sleep de 2 no final",
    `${ESC}34m●${ESC}0m ${bold("3 background agents launched")}`,
    gray("├ Run ls -la"),
    gray("├ Run for loop 1 to 10"),
    gray("└ Run sleep 2"),
    "",
    `● ${bold("Iniciei 3 subagentes em paralelo, um para cada comando.")}`,
    "",
    gray("✻ Waiting for 3 background agents to finish"),
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
    "Listening on http://localhost:3000",
    dim("GET /health 200 · 2 ms"),
    dim("GET /invoices 200 · 14 ms"),
  ]),
};
SCREENS[ID.core] = SCREENS[ID.claudeSplit];

// O run ao vivo do quadro Orbit: o shell do Terminal · api e o claude do Revisor.
SCREENS["orbit-term"] = nl([
  `${ESC}34m❯${ESC}0m ls -la`,
  "drwxr-xr-x   8 dev  staff   256 Oct 14 docs",
  "drwxr-xr-x 128 dev  staff  4096 Oct 15 drizzle",
  "-rw-r--r--   1 dev  staff  1786 Oct  5 package.json",
  "drwxr-xr-x   5 dev  staff   160 Oct  5 src",
  "drwxr-xr-x   6 dev  staff   192 Oct  5 test",
]);
SCREENS["orbit-rev"] = nl([
  `${orange("●")} Li o contexto do Terminal · api.`,
  dim("  ⎿ Leu billing.service.ts (180 linhas)"),
  `${orange("●")} Quero rodar os testes antes de revisar.`,
  "",
  `Rodar ${bold("bun run test")}? · responda no prompt do agente`,
]);
// Quadro Shell: o claude digitado no zsh, rodando há 1m 12s, embaixo dos blocos.
SCREENS[ID.dev] = SCREENS[ID.claudeSplit].split("\r\n").slice(0, 3).join("\r\n") + "\r\n";

/** Comando em execução por sessão (o `session://command` do core). */
export const RUNNING: Record<string, { command: string; startedAtMs: number }> = {
  [ID.dev]: { command: "claude", startedAtMs: NOW - 72_000 },
  [ID.server]: { command: "bun run dev", startedAtMs: NOW - 720_000 },
};
