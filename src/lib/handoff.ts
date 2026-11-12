// Handoff entre agentes (quadros H1–H4 de HandoffAgentes): só o que a tela
// precisa para desenhar. O texto vem pronto e redigido do core; daqui nunca
// sai texto, só o agente escolhido e a nota (ver `handoff-agentes/security`).

import type { HandoffAgent, HandoffTarget, Session, SessionId, TurnDigest } from "./ipc";

/** A marca que `session::redact` põe no lugar do segredo. */
export const REDACTION_MARK = "[REDACTED]";

/** Como o core abre o bloco da nota no fim do texto. */
const NOTE_HEAD = "\nNota do dono (o que fazer agora):\n";

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
 * O texto sem o bloco da nota: na folha a nota é o campo editável logo
 * abaixo (quadro H2), não uma segunda cópia dentro do texto.
 */
export function previewBody(text: string, note: string): string {
  if (!note.trim()) return text.replace(/\n+$/, "");
  const at = text.lastIndexOf(NOTE_HEAD);
  return (at >= 0 ? text.slice(0, at) : text).replace(/\n+$/, "");
}

export type PreviewPart =
  | { kind: "text"; text: string }
  | { kind: "mark" }
  | { kind: "add"; text: string }
  | { kind: "del"; text: string };

export interface PreviewLine {
  /** Cabeçalho do texto ou título de seção ("Pedido:"): em destaque. */
  head: boolean;
  parts: PreviewPart[];
}

const COUNTS = /^(- .* )(\+\d+) (−\d+)$/;

function withMarks(text: string): PreviewPart[] {
  const parts: PreviewPart[] = [];
  text.split(REDACTION_MARK).forEach((piece, i) => {
    if (i > 0) parts.push({ kind: "mark" });
    if (piece) parts.push({ kind: "text", text: piece });
  });
  return parts;
}

/** Linhas do texto para desenhar: títulos, marcas de redação e o +/− dos arquivos. */
export function previewLines(body: string): PreviewLine[] {
  const lines = body.split("\n");
  return lines.map((line, i) => {
    const head = i === 0 || (lines[i - 1] === "" && line.endsWith(":"));
    const counts = COUNTS.exec(line);
    if (counts) {
      return {
        head: false,
        parts: [
          ...withMarks(counts[1]),
          { kind: "add", text: counts[2] },
          { kind: "text", text: " " },
          { kind: "del", text: counts[3] },
        ],
      };
    }
    return { head, parts: withMarks(line) };
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
