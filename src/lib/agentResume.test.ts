import { describe, expect, test } from "bun:test";

import {
  isResumeCandidate,
  resumeCandidates,
  showAgentResumeInvite,
} from "./agentResume";
import type { Session } from "./ipc";

const session = (over: Partial<Session> = {}): Session => ({
  id: "11111111-1111-4111-8111-111111111111",
  kind: { type: "agent", runner: "claude_code" },
  title: "tyba/fatia",
  repo_root: "/repo",
  worktree: null,
  status: { state: "exited", code: 0 },
  attention: false,
  created_at: "2026-08-24T10:00:00Z",
  agent_conversation_id: "5f2a1c40-0000-4000-8000-00000000abcd",
  ...over,
});

describe("isResumeCandidate", () => {
  test("sessão de agente morta com conversa registrada é candidata", () => {
    expect(isResumeCandidate(session())).toBe(true);
    expect(isResumeCandidate(session({ status: { state: "failed", reason: "x" } }))).toBe(
      true,
    );
  });

  test("sem id de conversa não há o que retomar", () => {
    expect(isResumeCandidate(session({ agent_conversation_id: null }))).toBe(false);
    expect(isResumeCandidate(session({ agent_conversation_id: undefined }))).toBe(false);
  });

  test("agente vivo não é candidato — retomar é para sessão morta", () => {
    expect(isResumeCandidate(session({ status: { state: "running" } }))).toBe(false);
    expect(isResumeCandidate(session({ status: { state: "idle", summary: null } }))).toBe(
      false,
    );
  });

  test("shell não retoma conversa nenhuma", () => {
    expect(isResumeCandidate(session({ kind: { type: "shell" } }))).toBe(false);
  });
});

describe("resumeCandidates", () => {
  test("filtra a lista de sessões sem disparar IPC para as demais", () => {
    const alvo = session({ id: "a" });
    const outras = [
      session({ id: "b", kind: { type: "shell" } }),
      session({ id: "c", status: { state: "running" } }),
      session({ id: "d", agent_conversation_id: null }),
    ];
    expect(resumeCandidates([alvo, ...outras])).toEqual(["a"]);
  });
});

describe("showAgentResumeInvite", () => {
  test("só aparece depois do sim do core", () => {
    expect(showAgentResumeInvite(session(), true, false)).toBe(true);
  });

  test("enquanto o core não respondeu, nada na tela", () => {
    expect(showAgentResumeInvite(session(), undefined, false)).toBe(false);
  });

  test("core disse que não dá: silêncio, não erro", () => {
    expect(showAgentResumeInvite(session(), false, false)).toBe(false);
  });

  test("dispensado pelo usuário some", () => {
    expect(showAgentResumeInvite(session(), true, true)).toBe(false);
  });
});
