import type { AgentRepoConfig, CreateSessionOpts } from "./ipc";

export function needsConsentPrompt(config: AgentRepoConfig | null): boolean {
  return config !== null && config.consent !== true;
}

export type ConsentDecision = "allow" | "skip";

export interface ConsentOutcome {
  persist: boolean;
  config: AgentRepoConfig;
}

export function applyConsentDecision(
  config: AgentRepoConfig,
  decision: ConsentDecision,
): ConsentOutcome {
  if (decision === "allow") {
    return { persist: true, config: { ...config, consent: true } };
  }
  return { persist: false, config: { ...config, consent: false } };
}

export type AgentRunnerId = "claude_code" | "codex";

export function runnerFromDefault(
  value: string | null | undefined,
): AgentRunnerId | null {
  if (value === "codex") return "codex";
  if (value === "claude" || value === "claude_code") return "claude_code";
  return null;
}

/** A preferência de review é uma linha de comando (`claude`, `codex --flag`), mas
 * uma sessão de agente exige o runner do core — é ele que carrega hooks, sandbox
 * e env allowlist. Casa pelo binário, ignorando argumentos; o que não for
 * reconhecido cai no Claude Code, que é o default do produto. */
export function runnerFromCommand(
  command: string | null | undefined,
): AgentRunnerId {
  const binary = (command ?? "").trim().split(/\s+/)[0] ?? "";
  const name = binary.split("/").pop()?.toLowerCase() ?? "";
  return name === "codex" ? "codex" : "claude_code";
}

export interface AgentSessionParams {
  cwd: string;
  task: string;
  runner?: AgentRunnerId;
  cols?: number;
  rows?: number;
}

export function buildAgentSessionOpts(
  params: AgentSessionParams,
): CreateSessionOpts {
  return {
    kind: { type: "agent", runner: params.runner ?? "claude_code" },
    title: params.task,
    cwd: params.cwd,
    cols: params.cols ?? 100,
    rows: params.rows ?? 30,
    worktree_task: params.task,
  };
}
