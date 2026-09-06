import type {
  ApprovalRequest,
  LayoutState,
  ObservedAgent,
  ObservedState,
  PaneId,
  PaneNode,
  Session,
  SessionId,
  SessionStatus,
  TabId,
  Workspace,
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
  /**
   * Posição desta sessão na varredura do layout — abas na ordem da barra,
   * painéis na ordem da árvore.
   *
   * Existe para desempatar linhas que ficam idênticas: duas sessões no mesmo
   * workspace, com o mesmo agente, produzem duas linhas iguais na lista. Nome
   * de aba não resolve, porque duas abas da mesma pasta nascem com o mesmo
   * nome. O que sobra é a posição — e ela precisa vir do LAYOUT, nunca da
   * ordem da lista: aquela é por urgência e mudaria de lugar a cada turno do
   * agente, trocando o rótulo de uma linha parada.
   */
  order: number;
}

export interface AgentRow {
  session: Session;
  place: SessionPlace;
  visual: StatusVisual;
  /** Ordena o quadro, maior primeiro. Ver [`urgencyOf`]. */
  urgency: number;
  /**
   * O agente deduzido da tela, e `null` na seção dos gerenciados.
   *
   * Sai daqui, e não de `session.observed`, porque é este campo que decide o
   * selo "sem gate" na pintura: uma sessão gerenciada que por acidente carregue
   * `observed` continua sendo gerenciada, e ler o campo da sessão colaria nela
   * um selo que mente sobre as garantias que ela tem.
   */
  observed: ObservedAgent | null;
}

/** Estado sem cor própria no `statusVisual`: idle já visto e saída limpa. */
const RESTING: StatusVisual = {
  dotClass: "bg-tyba-bg ring-1 ring-inset ring-tyba-text-faint",
  textClass: "text-tyba-text-muted",
  labelKey: "sessionIdle",
  rank: 0,
};

/**
 * Presença sem estado: há um agente ali e o sinal não diz o que ele faz.
 *
 * Sem ponto colorido de propósito — cor é afirmação, e aqui não há o que
 * afirmar. Fingir um estado é pior que admitir que não se sabe, porque o ponto
 * âmbar de "esperando" e o azul de "trabalhando" mandam o usuário para lados
 * opostos.
 */
export const NO_SIGNAL: StatusVisual = {
  dotClass: "bg-tyba-bg ring-1 ring-inset ring-tyba-text-faint",
  textClass: "text-tyba-text-faint",
  labelKey: "agentsBoardNoSignal",
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

/** Sessão que o TYBA lançou: tem hook, gate de aprovação, inbox e jaula. */
const isAgent = (session: Session): boolean => session.kind.type === "agent";

/**
 * O estado deduzido dito na língua do `statusVisual`, para não abrir uma
 * segunda paleta que sairia do lugar na primeira vez que a primeira mudasse.
 *
 * `awaiting_input` vira `reason: "reply"` porque sem gate não existe aprovação
 * pendente: o que a tela pode sugerir é um agente parado esperando alguém, e é
 * isso que o rótulo "aguardando" diz.
 *
 * `null` para o que este front não sabe ler — incluindo o `state` ausente e o
 * estado que o core acrescente depois. A união do TypeScript vale na compilação
 * e o dado vem do core em tempo de execução: um `default` que confia no tipo
 * devolveria `undefined`, e o `statusVisual` estouraria lendo `status.state`.
 * Como o `buildRows` roda dentro de `useMemo`, o estouro não derruba a linha —
 * derruba a renderização inteira, e um campo novo do core viraria tela branca
 * sem release nenhum do front.
 */
const observedStatus = (state: ObservedState | null): SessionStatus | null => {
  switch (state) {
    case "awaiting_input":
      return { state: "awaiting_input", hint: null, reason: "reply" };
    case "running":
      return { state: "running" };
    case "idle":
      return { state: "idle", summary: null };
    default:
      return null;
  }
};

/**
 * O visual de uma linha observada.
 *
 * `idle` cai no `RESTING` porque o `statusVisual` só pinta idle quando há
 * `attention` — e `attention` é a marca de fim de turno que o hook levanta.
 * Sessão sem hook nunca a tem, então idle deduzido é repouso, não conclusão a
 * revisar.
 *
 * Sinal que este front não sabe ler cai no `NO_SIGNAL`, no mesmo lugar da
 * presença sem estado: há um agente ali e o que chegou não diz o que ele faz.
 * É a leitura honesta, e a única que não escolhe uma cor no chute.
 */
export const observedVisual = (observed: ObservedAgent): StatusVisual => {
  const status = observedStatus(observed.state);
  if (!status) return NO_SIGNAL;
  return statusVisual(status, false) ?? RESTING;
};

/**
 * Urgência de linha observada, na mesma escala das gerenciadas — o que permite
 * ao `wantsAttention` valer para as duas sem uma segunda régua.
 *
 * Sem o desempate de aprovação do [`urgencyOf`]: aprovação é coisa de quem tem
 * gate, e essas não têm.
 */
export const observedUrgency = (observed: ObservedAgent): number =>
  observedVisual(observed).rank * 10;

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
  let order = 0;
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
          order: order++,
        });
      }
    }
  }
  return found;
};

