import type {
  ApprovalRequest,
  DrawerTab,
  FileDiff,
  Session,
  SessionId,
  SubagentRun,
  TurnDigest,
} from "./ipc";
import { formatElapsed } from "./agentTree";
import type { PauseReason } from "./promptQueue";

/**
 * A gaveta Fila/Agentes do pane (quadros 4A′–4E de GavetaFilaAgentes). Abrir,
 * recolher e a aba escolhida pelo dono vêm do core (`DrawerState`); aqui só se
 * deriva o que a pílula e as abas mostram.
 */

export type RoundProgress = {
  /** Concluídos de verdade: interrompido e parado não contam. */
  done: number;
  total: number;
  /** A rodada não assentou (mesma regra do core: algum terminou e nenhum roda). */
  running: boolean;
  interrupted: number;
};

/** Concluídos / total da rodada atual (a mais nova). `null` sem subagentes. */
export function roundProgress(subagents: readonly SubagentRun[]): RoundProgress | null {
  if (subagents.length === 0) return null;
  const round = Math.max(...subagents.map((s) => s.round));
  let done = 0;
  let total = 0;
  let interrupted = 0;
  let anyDone = false;
  let anyRunning = false;
  for (const s of subagents) {
    if (s.round !== round) continue;
    total += 1;
    if (s.status === "running") anyRunning = true;
    if (s.status !== "done") continue;
    anyDone = true;
    if (s.interrupted) interrupted += 1;
    else if (!s.blocked) done += 1;
  }
  return { done, total, running: !(anyDone && !anyRunning), interrupted };
}

/** Pedidos pendentes dos subagentes desta sessão — vermelho incluso: ele
 *  também pede o dono, só não se decide na gaveta. */
export function pendingSubagentApprovals(
  sessionId: SessionId,
  subagents: readonly SubagentRun[],
  approvals: readonly ApprovalRequest[],
): number {
  const ids = new Set(subagents.map((s) => s.agent_id).filter(Boolean));
  return approvals.filter(
    (a) => a.session_id === sessionId && a.agent_id != null && ids.has(a.agent_id),
  ).length;
}

export type PillState =
  | { kind: "run" }
  | { kind: "ok" }
  | { kind: "attn"; count: number }
  | { kind: "fail"; count: number };

export type PillAgents = { done: number; total: number; state: PillState | null };

/**
 * A parte "Agentes x/y + estado" da pílula; `null` quando a sessão não tem
 * subagentes (a pílula é só "Fila · N"). Atenção vence tudo; o ✓ e o vermelho
 * do fim da rodada ficam até o dono abrir a gaveta.
 */
export function drawerPill(
  subagents: readonly SubagentRun[],
  pending: number,
  outcomeSeen: boolean,
): PillAgents | null {
  const progress = roundProgress(subagents);
  if (!progress) return null;
  const { done, total } = progress;
  if (pending > 0) return { done, total, state: { kind: "attn", count: pending } };
  if (progress.running) return { done, total, state: { kind: "run" } };
  if (outcomeSeen) return { done, total, state: null };
  if (progress.interrupted > 0) {
    return { done, total, state: { kind: "fail", count: progress.interrupted } };
  }
  return { done, total, state: { kind: "ok" } };
}

export type PillTurn = {
  outcome: "completed" | "failed" | "interrupted";
  /** `counts`: mexeu em arquivo. `none`: terminou sem mudar nada. `no_repo`:
   *  a pasta não é um repositório. `unavailable`: o core não conseguiu medir. */
  body:
    | { kind: "counts"; files: number; insertions: number; deletions: number }
    | { kind: "none" }
    | { kind: "no_repo" }
    | { kind: "unavailable" };
};

/**
 * O trecho "Turno" da pílula (quadro Pílulas de ResumoDoTurno): só com um
 * turno terminado e nenhum aberto. Os números são os totais do resumo, não o
 * tamanho de `files` (cortada pelo core).
 */
