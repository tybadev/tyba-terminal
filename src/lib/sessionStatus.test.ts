import { describe, expect, test } from "bun:test";

import {
  isFinishedStatus,
  mergeSessionUpdate,
  sameSessionStatus,
  statusVisual,
} from "./sessionStatus";
import type { Session, SessionStatus } from "./ipc";

describe("isFinishedStatus", () => {
  test("exited e failed são terminais", () => {
    expect(isFinishedStatus({ state: "exited", code: 0 })).toBe(true);
    expect(isFinishedStatus({ state: "failed", reason: "spawn" })).toBe(true);
  });

  test("estados vivos não são terminais", () => {
    expect(isFinishedStatus({ state: "running" })).toBe(false);
    expect(isFinishedStatus({ state: "idle", summary: null })).toBe(false);
    expect(
      isFinishedStatus({ state: "awaiting_input", hint: null, reason: "reply" }),
    ).toBe(false);
  });
});

describe("sameSessionStatus", () => {
  test("mesmo estado sem payload é igual", () => {
    expect(sameSessionStatus({ state: "running" }, { state: "running" })).toBe(
      true,
    );
  });

  test("estados diferentes nunca são iguais", () => {
    expect(sameSessionStatus({ state: "running" }, { state: "idle", summary: null })).toBe(
      false,
    );
  });

  test("compara o payload da variante, não a identidade do objeto", () => {
    const cases: Array<[SessionStatus, SessionStatus, boolean]> = [
      [
        { state: "exited", code: 0 },
        { state: "exited", code: 0 },
        true,
      ],
      [
        { state: "exited", code: 0 },
        { state: "exited", code: 1 },
        false,
      ],
      [
        { state: "awaiting_input", hint: "y/n", reason: "reply" },
        { state: "awaiting_input", hint: "y/n", reason: "reply" },
        true,
      ],
      [
        { state: "awaiting_input", hint: "y/n", reason: "reply" },
        { state: "awaiting_input", hint: null, reason: "reply" },
        false,
      ],
      [
        { state: "awaiting_input", hint: "git push", reason: "approval" },
        { state: "awaiting_input", hint: "git push", reason: "reply" },
        false,
      ],
      [
        { state: "failed", reason: "a" },
        { state: "failed", reason: "b" },
        false,
      ],
      [
        { state: "idle", summary: "fiz X" },
        { state: "idle", summary: "fiz X" },
        true,
      ],
      [
        { state: "idle", summary: "fiz X" },
        { state: "idle", summary: null },
        false,
      ],
    ];
    for (const [a, b, want] of cases) {
      expect(sameSessionStatus(a, b)).toBe(want);
    }
  });
});

describe("statusVisual", () => {
  test("failed vence tudo e é vermelho fixo", () => {
    const v = statusVisual({ state: "failed", reason: "x" }, false);
    expect(v?.rank).toBe(4);
    expect(v?.dotClass).toContain("bg-tyba-red");
    expect(v?.dotClass).not.toContain("animate-pulse");
  });

  test("awaiting distingue aprovação de resposta", () => {
    const approval = statusVisual(
      { state: "awaiting_input", hint: "git push", reason: "approval" },
      true,
    );
    const reply = statusVisual(
      { state: "awaiting_input", hint: null, reason: "reply" },
      true,
    );
    expect(approval?.labelKey).toBe("sessionBlocked");
    expect(reply?.labelKey).toBe("sessionAwaiting");
    expect(approval?.dotClass).toContain("bg-tyba-amber");
    expect(approval?.dotClass).toContain("animate-pulse");
  });

  test("idle só sinaliza com attention, verde e sem pulso", () => {
    expect(statusVisual({ state: "idle", summary: null }, false)).toBeNull();
    const v = statusVisual({ state: "idle", summary: null }, true);
    expect(v?.labelKey).toBe("sessionFinished");
    expect(v?.dotClass).toContain("bg-tyba-green");
    expect(v?.dotClass).not.toContain("animate-pulse");
  });

  test("running pulsa azul; exited não sinaliza", () => {
    const running = statusVisual({ state: "running" }, false);
    expect(running?.dotClass).toContain("bg-tyba-blue");
    expect(running?.dotClass).toContain("animate-pulse");
    expect(statusVisual({ state: "exited", code: 0 }, true)).toBeNull();
  });

  test("prioridade: failed > awaiting > idle+attention > running", () => {
    const ranks = [
      statusVisual({ state: "failed", reason: "x" }, false)?.rank ?? 0,
      statusVisual(
        { state: "awaiting_input", hint: null, reason: "reply" },
        false,
      )?.rank ?? 0,
      statusVisual({ state: "idle", summary: null }, true)?.rank ?? 0,
      statusVisual({ state: "running" }, false)?.rank ?? 0,
    ];
    expect(ranks).toEqual([...ranks].sort((a, b) => b - a));
  });
});

// A fase do Cano só chegava à tela num refresh incidental da lista: o listener
// descartava o evento como "nada mudou" porque status e atenção não mexem.
describe("mergeSessionUpdate e a fase do Cano", () => {
  const ssh = (over: Partial<Session> = {}): Session => ({
    id: "s1",
    kind: { type: "ssh", host_id: "h1" },
    title: "vps",
    repo_root: null,
    worktree: null,
    status: { state: "running" },
    attention: false,
    created_at: "",
    observed: null,
    connection: "connecting",
    connection_failure: null,
    ...over,
  });

  test("mudança de connection é novidade e chega à sessão", () => {
    const merged = mergeSessionUpdate(ssh(), ssh({ connection: "live" }));
    expect(merged?.connection).toBe("live");
  });

  // "Tentar de novo" pode falhar de novo com outro motivo: a fase continua
  // `failed` e só o motivo muda — o cartão não pode ficar com o antigo.
  test("motivo novo com a mesma fase é novidade", () => {
    const before = ssh({
      connection: "failed",
      connection_failure: { reason: "no_route", detail: "Connection refused" },
    });
    const after = ssh({
      connection: "failed",
      connection_failure: {
        reason: "auth_refused",
        detail: "root@203.0.113.7: Permission denied (publickey).",
      },
    });
    expect(mergeSessionUpdate(before, after)?.connection_failure).toEqual({
      reason: "auth_refused",
      detail: "root@203.0.113.7: Permission denied (publickey).",
    });
  });

  test("motivo que some ao sair de failed some da sessão", () => {
    const before = ssh({
      connection: "failed",
      connection_failure: { reason: "no_route", detail: "x" },
    });
    const merged = mergeSessionUpdate(before, ssh({ connection: "connecting" }));
    expect(merged?.connection).toBe("connecting");
    expect(merged?.connection_failure ?? null).toBeNull();
  });

  test("mesmo motivo com objeto novo não é novidade", () => {
    const failure = () => ({ reason: "no_route" as const, detail: "x" });
    const before = ssh({ connection: "failed", connection_failure: failure() });
    const after = ssh({ connection: "failed", connection_failure: failure() });
    expect(mergeSessionUpdate(before, after)).toBeNull();
  });
});
