export const AGENT_READY_TIMEOUT_MS = 20_000;

export interface AgentReadySchedulerDeps {
  onReady: (handler: () => void) => () => void;
  paste: (submit: boolean) => void;
  onTimeout: () => void;
  setTimeout: (cb: () => void, ms: number) => number;
  clearTimeout: (handle: number) => void;
  timeoutMs?: number;
}

export function scheduleAgentReadyPrompt(
  deps: AgentReadySchedulerDeps,
): () => void {
  const timeoutMs = deps.timeoutMs ?? AGENT_READY_TIMEOUT_MS;
  let settled = false;
  let unsubscribe: () => void = () => {};

  unsubscribe = deps.onReady(() => {
    if (settled) return;
    settled = true;
    deps.clearTimeout(handle);
    unsubscribe();
    deps.paste(true);
  });

  const handle = deps.setTimeout(() => {
    if (settled) return;
    settled = true;
    unsubscribe();
    deps.paste(false);
    deps.onTimeout();
  }, timeoutMs);

  return () => {
    if (settled) return;
    settled = true;
    deps.clearTimeout(handle);
    unsubscribe();
  };
}
