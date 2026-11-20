// Core falso do preview: responde aos `invoke` com os dados de `fixtures.ts`
// e guarda o pouco estado que as telas leem do core (layout, gaveta). Só
// existe no `vite dev` (`preview.html`); o bundle do Tauri não o importa.

import { emit } from "@tauri-apps/api/event";

import type { DrawerState, LayoutState, RepoSnapshot, Session, Workspace } from "../lib/ipc";
import {
  BLOCKS,
  HOME,
  ID,
  SCREENS,
  HOST_GROUPS,
  HOSTS,
  NOTIFICATIONS,
  RUNNING,
  SESSIONS,
  THEMES,
  initialLayout,
} from "./fixtures";
import {
  BRANCH,
  CONTAINER_LOG,
  CONTAINERS,
  DECORATIONS,
  HANDOFF_TARGETS,
  ORBIT_GRAPH,
  PR_DETAIL,
  PR_PAGE,
  RUN_LOG,
  SUBAGENTS,
  TURN_PANEL_GUTTER,
  TURN_PANEL_SRC,
  WORKFLOW_JOBS,
  WORKFLOW_RUNS,
  listDir,
} from "./fixtures-screens";

type Args = Record<string, unknown>;
type Handler = (args: Args) => unknown;

export const state = {
  theme: "dark" as "dark" | "light",
  font: null as "mono" | "grotesk" | null,
  layout: initialLayout(),
  drawers: [] as DrawerState[],
  /** A branch do repo api: a do quadro Header ("main") nas telas do shell;
   *  a do PR nas do Git. */
  branch: BRANCH,
  /** Sessões cujo shell ainda não chegou ao prompt (o Loader do quadro Main). */
  booting: [] as string[],
  /** Repos a mais no `repo_snapshots`, só no cenário que pede. */
  snapshots: [] as RepoSnapshot[],
};

function activeWorkspace(): Workspace {
  const ws = state.layout.workspaces.find((w) => w.id === state.layout.active_workspace);
  return ws ?? state.layout.workspaces[0];
}

function workspaceOf(session: unknown): Workspace {
  return (
    state.layout.workspaces.find((w) =>
      w.tabs.some((t) => JSON.stringify(t.root).includes(String(session))),
    ) ?? activeWorkspace()
  );
}

export function layoutChanged(): void {
  void emit("layout://changed", structuredClone(state.layout) satisfies LayoutState);
}

function setSideView(session: unknown, prefix: string): null {
  const ws = workspaceOf(session);
  state.layout.active_workspace = ws.id;
  ws.side_view = `${prefix}${String(session)}`;
  layoutChanged();
  return null;
}

const REPO_API = SESSIONS.find((x) => x.id === ID.api)?.repo_root ?? "~/projetos/api";

const b64 = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));

/** O PTY "manda" a tela do agente logo depois que o terminal se liga. */
function attach(id: unknown): null {
  const text = SCREENS[String(id)];
  if (text) setTimeout(() => void emit(`pty://output/${String(id)}`, { data: b64(text) }), 50);
  const running = RUNNING[String(id)];
  if (running) {
    setTimeout(
      () =>
        void emit(`session://command/${String(id)}`, {
          command: running.command,
          running: true,
          agent_match: true,
          continuation: false,
          started_at_ms: running.startedAtMs,
        }),
      300,
    );
  }
  return null;
}

const isShell = (id: unknown) => SESSIONS.some((s) => s.id === id && s.kind.type === "shell");

const themeState = () => ({
  mode: state.theme,
  dark: THEMES[0],
  light: THEMES[1],
});

// O quadro mostra o Docker no header (o dono liga o ícone nas Configurações).
const PREFS: Record<string, string> = { "pref.code.show_containers": "on" };
const LSP_READY = { state: "ready", server: "typescript", diagnostics: 0, truncated: false };

