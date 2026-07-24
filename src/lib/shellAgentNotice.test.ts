import { describe, expect, test } from "bun:test";

import type { DetectedAgent, SessionKind } from "./ipc";
import {
  agentBinaryName,
  noticeKey,
  showShellAgentNotice,
} from "./shellAgentNotice";

const shell: SessionKind = { type: "shell" };
const agent: SessionKind = { type: "agent", runner: "claude_code" };

const detected = (over: Partial<DetectedAgent> = {}): DetectedAgent => ({
  pid: 4242,
  start_ms: 1_000,
  kind: "claude_code",
  ...over,
});

describe("agentBinaryName", () => {
  test("mapeia runners conhecidos e custom", () => {
    expect(agentBinaryName("claude_code")).toBe("claude");
    expect(agentBinaryName("codex")).toBe("codex");
    expect(agentBinaryName({ custom: "gemini" })).toBe("gemini");
  });
});

describe("showShellAgentNotice", () => {
  test("shell com agente detectado avisa", () => {
    expect(showShellAgentNotice(shell, detected(), undefined)).toBe(true);
  });

  test("sessão gerenciada nunca avisa", () => {
    expect(showShellAgentNotice(agent, detected(), undefined)).toBe(false);
  });

  test("sem detecção não avisa", () => {
    expect(showShellAgentNotice(shell, null, undefined)).toBe(false);
  });

  test("ignorar esconde só aquela instância de processo", () => {
    const d = detected();
    expect(showShellAgentNotice(shell, d, noticeKey(d))).toBe(false);
    const reborn = detected({ pid: 9999 });
    expect(showShellAgentNotice(shell, reborn, noticeKey(d))).toBe(true);
    const restarted = detected({ start_ms: 2_000 });
    expect(showShellAgentNotice(shell, restarted, noticeKey(d))).toBe(true);
  });
});
