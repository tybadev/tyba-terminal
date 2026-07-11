import { describe, expect, test } from "bun:test";

import type { AgentRepoConfig } from "./ipc";
import {
  applyConsentDecision,
  buildAgentSessionOpts,
  needsConsentPrompt,
} from "./agentSession";

const config = (over: Partial<AgentRepoConfig> = {}): AgentRepoConfig => ({
  hash: "abc123",
  default_agent: null,
  env_allow: ["DATABASE_URL"],
  consent: null,
  ...over,
});

describe("needsConsentPrompt", () => {
  test("sem config no repo, nada pra consentir", () => {
    expect(needsConsentPrompt(null)).toBe(false);
  });

  test("config com consent true não pede de novo", () => {
    expect(needsConsentPrompt(config({ consent: true }))).toBe(false);
  });

  test("config com consent null ou false pede consent", () => {
    expect(needsConsentPrompt(config({ consent: null }))).toBe(true);
    expect(needsConsentPrompt(config({ consent: false }))).toBe(true);
  });
});

describe("applyConsentDecision", () => {
  test("permitir marca consent true e pede persistência", () => {
    const outcome = applyConsentDecision(config({ consent: null }), "allow");
    expect(outcome.persist).toBe(true);
    expect(outcome.config.consent).toBe(true);
  });

  test("agora não segue sem persistir e sem marcar consent true", () => {
    const outcome = applyConsentDecision(config({ consent: null }), "skip");
    expect(outcome.persist).toBe(false);
    expect(outcome.config.consent).toBe(false);
  });
});

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
});