/**
 * As duas coleções do quadro, cada uma ordenada por dentro.
 *
 * Separadas na estrutura, e não só na pintura, porque é o que garante que a
 * urgência nunca cruze a fronteira: um agente sem gate, por mais aflito que a
 * tela o faça parecer, não empurra um gerenciado para baixo. Junta-las num
 * array só e ordenar deixaria isso na mão de quem lê a lista.
 */
export interface BoardSections {
  /** Sessões que o TYBA lançou: hook, gate de aprovação, inbox e jaula. */
  managed: AgentRow[];
  /**
   * Agente deduzido da tela em sessão que o TYBA não lançou — sem gate, sem
   * inbox e sem jaula. Mostrado com selo próprio, nunca com a mesma cara das
   * gerenciadas: isso afirmaria garantias que a linha não tem.
   */
  observed: AgentRow[];
}

const byUrgencyThenName = (a: AgentRow, b: AgentRow): number =>
  b.urgency - a.urgency ||
  a.session.title.localeCompare(b.session.title) ||
  a.session.id.localeCompare(b.session.id);

/**
 * As linhas do quadro, ordenadas por urgência e, no empate, por nome.
 *
 * Sessão sem lugar no layout fica de fora: ela existe no core mas não tem para
 * onde saltar, e uma linha que não leva a lugar nenhum é pior do que a ausência
 * dela.
 *
 * A sessão gerenciada é decidida primeiro e sai da varredura: se um dia uma
 * delas carregar `observed` — palpite de tela sobre sessão que tem hook —, ela
 * conta uma vez só, entre as gerenciadas, que é onde estão as garantias.
 */
export const buildRows = (
  sessions: Session[],
  layout: LayoutState,
): BoardSections => {
  const places = placesBySession(layout);
  const managed: AgentRow[] = [];
  const observed: AgentRow[] = [];
  for (const session of sessions) {
    const place = places.get(session.id);
    if (!place) continue;
    if (isAgent(session)) {
      managed.push({
        session,
        place,
        visual: statusVisual(session.status, session.attention) ?? RESTING,
        urgency: urgencyOf(session),
        observed: null,
      });
      continue;
    }
    const seen = session.observed;
    if (!seen) continue;
    observed.push({
      session,
      place,
      // Do palpite de tela, nunca do `session.status`: aquele é o estado do
      // shell — que está sempre "rodando" — e pintaria de azul um agente que
      // pode estar parado esperando resposta.
      visual: observedVisual(seen),
      urgency: observedUrgency(seen),
      observed: seen,
    });
  }
  managed.sort(byUrgencyThenName);
  observed.sort(byUrgencyThenName);
  return { managed, observed };
};

/**
 * As duas seções numa lista só, gerenciadas primeiro — a ordem que o ciclo do
 * "ir para o próximo" percorre e que o badge conta.
 *
 * **Não é a ordem da tela.** Lá as linhas ainda passam pelo
 * [`groupByWorkspace`], que junta por workspace; aqui a urgência é plana, então
 * com mais de um workspace o ciclo visita numa sequência que a tela não mostra.
 * O que as duas ordens têm em comum — e é o que a fronteira entre as seções
 * existe para garantir — é que nenhum agente sem gate é visitado enquanto um
 * gerenciado ainda pede alguém.
 */
export const boardOrder = (sections: BoardSections): AgentRow[] => [
  ...sections.managed,
  ...sections.observed,
];

