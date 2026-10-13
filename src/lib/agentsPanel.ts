import type {
  ApprovalRequest,
  Session,
  SessionId,
  SessionKind,
  SessionStatus,
  SubagentRun,
  Workspace,
  WorkspaceId,
} from "./ipc";
import { isFinishedStatus, statusVisual, type StatusVisual } from "./sessionStatus";
import { isNarrowPanel } from "./sidePanel";

const AGENTS_VIEW_PREFIX = "agents:";

export const AGENTS_PANEL_LINGER_MS = 2_500;

export const agentsPanelSession = (sideView: string | null): SessionId | null => {
  if (!sideView?.startsWith(AGENTS_VIEW_PREFIX)) return null;
  return sideView.slice(AGENTS_VIEW_PREFIX.length);
};

/**
 * Como a coluna principal (a dos terminais) divide a largura com a superfície
 * lateral NESTE quadro.
 *
 * O painel Agentes sai com um fade de 150ms (`usePresence`): `sideView` já é
 * `null`, mas o painel continua montado e ocupando os 380px até o fade acabar.
 * Quem refaz o fit do terminal ao fechar tem que esperar ESTE valor mudar, não
 * `sideView` — o pedido de relayout disparado pelo `sideView` cai durante o
 * fade, mede a coluna ainda estreita, e o terminal ficava com a largura do
 * painel aberto depois que ele sumia (o PTY preso em 120 colunas num pane de
 * 168).
 */
export type MainColumn = {
  sideVisible: boolean;
  renderSideView: string | null;
  agentsSideNarrow: boolean;
  /** Muda exatamente quando a largura da coluna principal muda. */
  slot: string;
};

export const mainColumn = (
  sideView: string | null,
  agentsMounted: boolean,
  lastAgentsSession: SessionId | null,
  sideExpanded: boolean,
  sideRatio: number,
): MainColumn => {
  const sideVisible = Boolean(sideView) || agentsMounted;
  const renderSideView =
    sideView ??
    (agentsMounted && lastAgentsSession
      ? `${AGENTS_VIEW_PREFIX}${lastAgentsSession}`
      : null);
  // Nome histórico: hoje vale para todo painel de 380px fixos (agentes, git,
  // docker, túneis) — ver `isNarrowPanel`.
  const agentsSideNarrow = isNarrowPanel(renderSideView) && !sideExpanded;
  const slot = !sideVisible
    ? "full"
    : sideExpanded
      ? "hidden"
      : agentsSideNarrow
        ? "narrow"
        : `split:${sideRatio}`;
  return { sideVisible, renderSideView, agentsSideNarrow, slot };
};

export const deadAgentsPanels = (
  workspaces: Workspace[],
  sessions: Session[],
  seenSessions: Set<SessionId>,
): WorkspaceId[] => {
  const alive = new Map(sessions.map((s) => [s.id, s]));
  const out: WorkspaceId[] = [];
  for (const ws of workspaces) {
    const sessionId = agentsPanelSession(ws.side_view);
    if (!sessionId) continue;
    const session = alive.get(sessionId);
    if (session) {
      if (isFinishedStatus(session.status)) out.push(ws.id);
    } else if (seenSessions.has(sessionId)) {
      out.push(ws.id);
    }
  }
  return out;
};

export const showAgentsButton = (
  kind: SessionKind,
  detected: boolean,
): boolean => {
  if (kind.type === "agent") return true;
  if (kind.type === "shell") return detected;
  return false;
};

/**
 * Mostra o selo "sem gate" no painel de agentes?
 *
 * Shim v2 (tech-spec §7): `hosting` é o que decide, não só o `kind`. Um
 * `claude` hospedado pelo shim continua com `kind.type === "shell"` — o shim
 * mora dentro do shell, não vira sessão de agente —, mas já está dentro do
 * gate (princípios 1/4/6/10 do CLAUDE.md), jaulado ou não. Badge "sem gate"
 * é só para o caso `!hosting && detected`; hospedado é o sinal âmbar "sem
 * jaula" (`showUnjailedNotice`), nunca este.
 */
export const agentsPanelUngated = (
  kind: SessionKind,
  hosting: boolean,
): boolean => kind.type !== "agent" && !hosting;

