import { describe, expect, test } from "bun:test";

import {
  availableApprovalActions,
  canAlwaysAllow,
  shouldAutoClosePopover,
  toNotificationItems,
} from "./notifications";
import type { ApprovalRequest } from "./ipc";

const makeApproval = (
  overrides: Partial<ApprovalRequest> = {},
): ApprovalRequest => ({
  id: 1,
  session_id: "session-1",
  command: "rm -rf build",
  cwd: null,
  risk: "green",
  context: null,
  requested_at_ms: 0,
  ...overrides,
});

describe("availableApprovalActions", () => {
  test("vermelho nunca oferece sempre-permitir", () => {
    expect(availableApprovalActions("red")).toEqual(["approved", "denied"]);
  });

  test("verde e amarelo oferecem sempre-permitir", () => {
    expect(availableApprovalActions("green")).toEqual([
      "approved",
      "denied",
      "approved_always",
    ]);
    expect(availableApprovalActions("yellow")).toEqual([
      "approved",
      "denied",
      "approved_always",
    ]);
  });
});

describe("canAlwaysAllow", () => {
  test("vermelho é sempre false", () => {
    expect(canAlwaysAllow("red")).toBe(false);
  });

  test("verde e amarelo são true", () => {
    expect(canAlwaysAllow("green")).toBe(true);
    expect(canAlwaysAllow("yellow")).toBe(true);
  });
});

describe("shouldAutoClosePopover", () => {
  test("fecha quando o último pendente é resolvido com o popover aberto", () => {
    expect(
      shouldAutoClosePopover({ open: true, previousCount: 1, nextCount: 0 }),
    ).toBe(true);
  });

  test("mantém aberto se ainda houver pendentes", () => {
    expect(
      shouldAutoClosePopover({ open: true, previousCount: 2, nextCount: 1 }),
    ).toBe(false);
  });

  test("não reabre nem faz nada se já estava fechado", () => {
    expect(
      shouldAutoClosePopover({ open: false, previousCount: 1, nextCount: 0 }),
    ).toBe(false);
  });

  test("não dispara quando a fila já estava zerada", () => {
    expect(
      shouldAutoClosePopover({ open: true, previousCount: 0, nextCount: 0 }),
    ).toBe(false);
  });
});

describe("toNotificationItems", () => {
  test("mapeia cada aprovação para um item com discriminador de tipo", () => {
    const approval = makeApproval({ id: 42 });
    expect(toNotificationItems([approval])).toEqual([
      { kind: "approval", id: 42, approval },
    ]);
  });
});
