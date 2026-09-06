import { describe, expect, test } from "bun:test";

import type { DetectedAgent, SessionKind } from "./ipc";
import {
  agentBinaryName,
  noticeKey,
  showShellAgentNotice,
  showUnjailedNotice,
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
  test("shell com agente detectado E SEM GATE avisa", () => {
    expect(showShellAgentNotice(shell, detected(), false, undefined)).toBe(
      true,
    );
  });

  test("sessão gerenciada nunca avisa", () => {
    expect(showShellAgentNotice(agent, detected(), false, undefined)).toBe(
      false,
    );
  });

  test("sem detecção não avisa", () => {
    expect(showShellAgentNotice(shell, null, false, undefined)).toBe(false);
  });

  // Shim v2 (tech-spec §7): o agente hospedado JÁ tem gate — a faixa "sem
  // gate" é do v1 (agente cru, sem o shim) e não pode reaparecer por cima de
  // uma sessão que o próprio TYBA já protegeu, jaulada ou não.
  test("hospedado com gate nunca mostra a faixa 'sem gate', jaulado ou não", () => {
    expect(showShellAgentNotice(shell, detected(), true, undefined)).toBe(
      false,
    );
  });

  test("ignorar esconde só aquela instância de processo", () => {
    const d = detected();
    expect(showShellAgentNotice(shell, d, false, noticeKey(d))).toBe(false);
    const reborn = detected({ pid: 9999 });
    expect(showShellAgentNotice(shell, reborn, false, noticeKey(d))).toBe(
      true,
    );
    const restarted = detected({ start_ms: 2_000 });
    expect(showShellAgentNotice(shell, restarted, false, noticeKey(d))).toBe(
      true,
    );
  });
});

// Shim v2 (tech-spec §7/§9, decisão 5 da spec): hosting&&jailed -> nada;
// hosting&&!jailed -> ESTE sinal âmbar próprio, nunca o "sem gate" existente;
// !hosting -> nem entra aqui (showShellAgentNotice cuida daquele caso).
describe("showUnjailedNotice", () => {
  test("hospedado E jaulado não mostra nada", () => {
    expect(showUnjailedNotice(shell, detected(), true, true)).toBe(false);
  });

  test("hospedado SEM jaula mostra o sinal", () => {
    expect(showUnjailedNotice(shell, detected(), true, false)).toBe(true);
  });

  test("não hospedado não mostra o sinal de jaula (é o caso do v1)", () => {
    expect(showUnjailedNotice(shell, detected(), false, false)).toBe(false);
  });

  test("sem detecção não mostra nada", () => {
    expect(showUnjailedNotice(shell, null, true, false)).toBe(false);
  });

  test("sessão já gerenciada nunca mostra (não é uma sessão de shell)", () => {
    expect(showUnjailedNotice(agent, detected(), true, false)).toBe(false);
  });
});
