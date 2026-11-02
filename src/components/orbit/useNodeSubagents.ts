import { useEffect, useMemo, useState } from "react";

import { listSubagents, onSubagentsChanged, type SubagentRun } from "@/lib/ipc";

/**
 * Subagentes de cada nó de agente com sessão viva: lê o retrato na largada e
 * segue o `subagents://changed` daquelas sessões. Devolve por id de nó.
 * `sessions` é nó → sessão.
 */
export function useNodeSubagents(
  sessions: ReadonlyMap<string, string>,
): ReadonlyMap<string, SubagentRun[]> {
  const [bySession, setBySession] = useState<ReadonlyMap<string, SubagentRun[]>>(new Map());
  const key = [...sessions.values()].sort().join(",");

  useEffect(() => {
    if (!key) {
      setBySession(new Map());
      return;
    }
    const ids = new Set(key.split(","));
    let alive = true;
    let unlisten: (() => void) | null = null;
    setBySession((prev) => new Map([...prev].filter(([id]) => ids.has(id))));
    void onSubagentsChanged((p) => {
      if (!alive || !ids.has(p.session_id)) return;
      setBySession((prev) => new Map(prev).set(p.session_id, p.subagents));
    })
      .then((un) => {
        if (!alive) return un();
        unlisten = un;
        // Depois de assinar, para não perder o que mudou antes; o evento
        // que chegar primeiro vence, porque o retrato só entra se faltar.
        for (const id of ids) {
          void listSubagents(id)
            .then((snap) => {
              if (alive)
                setBySession((prev) =>
                  prev.has(id) ? prev : new Map(prev).set(id, snap.subagents),
                );
            })
            .catch(() => {});
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [key]);

  return useMemo(() => {
    const out = new Map<string, SubagentRun[]>();
    for (const [nodeId, sessionId] of sessions) {
      const runs = bySession.get(sessionId);
      if (runs && runs.length > 0) out.set(nodeId, runs);
    }
    return out;
  }, [sessions, bySession]);
}
