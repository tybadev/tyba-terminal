import { describe, expect, test } from "bun:test";

import {
  approvalActions,
  availableApprovalActions,
  canAlwaysAllow,
  decideApproval,
  hiddenApprovalIds,
  toNotificationItems,
  attentionCount,
  dayBucket,
  filterEntries,
  groupBySession,
  historyEntries,
  isDoneGroup,
  resetParts,
  unreadCount,
} from "./notifications";
import type { ApprovalRequest, NotificationEntry } from "./ipc";

const makeApproval = (
  overrides: Partial<ApprovalRequest> = {},
): ApprovalRequest => ({
  id: 1,
  session_id: "session-1",
  command: "rm -rf build",
  cwd: null,
  risk: "green",
  risk_reason: null,
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

describe("histórico de notificações", () => {
  const entry = (over: Partial<NotificationEntry>): NotificationEntry => ({
    id: 1,
    at_ms: 0,
    session_id: "s1",
    session_title: "fix-health",
    agent: "claude",
    kind: "agent_finished",
    category: "done",
    approval_id: null,
    pending: false,
    decision: null,
    risk: null,
    command: null,
    detail: null,
    usage: null,
    read: false,
    ...over,
  });

  test("pedido pendente sai do histórico (está ao vivo em Precisa de você)", () => {
    const list = [
      entry({ id: 1, kind: "approval", pending: true, category: "request" }),
      entry({ id: 2, kind: "approval", pending: false, category: "done" }),
      entry({ id: 3 }),
    ];
    expect(historyEntries(list).map((e) => e.id)).toEqual([2, 3]);
  });

  test("prompt nativo fica no histórico mesmo pendente (estado deduzido não vai ao vivo)", () => {
    const list = [
      entry({ id: 1, kind: "native_approval", pending: true, category: "request", risk: "yellow" }),
      entry({ id: 2, kind: "native_approval", pending: false, category: "done", decision: "denied" }),
      entry({ id: 3, kind: "approval", pending: true, category: "request" }),
    ];
    expect(historyEntries(list).map((e) => e.id)).toEqual([1, 2]);
  });

  test("grupo concluído: só quando tudo nele concluiu", () => {
    expect(isDoneGroup([entry({ id: 1 }), entry({ id: 2, kind: "approval", decision: "approved" })])).toBe(true);
    expect(isDoneGroup([entry({ id: 1 }), entry({ id: 2, category: "error" })])).toBe(false);
    expect(isDoneGroup([])).toBe(false);
  });

  test("filtros", () => {
    const list = [
      entry({ id: 1, kind: "approval", category: "done" }),
      entry({ id: 2, kind: "agent_failed", category: "error" }),
      entry({ id: 3, kind: "usage_limit", category: "info" }),
      entry({ id: 4 }),
      entry({ id: 5, kind: "native_approval", category: "done", decision: "approved" }),
    ];
    expect(filterEntries(list, "requests").map((e) => e.id)).toEqual([1, 5]);
    expect(filterEntries(list, "errors").map((e) => e.id)).toEqual([2]);
    expect(filterEntries(list, "done").map((e) => e.id)).toEqual([1, 4, 5]);
    expect(filterEntries(list, "all")).toHaveLength(5);
  });

  test("dayBucket", () => {
    const now = new Date(2026, 9, 13, 15, 0).getTime();
    expect(dayBucket(new Date(2026, 9, 13, 0, 5).getTime(), now)).toBe("today");
    expect(dayBucket(new Date(2026, 9, 12, 23, 59).getTime(), now)).toBe("yesterday");
    expect(dayBucket(new Date(2026, 9, 10).getTime(), now)).toBe("earlier");
  });

  test("dayBucket: ontem é o dia anterior por data, não 24h (dia de 25h no fim do horário de verão)", () => {
    const tz = process.env.TZ;
    process.env.TZ = "America/New_York";
    try {
      // 1º/11/2026 tem 25h em Nova York (volta do horário de verão).
      const now = new Date(2026, 10, 2, 10, 0).getTime();
      expect(dayBucket(new Date(2026, 10, 1, 0, 30).getTime(), now)).toBe("yesterday");
      expect(dayBucket(new Date(2026, 9, 31, 23, 59).getTime(), now)).toBe("earlier");
    } finally {
      process.env.TZ = tz;
    }
  });

  test("resetParts: só a hora nas próximas 24h, data + hora depois", () => {
    const now = new Date(2026, 9, 13, 15, 0).getTime();
    const secs = (d: Date) => d.getTime() / 1000;
    expect(resetParts(secs(new Date(2026, 9, 13, 18, 0)), now)).toEqual({ time: "18:00", date: null });
    expect(resetParts(secs(new Date(2026, 9, 14, 14, 59)), now)).toEqual({ time: "14:59", date: null });
    expect(resetParts(secs(new Date(2026, 9, 14, 15, 0)), now)).toEqual({ time: "15:00", date: "14/10" });
    expect(resetParts(secs(new Date(2026, 9, 20, 8, 5)), now)).toEqual({ time: "08:05", date: "20/10" });
  });

  test("groupBySession mantém a ordem do item mais recente", () => {
    const list = [
      entry({ id: 1, session_id: "b", session_title: "api" }),
      entry({ id: 2, session_id: "a", session_title: "fix" }),
      entry({ id: 3, session_id: "b", session_title: "api" }),
    ];
    const groups = groupBySession(list, (e) => ({ title: e.session_title, agent: e.agent }));
    expect(groups.map((g) => [g.session, g.items.map((i) => i.id)])).toEqual([
      ["b", [1, 3]],
      ["a", [2]],
    ]);
  });

  test("attentionCount soma pendentes e erros não lidos", () => {
    const list = [
      entry({ id: 1, category: "error" }),
      entry({ id: 2, category: "error", read: true }),
      entry({ id: 3 }),
    ];
    expect(attentionCount(2, list)).toBe(3);
    expect(unreadCount(list)).toBe(2);
    // O amarelo do prompt nativo não entra no badge: o pendente é deduzido e
    // pode ficar pendurado depois de negado no terminal.
    const withNative = [...list, entry({ id: 4, kind: "native_approval", pending: true, category: "request" })];
    expect(attentionCount(2, withNative)).toBe(3);
  });
});
