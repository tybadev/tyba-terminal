import type {
  LayoutState,
  PaneId,
  PaneNode,
  Session,
  SessionId,
  TabId,
  WorkspaceId,
} from "./ipc";
import { statusVisual, type StatusVisual } from "./sessionStatus";

/**
 * Onde a sessão mora no layout — o caminho inteiro que o salto percorre.
 *
 * Guardado por linha porque saltar exige os três: ativar o workspace, ativar a
 * aba e focar o painel. Só o `session_id` não basta: a mesma sessão não aparece
 * duas vezes, mas descobrir onde ela está custaria varrer a árvore de novo a
 * cada clique.
 */
export interface SessionPlace {
  workspaceId: WorkspaceId;
  workspaceName: string;
  workspaceColor: string | null;
  tabId: TabId;
  paneId: PaneId;
}

export interface AgentRow {
  session: Session;
  place: SessionPlace;
  visual: StatusVisual;
  /** Ordena o quadro, maior primeiro. Ver [`urgencyOf`]. */
  urgency: number;
}

/** Estado sem cor própria no `statusVisual`: idle já visto e saída limpa. */
const RESTING: StatusVisual = {
  dotClass: "bg-tyba-bg ring-1 ring-inset ring-tyba-text-faint",
  textClass: "text-tyba-text-muted",
  labelKey: "sessionIdle",
  rank: 0,
};

/**
 * Urgência derivada do `rank` que o sidebar já usa, nunca de uma escala nova.
 *
 * O `statusVisual` empata bloqueado-por-aprovação com bloqueado-por-resposta no
 * rank 3, e no sidebar isso não importa porque a linha mostra o motivo. Aqui o
 * quadro ordena, e aprovação vem primeiro: é a única das duas em que o agente
 * está parado esperando uma decisão de risco, não uma pergunta.
 *
 * Multiplica por 10 para o desempate caber sem invadir o degrau seguinte.
 */
export const urgencyOf = (session: Session): number => {
  const visual = statusVisual(session.status, session.attention);
  if (!visual) return 0;
  const tiebreak =
    session.status.state === "awaiting_input" &&
    session.status.reason === "approval"
      ? 1
      : 0;
  return visual.rank * 10 + tiebreak;
};

const isAgent = (session: Session): boolean =>
  session.kind.type === "agent" ||
  // Sessão de shell em que um agente foi detectado ainda não entra: sem hook
  // não há gate, e mostrá-la aqui com a mesma cara das outras diria que ela tem
  // as mesmas garantias. Entra com a detecção por manifesto, com selo próprio.
  false;

/** Todos os painéis de uma aba, com o id do painel junto do id da sessão. */
const placesIn = (node: PaneNode | null): Array<[SessionId, PaneId]> => {
  if (!node) return [];
  if (node.type === "leaf") return [[node.session_id, node.id]];
  // O visualizador de subagente espelha uma sessão que já tem painel próprio;
  // contá-lo duplicaria a linha no quadro.
  if (node.type === "agentviewer") return [];
  return [...placesIn(node.first), ...placesIn(node.second)];
};

/**
 * Onde cada sessão está, varrendo o layout uma vez só.
 *
 * Uma sessão aparece em um painel só, então o primeiro encontrado ganha: se um
 * dia deixar de ser verdade, o quadro mostra a primeira ocorrência em vez de
 * duplicar a linha.
 */
export const placesBySession = (
  layout: LayoutState,
): Map<SessionId, SessionPlace> => {
  const found = new Map<SessionId, SessionPlace>();
  for (const workspace of layout.workspaces) {
    for (const tab of workspace.tabs) {
      for (const [sessionId, paneId] of placesIn(tab.root)) {
        if (found.has(sessionId)) continue;
        found.set(sessionId, {
          workspaceId: workspace.id,
          workspaceName: workspace.name,
          workspaceColor: workspace.color,
          tabId: tab.id,
          paneId,
        });
      }
    }
  }
  return found;
};

/**
 * As linhas do quadro, ordenadas por urgência e, no empate, por nome.
 *
 * Sessão de agente sem lugar no layout fica de fora: ela existe no core mas não
 * tem para onde saltar, e uma linha que não leva a lugar nenhum é pior do que a
 * ausência dela.
 */
export const buildRows = (
  sessions: Session[],
  layout: LayoutState,
): AgentRow[] => {
  const places = placesBySession(layout);
  const rows: AgentRow[] = [];
  for (const session of sessions) {
    if (!isAgent(session)) continue;
    const place = places.get(session.id);
    if (!place) continue;
    rows.push({
      session,
      place,
      visual: statusVisual(session.status, session.attention) ?? RESTING,
      urgency: urgencyOf(session),
    });
  }
  rows.sort(
    (a, b) =>
      b.urgency - a.urgency ||
      a.session.title.localeCompare(b.session.title) ||
      a.session.id.localeCompare(b.session.id),
  );
  return rows;
};

export interface WorkspaceGroup {
  workspaceId: WorkspaceId;
  workspaceName: string;
  workspaceColor: string | null;
  rows: AgentRow[];
  /** A urgência da linha mais urgente — é ela que o cabeçalho do grupo mostra. */
  urgency: number;
}

/**
 * Agrupa por workspace com rollup, como o sidebar do herdr: o estado do grupo é
 * o do pior filho, e não a média nem a contagem. Um agente bloqueado num
 * workspace de dez torna o workspace bloqueado, porque é isso que decide para
 * onde o usuário olha primeiro.
 */
export const groupByWorkspace = (rows: AgentRow[]): WorkspaceGroup[] => {
  const groups = new Map<WorkspaceId, WorkspaceGroup>();
  for (const row of rows) {
    const existing = groups.get(row.place.workspaceId);
    if (existing) {
      existing.rows.push(row);
      existing.urgency = Math.max(existing.urgency, row.urgency);
      continue;
    }
    groups.set(row.place.workspaceId, {
      workspaceId: row.place.workspaceId,
      workspaceName: row.place.workspaceName,
      workspaceColor: row.place.workspaceColor,
      rows: [row],
      urgency: row.urgency,
    });
  }
  return [...groups.values()].sort(
    (a, b) =>
      b.urgency - a.urgency || a.workspaceName.localeCompare(b.workspaceName),
  );
};

/** Linha que está pedindo alguém: bloqueada, falha, ou concluída sem revisão. */
export const wantsAttention = (row: AgentRow): boolean => row.urgency >= 20;

/**
 * A próxima linha a visitar a partir da atual, em ciclo.
 *
 * Ciclo, e não "a mais urgente": com duas sessões bloqueadas, saltar sempre para
 * a mais urgente prenderia o atalho na primeira e a segunda nunca seria
 * alcançada. Partindo da atual e dando a volta, apertar de novo anda.
 */
export const nextAttention = (
  rows: AgentRow[],
  current: SessionId | null,
): AgentRow | null => {
  const wanting = rows.filter(wantsAttention);
  if (wanting.length === 0) return null;
  const at = wanting.findIndex((row) => row.session.id === current);
  if (at === -1) return wanting[0];
  return wanting[(at + 1) % wanting.length];
};
