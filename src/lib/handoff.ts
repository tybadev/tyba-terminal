import type { AgentRunner, SessionId } from "./ipc";

/** "Passar para…" da aba Turno (proposta 11). Quem abre o agente novo é o
 * App, que tem a lista de sessões e os workspaces; a aba só pede. */
type HandOff = (from: SessionId, runner: AgentRunner) => Promise<void>;

let current: HandOff | null = null;

export function setHandOff(fn: HandOff | null): void {
  current = fn;
}

export function handOff(from: SessionId, runner: AgentRunner): Promise<void> {
  return current ? current(from, runner) : Promise.reject(new Error("handoff indisponível"));
}