/**
 * O agente que representa o workspace na barra lateral.
 *
 * Nasceu como `useCallback` dentro do `App` e olhava só sessão de agente
 * lançada pelo TYBA — por isso um `claude` digitado num shell não aparecia em
 * lugar nenhum da lista que está sempre na tela, e para vê-lo era preciso
 * navegar até uma página. Era o contrário do que a feature promete: quem tem de
 * caçar não foi avisado.
 *
 * **Gerenciado sempre vence observado**, mesmo que o observado pareça mais
 * urgente. É a mesma regra que separa as seções do quadro: um palpite de tela
 * não empurra para baixo um agente que tem hook, gate e jaula.
 */
export interface WorkspaceAgent {
  session: Session;
  visual: StatusVisual;
  /** Veio da tela: sem gate, sem inbox, sem jaula. */
  observed: ObservedAgent | null;
}

export const agentForWorkspace = (
  workspace: Workspace,
  sessionById: Map<SessionId, Session>,
): WorkspaceAgent | null => {
  let best: (WorkspaceAgent & { urgency: number }) | null = null;
  for (const tab of workspace.tabs) {
    if (!tab.root) continue;
    for (const [sessionId] of placesIn(tab.root)) {
      const session = sessionById.get(sessionId);
      if (!session) continue;

      let candidate: (WorkspaceAgent & { urgency: number }) | null = null;
      if (session.kind.type === "agent") {
        const visual = statusVisual(session.status, session.attention);
        if (visual) {
          // Gerenciado entra num degrau acima de qualquer observado: somar 100
          // é o que garante que a comparação de urgência nunca inverta a
          // precedência, por mais urgente que o palpite pareça.
          candidate = {
            session,
            visual,
            observed: null,
            urgency: urgencyOf(session) + 100,
          };
        }
      } else if (session.observed) {
        candidate = {
          session,
          visual: observedVisual(session.observed),
          observed: session.observed,
          urgency: observedUrgency(session.observed),
        };
      }

      if (candidate && (!best || candidate.urgency > best.urgency)) {
        best = candidate;
      }
    }
  }
  if (!best) return null;
  const { session, visual, observed } = best;
  return { session, visual, observed };
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

/**
 * Linha que está pedindo alguém: bloqueada, falha, ou concluída sem revisão.
 *
 * Vale também para a linha observada, e é de propósito: o badge do sidebar é o
 * canal mais barato que existe — um número que custa uma olhada quando erra —,
 * enquanto a notificação tem guardas próprias justamente porque interrompe. Um
 * agente sem gate parado esperando é o caso em que ninguém mais avisa: não há
 * inbox, não há aprovação, não há hook. Deixá-lo fora do badge seria escolher o
 * silêncio para a única sessão que não tem outro canal.
 *
 * `state: null` não entra sozinho: presença sem estado não afirma que alguém
 * está esperando, e vale urgência 0.
 */
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

/**
 * O pedido que a fila mostra por sessão, quando ela tem mais de um pendente.
 *
 * O mais antigo primeiro: se uma sessão acumula dois pedidos, o que está
 * esperando há mais tempo é o que a linha da fila mostra — é a mesma regra
 * que a fila (`AgentsQueue`) já aplicava inline; sai daqui para não haver
 * uma segunda cópia da regra em outro lugar, com risco de as duas
 * divergirem.
 */
export const oldestApprovalBySession = (
  approvals: ApprovalRequest[],
): Map<SessionId, ApprovalRequest> => {
  const bySession = new Map<SessionId, ApprovalRequest>();
  for (const approval of [...approvals].sort(
    (a, b) => a.requested_at_ms - b.requested_at_ms,
  )) {
    if (!bySession.has(approval.session_id)) {
      bySession.set(approval.session_id, approval);
    }
  }
  return bySession;
};

/**
 * Os ids de aprovação que a fila de agentes torna acionáveis ao vivo — nunca
 * todos os pendentes.
 *
 * A fila colapsa pra um pedido por sessão (o mais antigo) e só mostra
 * sessões com linha no quadro (`rows`, já filtradas por lugar no layout).
 * Uma sessão com dois pedidos pendentes deixa o segundo de fora — e é
 * exatamente por isso que o toast dele não pode ser escondido junto: ele
 * ficaria sem NENHUM ponto de ação visível enquanto a fila está aberta.
 */
export const agentQueueVisibleApprovalIds = (
  rows: AgentRow[],
  approvals: ApprovalRequest[],
): Set<number> => {
  const bySession = oldestApprovalBySession(approvals);
  const ids = new Set<number>();
  for (const row of rows) {
    const approval = bySession.get(row.session.id);
    if (approval) ids.add(approval.id);
  }
  return ids;
};
