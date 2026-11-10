// Fila de prompts (quadros 4A/4B, SshVariaveisFila): o que a gaveta mostra a
// partir do estado que o core manda em `prompt-queue://changed`. Nada aqui
// decide quando um prompt sai; isso é do core (`agent/prompt_queue.rs`).

import type { SessionStatus } from "./ipc";

/** Espelha `PauseReason` do core. */
export type PauseReason =
  | "user"
  | "failed"
  | "interrupted"
  | "approval"
  | "agent_ended"
  | "session_ended"
  | "send_failed";

export interface QueuedPrompt {
  id: string;
  text: string;
}

export interface QueueState {
  session_id: string;
  prompts: QueuedPrompt[];
  paused: PauseReason | null;
}

/** As cores do indicador: verde envia, âmbar aprovação, vermelho falha,
 * cinza parada por alguém (ou porque o agente saiu). */
export type QueueTone = "send" | "approval" | "failed" | "stopped";

export interface QueueIndicator {
  tone: QueueTone;
  /** Chave i18n do rótulo. */
  label: string;
  /** O porquê à direita: uma chave i18n com parâmetros, ou o texto que o
   * próprio status da sessão já traz (o pedido de aprovação, o erro). */
  detail: { key: string; params?: Record<string, unknown> } | { text: string } | null;
}

const PAUSED: Record<PauseReason, { tone: QueueTone; label: string }> = {
  approval: { tone: "approval", label: "promptQueuePausedApproval" },
  failed: { tone: "failed", label: "promptQueuePausedFailed" },
  send_failed: { tone: "failed", label: "promptQueuePausedSendFailed" },
  interrupted: { tone: "stopped", label: "promptQueuePausedInterrupted" },
  user: { tone: "stopped", label: "promptQueuePausedUser" },
  agent_ended: { tone: "stopped", label: "promptQueuePausedAgentEnded" },
  session_ended: { tone: "stopped", label: "promptQueuePausedSessionEnded" },
};

export function queueIndicator(
  state: Pick<QueueState, "prompts" | "paused">,
  status: SessionStatus,
): QueueIndicator {
  if (state.paused === null) {
    return {
      tone: "send",
      label: "promptQueueSends",
      detail:
        state.prompts.length > 0
          ? { key: "promptQueueNext", params: { n: 1 } }
          : null,
    };
  }
  const { tone, label } = PAUSED[state.paused];
  let text: string | null = null;
  if (state.paused === "approval" && status.state === "awaiting_input") {
    text = status.hint;
  } else if (state.paused === "failed" && status.state === "idle") {
    text = status.summary;
  }
  return { tone, label, detail: text ? { text } : null };
}

/**
 * A ordem nova depois de soltar o item `from` na fenda `slot` (0 = antes do
 * primeiro, `ids.length` = depois do último). `null` quando nada muda.
 */
export function reorderIds(
  ids: readonly string[],
  from: number,
  slot: number,
): string[] | null {
  if (from < 0 || from >= ids.length || slot < 0 || slot > ids.length) {
    return null;
  }
  if (slot === from || slot === from + 1) return null;
  const next = ids.slice();
  const [moved] = next.splice(from, 1);
  next.splice(slot > from ? slot - 1 : slot, 0, moved);
  return next;
}

/** A fenda sob o ponteiro: quantos itens têm o meio acima de `y`. */
export function slotAt(y: number, mids: readonly number[]): number {
  let slot = 0;
  while (slot < mids.length && mids[slot] < y) slot += 1;
  return slot;
}