export function pillTurn(
  session: Pick<Session, "last_turn" | "turn_open"> | null | undefined,
): PillTurn | null {
  const turn = session?.last_turn;
  if (!turn || session?.turn_open) return null;
  const body: PillTurn["body"] =
    turn.diff !== "ok"
      ? { kind: turn.diff }
      : turn.files_total > 0
        ? {
            kind: "counts",
            files: turn.files_total,
            insertions: turn.insertions,
            deletions: turn.deletions,
          }
        : { kind: "none" };
  return { outcome: turn.outcome, body };
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** Título do trecho: "Turno", "Turno falhou", "Turno interrompido". */
export function pillTurnHead(turn: PillTurn, t: Translate): string {
  if (turn.outcome === "failed") return t("pillTurnFailed");
  if (turn.outcome === "interrupted") return t("pillTurnInterrupted");
  return t("pillTurn");
}

/**
 * O número da pílula compacta (ícone ± e arquivos): 0 sem mudança e sem git,
 * como no quadro; `null` quando o core não mediu, para não afirmar zero.
 */
export function pillTurnCount(turn: PillTurn): number | null {
  if (turn.body.kind === "unavailable") return null;
  return turn.body.kind === "counts" ? turn.body.files : 0;
}

/** O trecho por extenso, para o tooltip e o `aria-label` da pílula. */
export function pillTurnLabel(turn: PillTurn, t: Translate): string {
  const head = pillTurnHead(turn, t);
  const { body } = turn;
  if (body.kind === "counts") {
    const files = t("diffFilesSummary", { count: body.files });
    return `${head} · ${files} +${body.insertions} −${body.deletions}`;
  }
  if (body.kind === "none") return `${head} · ${t("pillTurnNoChanges")}`;
  if (body.kind === "no_repo") return `${head} · ${t("pillTurnNoGit")}`;
  return head;
}

/**
 * Qual aba a gaveta mostra (quadro 4C): a do dono, se ele escolheu (o core a
 * zera quando a gaveta recolhe) e ela existe; senão a que pede você
 * (aprovação pendente → Agentes), Agentes enquanto a rodada roda e Fila fora
 * dela. A aba Agentes só existe com subagentes; a Turno, depois do primeiro
 * fim de turno — e só entra por escolha: a gaveta aberta nunca troca para
 * ela sozinha (o dono pode estar escrevendo na Fila).
 */
export function drawerTab(input: {
  owner: DrawerTab | null;
  hasSubagents: boolean;
  pending: number;
  running: boolean;
  /** `Session.last_turn` da sessão da gaveta. */
  turn?: Pick<Session, "last_turn"> | null;
}): DrawerTab {
  const { owner } = input;
  if (
    owner === "queue" ||
    (owner === "agents" && input.hasSubagents) ||
    (owner === "turn" && Boolean(input.turn?.last_turn))
  ) {
    return owner;
  }
  return input.hasSubagents && (input.pending > 0 || input.running) ? "agents" : "queue";
}

/**
 * A aba em que o clique na pílula abre a gaveta (quadro T1 de ResumoDoTurno):
 * Turno depois de um turno terminado, a não ser que algo peça você — pedido
 * de subagente, rodada rodando ou fila pausada. `null`: a da regra.
 */
export function pillOpensTab(input: {
  hasSubagents: boolean;
  pending: number;
  running: boolean;
  queuePaused: boolean;
  turn: Pick<Session, "last_turn" | "turn_open"> | null | undefined;
}): DrawerTab | null {
  if (input.hasSubagents && (input.pending > 0 || input.running)) return null;
  if (input.queuePaused) return null;
  return input.turn?.last_turn && !input.turn.turn_open ? "turn" : null;
}

/** Linhas de arquivo da aba Turno; o resto vira "mais N arquivos". */
export const TURN_ROWS = 8;

export type TurnBand =
  | "failed"
  | "interrupted"
  | "changed"
  | "none"
  | "no_repo"
  | "unavailable";

/**
 * A faixa de estado da aba Turno (quadros T2 e T3): falha e interrupção
 * vencem; turno concluído diz o que o diff mediu.
 */
export function turnBand(turn: TurnDigest): TurnBand {
  if (turn.outcome === "failed") return "failed";
  if (turn.outcome === "interrupted") return "interrupted";
  if (turn.diff !== "ok") return turn.diff;
  return turn.files_total > 0 ? "changed" : "none";
}

/** As linhas da lista e quantos arquivos ficam fora dela. O total é o do
 *  resumo (`files_total`), não o tamanho de `files` (cortada pelo core). */
export function turnRows(turn: TurnDigest): { rows: FileDiff[]; more: number } {
  const rows = turn.files.slice(0, TURN_ROWS);
  return { rows, more: Math.max(0, turn.files_total - rows.length) };
}

/** A contagem da aba: arquivos do turno, ou `null` (traço) sem nenhum. */
export function turnTabCount(turn: TurnDigest): number | null {
  return turn.files_total > 0 ? turn.files_total : null;
}

/**
 * A segunda linha da aba: duração · tokens · custo · commits. Só o que o core
 * mandou: o Codex não reporta uso e fica só com a duração.
 */
export function turnMeta(turn: TurnDigest, locale: string, t: Translate): string {
  const parts = [formatElapsed(turn.duration_ms)];
  if (turn.tokens != null) {
    parts.push(
      t("agentsUsageTokens", {
        count: turn.tokens,
        tokens: new Intl.NumberFormat(locale, {
          notation: "compact",
          maximumFractionDigits: 1,
        }).format(turn.tokens),
      }),
    );
  }
  if (turn.cost_usd != null) {
    parts.push(
      new Intl.NumberFormat(locale, { style: "currency", currency: "USD" }).format(
        turn.cost_usd,
      ),
    );
  }
  if (turn.commits > 0) parts.push(t("turnCommits", { count: turn.commits }));
  return parts.join(" · ");
}

/** Há quanto tempo o turno terminou: segundos, depois minutos, depois horas. */
export function turnAgo(turn: TurnDigest, now: number, t: Translate): string {
  const seconds = Math.max(0, Math.floor((now - turn.ended_at_ms) / 1000));
  if (seconds < 60) return t("agentsAgo", { time: `${seconds}s` });
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return t("agentsRoundMinutes", { count: minutes });
  return t("agentsRoundHours", { count: Math.floor(minutes / 60) });
}

/** Caminho em pasta (com a barra) e nome, para apagar uma e destacar o outro. */
export function splitPath(path: string): { dir: string; name: string } {
  const cut = path.lastIndexOf("/") + 1;
  return { dir: path.slice(0, cut), name: path.slice(cut) };
}

/** A fila pausada por aprovação de subagente diz "ver em Agentes". */
export function queueGoesToAgents(paused: PauseReason | null, pending: number): boolean {
  return paused === "approval" && pending > 0;
}

/**
 * A sessão tem gaveta Fila/Agentes: a sessão de agente, ou o shell em que o
 * probe achou um claude cru. No split, a pílula do rodapé mostra a do pane
 * ativo (como o nome da sessão do rodapé) e some quando ele não tem gaveta.
 */
export function hasAgentDrawer(kind: Session["kind"]["type"], detected: boolean): boolean {
  return kind === "agent" || (kind === "shell" && detected);
}
