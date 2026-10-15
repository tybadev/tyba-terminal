import { describe, expect, it, test } from "bun:test";
import { readFileSync } from "node:fs";

import {
  connectingHostIds,
  awaitingLabelKey,
  awaitsDecision,
  isFinishedStatus,
  mergeSessionUpdate,
  sameSessionStatus,
  statusGlyph,
  statusVisual,
  tabAttention,
  withSubagentsReason,
} from "./sessionStatus";
import type { AwaitingReason, Session, SessionStatus } from "./ipc";

const REASONS: AwaitingReason[] = ["approval", "reply", "native_prompt"];

const awaiting = (reason: AwaitingReason, hint: string | null = null) =>
  ({ state: "awaiting_input", hint, reason }) as const;

const withStatus = (id: string, status: SessionStatus): Session =>
  ({
    id,
    kind: { type: "agent", runner: "claude_code" },
    title: id,
    repo_root: null,
    worktree: null,
    status,
    attention: false,
    created_at: "",
  }) as unknown as Session;

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

  test("prompt nativo tem rótulo próprio, não o de resposta", () => {
    const native = statusVisual(awaiting("native_prompt", "Bash: ls"), false);
    expect(native?.labelKey).toBe("sessionNeedsPermission");
    expect(native?.labelKey).not.toBe(
      statusVisual(awaiting("reply"), false)?.labelKey,
    );
  });

  test("prompt nativo tem o mesmo peso visual do vermelho (R5)", () => {
    const native = statusVisual(awaiting("native_prompt"), false);
    const approval = statusVisual(awaiting("approval"), false);
    expect(native?.rank).toBe(approval?.rank);
    expect(native?.dotClass).toBe(approval?.dotClass);
    expect(native?.textClass).toBe(approval?.textClass);
  });

  test("cada motivo de espera tem um rótulo distinto", () => {
    const keys = REASONS.map(awaitingLabelKey);
    expect(new Set(keys).size).toBe(REASONS.length);
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

describe("awaitsDecision", () => {
  test("vermelho e prompt nativo esperam decisão; resposta não", () => {
    expect(awaitsDecision("approval")).toBe(true);
    expect(awaitsDecision("native_prompt")).toBe(true);
    expect(awaitsDecision("reply")).toBe(false);
  });
});

describe("selo de atenção da aba", () => {
  test("aba sem ninguém esperando não tem selo", () => {
    expect(tabAttention([])).toBeNull();
    expect(
      tabAttention([
        withStatus("a", { state: "running" }),
        withStatus("b", { state: "idle", summary: null }),
        withStatus("c", { state: "failed", reason: "x" }),
        withStatus("d", { state: "exited", code: 0 }),
        undefined,
      ]),
    ).toBeNull();
  });

  test("todo motivo de awaiting_input acende o selo, com o rótulo dele", () => {
    for (const reason of REASONS) {
      const selo = tabAttention([withStatus("a", awaiting(reason))]);
      expect(selo?.labelKey).toBe(awaitingLabelKey(reason));
      expect(selo?.dotClass).toContain("bg-tyba-amber");
      expect(selo?.dotClass).toContain("motion-safe:animate-pulse");
    }
  });

  test("prompt nativo acende como pedido de permissão, não como resposta", () => {
    const selo = tabAttention([withStatus("a", awaiting("native_prompt"))]);
    expect(selo?.labelKey).toBe("sessionNeedsPermission");
  });

  test("em split, espera por decisão vence espera por resposta", () => {
    for (const decision of ["approval", "native_prompt"] as const) {
      const antes = tabAttention([
        withStatus("r", awaiting("reply")),
        withStatus("d", awaiting(decision)),
      ]);
      const depois = tabAttention([
        withStatus("d", awaiting(decision)),
        withStatus("r", awaiting("reply")),
      ]);
      expect(antes?.labelKey).toBe(awaitingLabelKey(decision));
      expect(depois?.labelKey).toBe(awaitingLabelKey(decision));
    }
  });

  test("sessão ausente do mapa não derruba o selo das outras", () => {
    const selo = tabAttention([undefined, withStatus("a", awaiting("approval"))]);
    expect(selo?.labelKey).toBe("sessionBlocked");
  });
});

// O módulo de i18n lê `localStorage` ao carregar; sem DOM, a checagem é pelo
// texto do arquivo. Chave ausente vira a própria chave na tela.
describe("rótulo de espera existe em pt-BR e en", () => {
  test("uma vez em cada idioma", () => {
    const source = readFileSync(
      new URL("../i18n/index.ts", import.meta.url),
      "utf8",
    );
    for (const reason of REASONS) {
      const key = awaitingLabelKey(reason);
      const hits = source.split(new RegExp(`\\n\\s+"?${key}"?:`)).length - 1;
      expect(hits).toBe(2);
    }
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

describe("connectingHostIds", () => {
  const session = (over: Partial<Session>): Session => ({
    id: "s",
    kind: { type: "ssh", host_id: "h1" },
    title: "",
    repo_root: null,
    worktree: null,
    status: { state: "running" },
    attention: false,
    created_at: "",
    connection: "connecting",
    ...over,
  });

  test("só SSH Session em connecting conta", () => {
    const ids = connectingHostIds([
      session({ id: "a", kind: { type: "ssh", host_id: "h1" } }),
      session({ id: "b", kind: { type: "ssh", host_id: "h2" }, connection: "live" }),
      session({ id: "c", kind: { type: "ssh", host_id: "h3" }, connection: "failed" }),
      session({ id: "d", kind: { type: "shell" } }),
    ]);
    expect([...ids]).toEqual(["h1"]);
  });

  test("conectado ou caído libera o Host", () => {
    for (const connection of ["live", "reconnecting", "dropped", "failed"] as const) {
      expect(connectingHostIds([session({ connection })]).size).toBe(0);
    }
  });
});

describe("withSubagentsReason", () => {
  test("rodando só por subagente vira 'subagentes trabalhando'", () => {
    const running = statusVisual({ state: "running" }, false);
    expect(withSubagentsReason(running, true)?.labelKey).toBe(
      "sessionSubagentsWorking",
    );
    expect(withSubagentsReason(running, false)?.labelKey).toBe(
      "sessionInProgress",
    );
  });

  test("esperar alguém vence: prompt nativo segue com o próprio rótulo", () => {
    const prompt = statusVisual(
      { state: "awaiting_input", hint: null, reason: "native_prompt" },
      false,
    );
    expect(withSubagentsReason(prompt, true)?.labelKey).toBe(
      "sessionNeedsPermission",
    );
    expect(withSubagentsReason(null, true)).toBeNull();
  });
});

describe("statusGlyph", () => {
  it("cada estado tem a sua forma; rótulo sem estado não afirma nada", () => {
    expect(statusGlyph("sessionInProgress")).toBe("progress");
    expect(statusGlyph("sessionSubagentsWorking")).toBe("progress");
    expect(statusGlyph("sessionFinished")).toBe("done");
    expect(statusGlyph("sessionNeedsPermission")).toBe("waiting");
    expect(statusGlyph("sessionFailed")).toBe("failed");
    expect(statusGlyph("sessionIdle")).toBe("resting");
    expect(statusGlyph("agentsBoardNoSignal")).toBe("resting");
  });
});
