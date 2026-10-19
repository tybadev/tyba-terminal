import type { AgentRunner } from "./ipc";

export const agentBinaryName = (kind: AgentRunner): string => {
  if (kind === "claude_code") return "claude";
  if (kind === "codex") return "codex";
  return typeof kind === "object" ? kind.custom : String(kind);
};
