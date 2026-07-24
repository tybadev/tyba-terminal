import type { AgentRunner, DetectedAgent, SessionKind } from "./ipc";

export const noticeKey = (detected: DetectedAgent): string =>
  `${detected.pid}:${detected.start_ms}`;

export const agentBinaryName = (kind: AgentRunner): string => {
  if (kind === "claude_code") return "claude";
  if (kind === "codex") return "codex";
  return typeof kind === "object" ? kind.custom : String(kind);
};

export const showShellAgentNotice = (
  kind: SessionKind,
  detected: DetectedAgent | null | undefined,
  dismissedKey: string | undefined,
): boolean =>
  kind.type === "shell" &&
  detected != null &&
  dismissedKey !== noticeKey(detected);