const handlers: Record<string, Handler> = {
  boot_snapshot: () => ({
    ready: true,
    bootFailure: null,
    prefs: PREFS,
    sessions: SESSIONS,
    layout: structuredClone(state.layout),
  }),
  boot_poll: () => ({ ready: true, bootFailure: null }),
  layout_state: () => ({ ready: true, value: structuredClone(state.layout) }),
  list_sessions: () => ({ ready: true, value: SESSIONS }),
  get_pref: ({ key }) =>
    key === "pref.ui_font" && state.font ? state.font : (PREFS[String(key)] ?? null),
  set_pref: () => null,
  get_theme_state: themeState,
  list_themes: () => THEMES,
  set_theme_mode: ({ mode }) => {
    state.theme = mode === "light" ? "light" : "dark";
    void emit("theme://changed", themeState());
    return null;
  },
  app_version: () => "2.0.0-preview",
  update_check: () => null,
  config_warnings: () => [],
  list_hosts: () => HOSTS,
  list_host_groups: () => HOST_GROUPS,
  list_recent_folders: () => [],
  list_approvals: () => [],
  notifications_list: () => NOTIFICATIONS,
  // O pedido do codex aparece pelo status da sessão; o toast de "pedindo
  // algo" não está em nenhum quadro.
  agent_asks: () => [],
  agents_drawer_list: () => state.drawers,
  repo_snapshots: () => [
    {
      ahead: 2,
      behind: 0,
      upstream: `origin/${state.branch}`,
      root: REPO_API,
      branch: state.branch,
      status: { dirty: true, changed: 4, insertions: 42, deletions: 7 },
    },
    // core e site limpos na main (o "⎇ main" do site no quadro Sidebar).
    ...[ID.core, ID.site].map((id) => ({
      ahead: 0,
      behind: 0,
      upstream: "origin/main",
      root: SESSIONS.find((x) => x.id === id)?.repo_root ?? "",
      branch: "main",
      status: { dirty: false, changed: 0, insertions: 0, deletions: 0 },
    })),
    ...state.snapshots,
  ],
  // O `bun run dev` do Shell dividido escuta na 3000.
  services_session_ports: () => ({ [ID.server]: [3000] }),
  "plugin:path|resolve_directory": () => HOME,
  attach_session: ({ id }) => attach(id),
  resize_session: () => null,
  session_mark_seen: () => null,
  session_blocks: ({ id }) => BLOCKS[String(id)] ?? [],
  session_prompt_mode: ({ id }) => isShell(id) && !state.booting.includes(String(id)),
  session_hook_expected: ({ id }) => isShell(id),
  session_shell_boot: ({ id }) =>
    isShell(id) && !state.booting.includes(String(id)) ? { shell: "zsh", rc: "~/.zshrc", started_at_ms: Date.now() - 600_000 } : null,
  session_cwd: ({ id }) => {
    const s = SESSIONS.find((x) => x.id === id);
    return s?.repo_root ? { cwd: s.repo_root, canonical: s.repo_root } : null;
  },
  session_git_status: ({ id }) => {
    const s = SESSIONS.find((x) => x.id === id);
    if (!s?.repo_root) return null;
    const branch = id === ID.claudeApi || id === ID.tyba ? BRANCH : "main";
    return { root: s.repo_root, branch, dirty: id !== ID.site };
  },
  list_subagents: ({ sessionId }) => ({ focused: null, subagents: SUBAGENTS[String(sessionId)] ?? [] }),
  handoff_targets: () => HANDOFF_TARGETS,
  session_handoff_preview: ({ note }) => {
    const body = [
      "Handoff de Claude · api",
      "Você continua o trabalho de outro agente nesta mesma pasta.",
      "",
      "Pedido:",
      "roda os três subagentes e me diz se deu erro",
      "",
      "O que o Claude disse por último:",
      "> Os três subagentes terminaram sem erro.",
      "> for de 1 a 10: imprimiu os números de 1 a 10.",
      "> ls -la: listou 12 entradas na pasta.",
    ].join("\n");
    return { text: body, body, note: String(note ?? ""), redacted: 0, folder: "api", target_label: "Codex" };
  },

  // Git: forja de mentira, só leitura.
  forge_status: () => ({ kind: "github", cli: "gh", installed: true, authenticated: true, web_create_url: null }),
  forge_pr_for_session: () => null,
  forge_pr_page: () => PR_PAGE,
  forge_pr_detail: () => PR_DETAIL,
  forge_workflow_runs: () => WORKFLOW_RUNS,
  forge_workflow_jobs: () => WORKFLOW_JOBS,
  forge_run_log: () => RUN_LOG,

  // Arquivos
  files_panel_info: ({ id }) => ({
    root: SESSIONS.find((x) => x.id === id)?.repo_root ?? REPO_API,
    kind: "repo",
    decorated: true,
    remote: false,
    host: null,
    drifted_to: null,
  }),
  files_list_dir: ({ path }) => listDir(String(path ?? "")),
  files_watch_dir: () => null,
  files_unwatch_dir: () => null,
  files_decorations: () => DECORATIONS,
  files_gutter: ({ path }) => (String(path).endsWith("TurnPanel.tsx") ? TURN_PANEL_GUTTER : []),
  files_edit_begin: ({ path }) => ({ text: String(path).endsWith(".tsx") ? TURN_PANEL_SRC : "", hash: "preview" }),
  lsp_status: () => LSP_READY,
  lsp_open: () => LSP_READY,
  lsp_symbol_path: ({ line }) => (Number(line) >= 39 ? ["TurnPanel"] : []),
  lsp_code_actions: () => [],
  lsp_open_doc: () => null,
  lsp_close_doc: () => null,
  lsp_did_change: () => null,
  lsp_change: () => null,
  files_read: () => ({
    kind: "text",
    text: TURN_PANEL_SRC,
    offset: 0,
    next_offset: TURN_PANEL_SRC.length,
    total: TURN_PANEL_SRC.length,
    truncated: false,
  }),

  // Docker
  docker_available: () => true,
  docker_list_containers: () => CONTAINERS,
  docker_compose_runs: () => ({}),
  docker_logs_start: () => ({ lines: CONTAINER_LOG, following: false }),
  docker_logs_stop: () => null,
  docker_panel_usage: () => ({ usage: [], exits: [] }),
  docker_container_detail: ({ containerId }) => {
    const c = CONTAINERS.find((x) => x.id === containerId) ?? CONTAINERS[0];
    return { id: c.id, name: c.name, image: c.image, state: c.state, started_at: null, workdir: null, mounts: [], ports: [] };
  },
  docker_list_images: () => [],
  docker_list_volumes: () => [],
  docker_screen_select: ({ workspace, container }) => {
    const ws = state.layout.workspaces.find((w) => w.id === workspace) ?? activeWorkspace();
    ws.screens.docker.container = (container as string | null) ?? null;
    layoutChanged();
    return null;
  },

  // Configurações e Orbit
  user_config_path: () => "~/.tyba/config.toml",
  list_editors: () => [],
  detach_session: () => null,
  orbit_list_graphs: () => [ORBIT_GRAPH],
  orbit_get_graph: () => ORBIT_GRAPH,
  orbit_runs_list: () => [],
  orbit_node_log: () => [],
  orbit_var_env_problems: () => [],
  orbit_save_graph: () => null,
  orbit_list_roles: () => [{ name: "Revisor", instructions: "Revise o que mudou antes de abrir PR." }],
  orbit_list_integrations: () => [],
  orbit_service_logs_retain: () => null,
  orbit_browser_retain: () => null,
  detected_agent: () => null,
  prompt_queue_list: ({ id }) => ({ session_id: String(id), prompts: [], paused: null }),
  set_app_menu: () => null,
  splash_done: () => null,

  // Intenções que mudam o que a tela mostra: o core falso aplica e avisa.
  open_git_panel: ({ id, sessionId }) => setSideView(id ?? sessionId, "git:"),
  open_files_panel: ({ id, sessionId }) => setSideView(id ?? sessionId, "files:"),
  open_docker_panel: ({ id, sessionId }) => setSideView(id ?? sessionId, "docker:"),
  open_view_tab: ({ view }) => {
    const ws = activeWorkspace();
    const id = `tab-view-${String(view)}`;
    if (!ws.tabs.some((t) => t.id === id)) {
      ws.tabs.push({ id, title: null, view: String(view), active_pane: null, root: null, created_at: new Date().toISOString() });
    }
    ws.active_tab = id;
    layoutChanged();
    return null;
  },
  close_side_view: () => {
    activeWorkspace().side_view = null;
    layoutChanged();
    return null;
  },
};

