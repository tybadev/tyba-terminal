import { useEffect, useState } from "react";

import { onSessionPorts, servicesSessionPorts, type SessionId } from "./ipc";

/**
 * As portas que os processos de cada sessão abriram (o chip da sidebar). O core
 * varre num poll lento e só emite quando muda; aqui só se espelha.
 */
export function useSessionPorts(): Record<SessionId, number[]> {
  const [ports, setPorts] = useState<Record<SessionId, number[]>>({});
  useEffect(() => {
    let alive = true;
    let off: (() => void) | null = null;
    void servicesSessionPorts()
      .then((p) => alive && setPorts(p))
      .catch(() => {});
    void onSessionPorts((p) => setPorts(p)).then((unlisten) => {
      if (alive) off = unlisten;
      else unlisten();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, []);
  return ports;
}
