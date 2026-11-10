import { useEffect, useState } from "react";

import { onPromptQueueChanged, promptQueueList, type SessionId } from "../lib/ipc";
import type { QueueState } from "../lib/promptQueue";

/**
 * A fila de prompts da sessão como o core a manda (`prompt-queue://changed`).
 * A gaveta e a pílula do rodapé leem daqui; `null` sem sessão ou antes da
 * primeira resposta.
 */
export function usePromptQueue(sessionId: SessionId | null): QueueState | null {
  const [queue, setQueue] = useState<QueueState | null>(null);
  useEffect(() => {
    setQueue(null);
    if (!sessionId) return;
    let alive = true;
    const unlisten = onPromptQueueChanged((state) => {
      if (state.session_id === sessionId) setQueue(state);
    });
    void promptQueueList(sessionId)
      .then((state) => alive && setQueue(state))
      .catch(() => {});
    return () => {
      alive = false;
      void unlisten.then((f) => f());
    };
  }, [sessionId]);
  return queue;
}
