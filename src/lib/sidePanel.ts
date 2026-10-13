import type { SessionId } from "./ipc";

/**
 * O painel lateral único: os ícones da direita do header são as ABAS dele.
 *
 * Quem guarda qual aba está aberta é o core (`workspace.side_view`); este
 * módulo só lê esse valor e decide o que o header mostra — qual ícone acende,
 * que indicador cada um carrega, e se o clique abre ou fecha. Nada de estado
 * de painel no React.
 */
export type PanelKind =
  | "history"
  | "docker"
  | "tunnels"
  | "files"
  | "git"
  | "diff"
  | "agents";

/** Toda aba vira `side_view` no core. */
export type SlotPanel = PanelKind;

/** Ordem do mockup (Navegacao/PainelGit): túneis só aparece em sessão SSH. */
export const PANEL_ORDER: readonly PanelKind[] = [
  "history",
  "docker",
  "tunnels",
  "files",
  "git",
  "diff",
  "agents",
];

const PREFIX: Record<SlotPanel, string> = {
  history: "history:",
  docker: "docker:",
  tunnels: "tunnels:",
  files: "files:",
  git: "git:",
  diff: "diff:",
  agents: "agents:",
};

/**
 * A fila de agentes é o único `side_view` sem sessão. Ocupa o mesmo slot (e a
 * mesma largura), mas não é a aba Agentes desta sessão: não acende ícone, e
 * clicar em Agentes troca a fila pelo painel da sessão em vez de fechá-la.
 */
const AGENT_QUEUE_VIEW = "agent-queue";

export type PanelView = { kind: SlotPanel; session: SessionId };

export function parseSideView(view: string | null | undefined): PanelView | null {
  if (!view) return null;
  for (const kind of Object.keys(PREFIX) as SlotPanel[]) {
    const prefix = PREFIX[kind];
    if (view.startsWith(prefix)) {
      const session = view.slice(prefix.length);
      return session ? { kind, session } : null;
    }
  }
  return null;
}

/**
 * Qual ícone acende. Só acende o painel da sessão em foco: um painel aberto
 * para outra sessão do workspace não é "o painel desta aba" — clicar no ícone
 * abre o desta sessão em vez de fechar o da outra.
 */
export function litPanel(
  sideView: string | null | undefined,
  activeId: SessionId | null,
): SlotPanel | null {
  const view = parseSideView(sideView);
  if (!view || view.session !== activeId) return null;
  return view.kind;
}

export type PanelClick = "open" | "close";

/** Clique no ícone: acende → abre; aceso → fecha. Um painel por vez. */
export function panelClick(
  kind: PanelKind,
  sideView: string | null | undefined,
  activeId: SessionId | null,
): PanelClick {
  return litPanel(sideView, activeId) === kind ? "close" : "open";
}

/**
 * Largura do slot: 380px fixos (mockup). Só o Diff segue no split
 * arrastável — os hunks não cabem em 380px. O diff como aba de conteúdo é
 * outra porta (`openDiffTab` em `lib/contentTabs`, usada pelo painel Git).
 */
export function isNarrowPanel(view: string | null | undefined): boolean {
  if (view === AGENT_QUEUE_VIEW) return true;
  const kind = parseSideView(view)?.kind;
  return kind !== undefined && kind !== "diff";
}

export type PanelIndicator = {
  dot: "green" | "amber" | "red" | null;
  count: number | null;
};

export type IndicatorInput = {
  dockerUp: boolean;
  dockerRunning: number;
  gitDirty: boolean;
  pendingApprovals: number;
};

const NONE: PanelIndicator = { dot: null, count: null };

/** O estado que o ícone mostra sem abrir o painel. */
export function panelIndicator(kind: PanelKind, input: IndicatorInput): PanelIndicator {
  switch (kind) {
    case "docker":
      if (!input.dockerUp) return { dot: "red", count: null };
      return input.dockerRunning > 0 ? { dot: "green", count: null } : NONE;
    case "git":
      return input.gitDirty ? { dot: "amber", count: null } : NONE;
    case "agents":
      return input.pendingApprovals > 0
        ? { dot: null, count: input.pendingApprovals }
        : NONE;
    default:
      return NONE;
  }
}

export type VisibilityInput = {
  hasSession: boolean;
  isSsh: boolean;
  showDocker: boolean;
  hasRepo: boolean;
  agentsAvailable: boolean;
};

/** Quais abas existem para a sessão em foco — as mesmas regras de antes. */
export function visiblePanels(input: VisibilityInput): PanelKind[] {
  if (!input.hasSession) return [];
  return PANEL_ORDER.filter((kind) => {
    switch (kind) {
      case "docker":
        return input.showDocker;
      case "tunnels":
        return input.isSsh;
      case "git":
      case "diff":
        return input.hasRepo;
      case "agents":
        return input.agentsAvailable;
      default:
        return true;
    }
  });
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** aria-label com o estado que o ícone mostra (contagens inclusas). */
export function panelAriaLabel(
  kind: PanelKind,
  input: IndicatorInput & { gitChanged: number | null },
  t: Translate,
): string {
  switch (kind) {
    case "history":
      return t("sidePanelHistory");
    case "docker":
      if (!input.dockerUp) return t("sidePanelDockerDown");
      return input.dockerRunning > 0
        ? t("sidePanelDockerRunning", { count: input.dockerRunning })
        : t("sidePanelDocker");
    case "tunnels":
      return t("tunnelsAction");
    case "files":
      return t("filesPanel");
    case "git":
      return input.gitDirty && input.gitChanged
        ? t("sidePanelGitChanges", { count: input.gitChanged })
        : t("sidePanelGit");
    case "diff":
      return t("sidePanelDiff");
    case "agents":
      return input.pendingApprovals > 0
        ? t("sidePanelAgentsPending", { count: input.pendingApprovals })
        : t("agentsAction");
  }
}

/**
 * Esc fecha o painel — mas não quando o Esc é de um campo (commit, busca,
 * editor) ou já foi tratado por quem estava com o foco.
 */
export function escClosesPanel(event: {
  key: string;
  defaultPrevented: boolean;
  targetEditable: boolean;
}): boolean {
  return event.key === "Escape" && !event.defaultPrevented && !event.targetEditable;
}