/** Estado de partida do cenário: qual workspace e qual aba estão à frente. */
export function setup(opts: {
  workspace?: string;
  tab?: string;
  drawer?: DrawerState;
  files?: Workspace["screens"]["files"];
  branch?: string;
  booting?: string[];
  /** Campos a mais em sessões só deste cenário (as outras telas não mudam). */
  sessions?: Record<string, Partial<Session>>;
  snapshots?: RepoSnapshot[];
}): void {
  if (opts.booting) state.booting = opts.booting;
  if (opts.branch) state.branch = opts.branch;
  if (opts.snapshots) state.snapshots = opts.snapshots;
  for (const [id, patch] of Object.entries(opts.sessions ?? {})) {
    const target = SESSIONS.find((x) => x.id === id);
    if (target) Object.assign(target, patch);
  }
  if (opts.workspace) state.layout.active_workspace = opts.workspace;
  if (opts.tab) activeWorkspace().active_tab = opts.tab;
  if (opts.files) activeWorkspace().screens.files = opts.files;
  if (opts.drawer) state.drawers = [opts.drawer];
}

export function handle(cmd: string, args: Args = {}): unknown {
  if (cmd.startsWith("plugin:window|") || cmd.startsWith("plugin:webview|")) return null;
  const handler = handlers[cmd];
  if (!handler) {
    console.warn(`[preview] comando sem fixture: ${cmd}`);
    return null;
  }
  return handler(args);
}
