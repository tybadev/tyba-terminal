import { describe, expect, test } from "bun:test";

import {
  buildAgentSessionOpts,
  runnerFromCommand,
  runnerFromDefault,
} from "./agentSession";

describe("buildAgentSessionOpts", () => {
  test("monta o payload com kind agent/claude_code e worktree_task", () => {
    const opts = buildAgentSessionOpts({
      cwd: "/tmp/worktree/foo",
      task: "corrige o watcher",
    });
    expect(opts).toEqual({
      kind: { type: "agent", runner: "claude_code" },
      title: "corrige o watcher",
      cwd: "/tmp/worktree/foo",
      cols: 100,
      rows: 30,
      worktree_task: "corrige o watcher",
    });
  });

  test("aceita cols/rows customizados", () => {
    const opts = buildAgentSessionOpts({
      cwd: "/tmp/foo",
      task: "x",
      cols: 80,
      rows: 24,
    });
    expect(opts.cols).toBe(80);
    expect(opts.rows).toBe(24);
  });

  test("runner codex vai no kind", () => {
    const opts = buildAgentSessionOpts({
      cwd: "/tmp/foo",
      task: "x",
      runner: "codex",
    });
    expect(opts.kind).toEqual({ type: "agent", runner: "codex" });
  });
});

describe("runnerFromDefault", () => {
  test("mapeia codex e claude", () => {
    expect(runnerFromDefault("codex")).toBe("codex");
    expect(runnerFromDefault("claude")).toBe("claude_code");
    expect(runnerFromDefault("claude_code")).toBe("claude_code");
  });

  test("desconhecido ou ausente vira null", () => {
    expect(runnerFromDefault("cursor")).toBe(null);
    expect(runnerFromDefault(null)).toBe(null);
    expect(runnerFromDefault(undefined)).toBe(null);
  });
});

describe("runnerFromCommand", () => {
  test("casa pelo binário, ignorando argumentos e caminho", () => {
    expect(runnerFromCommand("codex")).toBe("codex");
    expect(runnerFromCommand("  codex --sandbox workspace-write ")).toBe("codex");
    expect(runnerFromCommand("/opt/homebrew/bin/codex")).toBe("codex");
    expect(runnerFromCommand("claude --model opus")).toBe("claude_code");
  });

  test("desconhecido ou vazio cai no default do produto", () => {
    expect(runnerFromCommand("cursor")).toBe("claude_code");
    expect(runnerFromCommand("")).toBe("claude_code");
    expect(runnerFromCommand(null)).toBe("claude_code");
    expect(runnerFromCommand(undefined)).toBe("claude_code");
  });
});