const anyActive = (subagents: SubagentRun[]): boolean =>
  subagents.some((s) => s.status === "running" || s.status === "starting");

export const agentsPanelRunConcluded = (
  kind: SessionKind,
  status: SessionStatus,
  subagents: SubagentRun[],
): boolean => {
  if (subagents.length === 0 || anyActive(subagents)) return false;
  if (kind.type === "agent") {
    return (
      status.state === "idle" ||
      status.state === "exited" ||
      status.state === "failed"
    );
  }
  return true;
};

export type PanelRunEntry = { session: SessionId; armed: boolean };

export const trackPanelRun = (
  entry: PanelRunEntry | undefined,
  session: SessionId,
  concluded: boolean,
): { entry: PanelRunEntry; action: "cancel" | "schedule" | "none" } => {
  if (!entry || entry.session !== session) {
    return { entry: { session, armed: !concluded }, action: "cancel" };
  }
  if (!concluded) {
    return { entry: { session, armed: true }, action: "cancel" };
  }
  if (entry.armed) {
    return { entry: { session, armed: false }, action: "schedule" };
  }
  return { entry, action: "none" };
};

export const orchestratorVisual = (
  status: SessionStatus,
  attention: boolean,
  subagents: SubagentRun[],
): StatusVisual | null => {
  if (status.state === "failed" || status.state === "awaiting_input") {
    return statusVisual(status, attention);
  }
  if (anyActive(subagents)) {
    return statusVisual({ state: "running" }, attention);
  }
  if (subagents.length > 0 && status.state !== "exited") {
    return statusVisual({ state: "idle", summary: null }, true);
  }
  return statusVisual(status, attention);
};

/**
 * O pedido de aprovação que a linha de cada subagente mostra: o MAIS ANTIGO
 * pendente dele, nesta sessão. Um por linha — o segundo pedido do mesmo
 * subagente segue no toast, que continua sendo um ponto de ação para ele.
 */
export function subagentInlineApprovals(
  sessionId: SessionId,
  subagents: SubagentRun[],
  approvals: readonly ApprovalRequest[],
): Map<string, ApprovalRequest> {
  const ids = new Set(
    subagents.map((s) => s.agent_id).filter((id): id is string => id != null),
  );
  const out = new Map<string, ApprovalRequest>();
  for (const request of approvals) {
    if (request.session_id !== sessionId) continue;
    const agent = request.agent_id;
    if (!agent || !ids.has(agent)) continue;
    const current = out.get(agent);
    if (
      !current ||
      request.requested_at_ms < current.requested_at_ms ||
      (request.requested_at_ms === current.requested_at_ms &&
        request.id < current.id)
    ) {
      out.set(agent, request);
    }
  }
  return out;
}

/**
 * Os pedidos que o painel de agentes torna ACIONÁVEIS (botões Aprovar/Recusar
 * na linha), para saírem do toast — acionável em um lugar só.
 *
 * Vermelho fica de fora: ele continua com a semântica do painel dentro da
 * sessão (e do toast que só aponta para lá). O painel de agentes mostra o
 * comando, mas não decide ação vermelha.
 */
export function agentsPanelActionableIds(
  inline: ReadonlyMap<string, ApprovalRequest>,
): Set<number> {
  const ids = new Set<number>();
  for (const request of inline.values()) {
    if (request.risk !== "red") ids.add(request.id);
  }
  return ids;
}

/** A segunda linha de um subagente: o que faz agora, ou como terminou. */
export type SubagentLine =
  | { kind: "run"; detail: string }
  | { kind: "tool"; tool: string; detail: string | null }
  | { kind: "result"; text: string }
  | { kind: "interrupted" }
  | null;

export function subagentLine(run: SubagentRun): SubagentLine {
  if (run.status === "done") {
    if (run.interrupted) return { kind: "interrupted" };
    const first = run.summary
      ?.split("\n")
      .map((line) => line.trim())
      .find((line) => line.length > 0);
    return first ? { kind: "result", text: first } : null;
  }
  const tool = run.activity_tool;
  if (!tool) return null;
  const detail = run.activity_detail?.trim() || null;
  if (tool === "Bash" && detail) return { kind: "run", detail };
  return { kind: "tool", tool, detail };
}
