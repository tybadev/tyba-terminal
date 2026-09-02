import { describe, expect, test } from "bun:test";

import {
  approvalActions,
  availableApprovalActions,
  canAlwaysAllow,
  decideApproval,
  hiddenApprovalIds,
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

describe("approvalActions", () => {
  test("amarelo e verde oferecem aprovar, sempre permitir, recusar e recusar com motivo", () => {
    const actions = approvalActions("yellow");
    expect(actions.map((a) => a.id)).toEqual([
      "approve",
      "alwaysAllow",
      "deny",
      "denyWithReason",
    ]);
  });

  test("vermelho nunca oferece sempre permitir, mas mantém o resto do conjunto", () => {
    const actions = approvalActions("red");
    expect(actions.map((a) => a.id)).toEqual([
      "approve",
      "deny",
      "denyWithReason",
    ]);
  });
});

describe("hiddenApprovalIds", () => {
  test("nenhuma superfície aberta: nada escondido, todo toast aparece", () => {
    const approvals = [
      makeApproval({ id: 10 }),
      makeApproval({ id: 11 }),
    ];
    const ids = hiddenApprovalIds({
      approvals,
      notificationsOpen: false,
      agentQueueOpen: false,
      agentQueueVisibleIds: new Set([10]),
    });
    expect(ids.size).toBe(0);
  });

  test("painel de notificações aberto esconde TODOS os pendentes — é 1:1", () => {
    const approvals = [
      makeApproval({ id: 10 }),
      makeApproval({ id: 11 }),
    ];
    const ids = hiddenApprovalIds({
      approvals,
      notificationsOpen: true,
      agentQueueOpen: false,
      agentQueueVisibleIds: new Set(),
    });
    expect(ids).toEqual(new Set([10, 11]));
  });

  test("fila aberta esconde só os ids que ELA renderiza — o resto continua com toast", () => {
    const approvals = [
      makeApproval({ id: 10 }),
      makeApproval({ id: 11 }),
    ];
    const ids = hiddenApprovalIds({
      approvals,
      notificationsOpen: false,
      agentQueueOpen: true,
      agentQueueVisibleIds: new Set([10]),
    });
    expect(ids).toEqual(new Set([10]));
  });

  test("fila fechada ignora agentQueueVisibleIds mesmo que venha preenchido", () => {
    const ids = hiddenApprovalIds({
      approvals: [makeApproval({ id: 10 })],
      notificationsOpen: false,
      agentQueueOpen: false,
      agentQueueVisibleIds: new Set([10]),
    });
    expect(ids.size).toBe(0);
  });
});

describe("decideApproval", () => {
  test("risco não-vermelho resolve direto, sem passo de confirmação", () => {
    const request = makeApproval({ id: 5, risk: "green" });
    expect(
      decideApproval({ request, decision: "denied", confirmingId: null }),
    ).toEqual({ type: "resolve", requestId: 5, decision: "denied" });
  });

  test("aprovar risco vermelho arma a confirmação no primeiro clique", () => {
    const request = makeApproval({ id: 8, risk: "red" });
    expect(
      decideApproval({ request, decision: "approved", confirmingId: null }),
    ).toEqual({ type: "armRedConfirm", requestId: 8 });
  });

  test("aprovar risco vermelho resolve no segundo clique, já armado", () => {
    const request = makeApproval({ id: 8, risk: "red" });
    expect(
      decideApproval({ request, decision: "approved", confirmingId: 8 }),
    ).toEqual({ type: "resolve", requestId: 8, decision: "approved" });
  });

  test("recusar risco vermelho não passa pela confirmação", () => {
    const request = makeApproval({ id: 8, risk: "red" });
    expect(
      decideApproval({ request, decision: "denied", confirmingId: null }),
    ).toEqual({ type: "resolve", requestId: 8, decision: "denied" });
  });

  test("recusar com motivo carrega o feedback no efeito de resolver", () => {
    const request = makeApproval({ id: 3, risk: "yellow" });
    expect(
      decideApproval({
        request,
        decision: "denied",
        confirmingId: null,
        feedback: "usa o outro branch",
      }),
    ).toEqual({
      type: "resolve",
      requestId: 3,
      decision: "denied",
      feedback: "usa o outro branch",
    });
  });
});
