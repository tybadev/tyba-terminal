import { describe, expect, test } from "bun:test";

import { isFinishedStatus, sameSessionStatus } from "./sessionStatus";
import type { SessionStatus } from "./ipc";

describe("isFinishedStatus", () => {
  test("exited e failed são terminais", () => {
    expect(isFinishedStatus({ state: "exited", code: 0 })).toBe(true);
    expect(isFinishedStatus({ state: "failed", reason: "spawn" })).toBe(true);
  });

  test("estados vivos não são terminais", () => {
    expect(isFinishedStatus({ state: "running" })).toBe(false);
    expect(isFinishedStatus({ state: "idle" })).toBe(false);
    expect(isFinishedStatus({ state: "awaiting_input", hint: null })).toBe(
      false,
    );
  });
});

describe("sameSessionStatus", () => {
  test("mesmo estado sem payload é igual", () => {
    expect(sameSessionStatus({ state: "running" }, { state: "running" })).toBe(
      true,
    );
  });

  test("estados diferentes nunca são iguais", () => {
    expect(sameSessionStatus({ state: "running" }, { state: "idle" })).toBe(
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
        { state: "awaiting_input", hint: "y/n" },
        { state: "awaiting_input", hint: "y/n" },
        true,
      ],
      [
        { state: "awaiting_input", hint: "y/n" },
        { state: "awaiting_input", hint: null },
        false,
      ],
      [
        { state: "failed", reason: "a" },
        { state: "failed", reason: "b" },
        false,
      ],
    ];
    for (const [a, b, want] of cases) {
      expect(sameSessionStatus(a, b)).toBe(want);
    }
  });
});
