// Handoff entre agentes (quadros H1–H4 de HandoffAgentes): só o que a tela
// precisa para desenhar. O texto vem pronto e redigido do core; daqui nunca
// sai texto, só o agente escolhido e a nota (ver `handoff-agentes/security`).

import type {
  AgentRunner,
  HandoffAgent,
  HandoffTarget,
  Session,
  SessionId,
  TurnDigest,
} from "./ipc";

/** A marca que `session::redact` põe no lugar do segredo. */
export const REDACTION_MARK = "[REDACTED]";

/**
 * O agente de origem como o dono vê, se dá para passar o trabalho dele:
 * só claude e codex. Runner custom (ou sessão sem agente) fica de fora.
 */
export function handoffSource(runner: AgentRunner | null | undefined): string | null {
  if (runner === "claude_code") return "claude";
  if (runner === "codex") return "codex";
  return null;
}

/**
 * O handoff só sai de turno fechado: há resumo e nenhum turno aberto. Com o
 * agente trabalhando, o botão e o item da paleta ficam desligados.
 */
export function handoffReady(
  session: Pick<Session, "last_turn" | "turn_open"> | null | undefined,
): boolean {
  return Boolean(session?.last_turn) && !session?.turn_open;
}

/** O agente que a folha marca ao abrir: o primeiro disponível. */
export function defaultTarget(targets: HandoffTarget[]): HandoffAgent | null {
  return targets.find((t) => t.available)?.agent ?? null;
}

/**
 * A nota como vai, quando difere do que o dono digitou (redação, corte em
 * 2000, invisíveis): a folha mostra "Vai assim:" com ela. `asked` é a nota
 * com que o core montou esse texto; enquanto o dono digita e o texto novo não
 * chegou, não há o que comparar.
 */
export function noteAsSent(
  previewNote: string | null | undefined,
  asked: string | null,
  note: string,
): string | null {
  if (previewNote == null || asked !== note) return null;
  return previewNote === note.trim() ? null : previewNote;
}

export type PreviewPart =
  | { kind: "text"; text: string }
  | { kind: "mark" }
  | { kind: "add"; text: string }
  | { kind: "del"; text: string };

export interface PreviewLine {
  /** Cabeçalho do texto ou título de seção ("Pedido:"): em destaque. */
  head: boolean;
  /** Fala citada do agente ("> …"): apagada, nunca título. */
  quote: boolean;
  parts: PreviewPart[];
}

const COUNTS = /^(- .* )(\+\d+) (−\d+)$/;

/** O texto em peças, com a marca de redação como peça própria. */
export function withMarks(text: string): PreviewPart[] {
  const parts: PreviewPart[] = [];
  text.split(REDACTION_MARK).forEach((piece, i) => {
    if (i > 0) parts.push({ kind: "mark" });
    if (piece) parts.push({ kind: "text", text: piece });
  });
  return parts;
}

/**
 * Linhas do corpo (`preview.body`, sem a nota) para desenhar: títulos, fala
 * citada, marcas de redação e o +/− dos arquivos.
 */
export function previewLines(body: string): PreviewLine[] {
  const lines = body.replace(/\n+$/, "").split("\n");
  return lines.map((line, i) => {
    const quote = line.startsWith(">");
    const head = !quote && (i === 0 || (lines[i - 1] === "" && line.endsWith(":")));
    const counts = quote ? null : COUNTS.exec(line);
    if (counts) {
      return {
        head: false,
        quote: false,
        parts: [
          ...withMarks(counts[1]),
          { kind: "add", text: counts[2] },
          { kind: "text", text: " " },
          { kind: "del", text: counts[3] },
        ],
      };
    }
    return { head, quote, parts: withMarks(line) };
  });
}

/* ---------- quem abre a folha: a aba Turno (H1) e a paleta (H4) ---------- */

export interface HandoffRequest {
  sessionId: SessionId;
  /** O agente de origem como o dono vê ("claude", "codex"). */
  from: string;
  turn: TurnDigest;
}

type Listener = (request: HandoffRequest | null) => void;

let current: HandoffRequest | null = null;
const listeners = new Set<Listener>();

function emit() {
  for (const listener of listeners) listener(current);
}

export function subscribeHandoff(listener: Listener): () => void {
  listeners.add(listener);
  listener(current);
  return () => {
    listeners.delete(listener);
  };
}

export function requestHandoff(request: HandoffRequest) {
  current = request;
  emit();
}

export function closeHandoff() {
  current = null;
  emit();
}
