import { describe, expect, test } from "bun:test";

import {
  INITIAL_PANEL_UI,
  panelRequestsBySession,
  panelStep,
  panelVisibleApprovalIds,
  redReasonText,
  toastIsPointerOnly,
  withoutResolved,
  type PanelEvent,
  type PanelUi,
} from "./approvalPanel";
import { approvalActions, hiddenApprovalIds } from "./notifications";
import type { ApprovalRequest } from "./ipc";

const red = (overrides: Partial<ApprovalRequest> = {}): ApprovalRequest => ({
  id: 1,
  session_id: "s1",
  command: "sudo true",
  cwd: "/repo/wt",
  risk: "red",
  risk_reason: { key: "sudo" },
  context: null,
  requested_at_ms: 100,
  ...overrides,
});

const run = (request: ApprovalRequest, events: PanelEvent[]) => {
  let ui: PanelUi = INITIAL_PANEL_UI;
  const effects = [];
  for (const event of events) {
    const step = panelStep(request, ui, event);
    ui = step.ui;
    effects.push(step.effect);
  }
  return { ui, effects };
};

describe("panelRequestsBySession", () => {
  test("mostra o vermelho mais antigo de cada sessão", () => {
    const approvals = [
      red({ id: 3, session_id: "s1", requested_at_ms: 300 }),
      red({ id: 1, session_id: "s1", requested_at_ms: 100 }),
      red({ id: 2, session_id: "s2", requested_at_ms: 200 }),
    ];
    const map = panelRequestsBySession(approvals);
    expect(map.get("s1")?.id).toBe(1);
    expect(map.get("s2")?.id).toBe(2);
    expect(map.size).toBe(2);
  });

  test("mesmo instante desempata pelo id (ordem de chegada)", () => {
    const approvals = [
      red({ id: 9, requested_at_ms: 100 }),
      red({ id: 4, requested_at_ms: 100 }),
    ];
    expect(panelRequestsBySession(approvals).get("s1")?.id).toBe(4);
  });

  test("amarelo e verde nunca entram no painel (amarelo do Codex fica no toast)", () => {
    const approvals = [
      red({ id: 1, risk: "yellow", risk_reason: null, requested_at_ms: 1 }),
      red({ id: 2, risk: "green", risk_reason: null, requested_at_ms: 2 }),
      red({ id: 3, requested_at_ms: 3 }),
    ];
    expect(panelRequestsBySession(approvals).get("s1")?.id).toBe(3);
    expect(
      panelRequestsBySession([approvals[0], approvals[1]]).size,
    ).toBe(0);
  });

  test("resolver o primeiro revela o próximo da mesma sessão", () => {
    const approvals = [
      red({ id: 1, requested_at_ms: 100 }),
      red({ id: 2, requested_at_ms: 200 }),
    ];
    const after = withoutResolved(approvals, 1);
    expect(panelRequestsBySession(after).get("s1")?.id).toBe(2);
  });
});

describe("panelVisibleApprovalIds", () => {
  const approvals = [
    red({ id: 1, session_id: "s1", requested_at_ms: 100 }),
    red({ id: 2, session_id: "s1", requested_at_ms: 200 }),
    red({ id: 3, session_id: "s2", requested_at_ms: 150 }),
  ];

  test("só os pedidos que um painel na tela mostra", () => {
    expect([...panelVisibleApprovalIds(approvals, new Set(["s1"]))]).toEqual([
      1,
    ]);
  });

  test("sessão fora da tela não esconde o toast dela", () => {
    expect(panelVisibleApprovalIds(approvals, new Set()).size).toBe(0);
  });

  test("entram em hiddenApprovalIds — o toast some para eles", () => {
    const hidden = hiddenApprovalIds({
      approvals,
      notificationsOpen: false,
      agentQueueOpen: false,
      agentQueueVisibleIds: new Set(),
      sessionPanelIds: panelVisibleApprovalIds(approvals, new Set(["s1", "s2"])),
    });
    expect([...hidden].sort()).toEqual([1, 3]);
  });
});

describe("panelStep — dois passos no vermelho", () => {
  test("o primeiro clique em Aprovar só arma; o segundo resolve", () => {
    const { ui, effects } = run(red(), [
      { type: "action", action: "approve" },
      { type: "action", action: "approve" },
    ]);
    expect(effects[0]).toBeNull();
    expect(effects[1]).toEqual({
      type: "resolve",
      requestId: 1,
      decision: "approved",
    });
    expect(ui.confirming).toBe(true);
  });

  test("um clique só em Aprovar nunca chama o core", () => {
    const { ui, effects } = run(red(), [{ type: "action", action: "approve" }]);
    expect(effects).toEqual([null]);
    expect(ui.confirming).toBe(true);
  });

  test("Negar resolve no primeiro clique", () => {
    const { effects } = run(red(), [{ type: "action", action: "deny" }]);
    expect(effects[0]).toEqual({
      type: "resolve",
      requestId: 1,
      decision: "denied",
    });
  });

  test("abrir 'dizer o que fazer' desarma a confirmação", () => {
    const { ui, effects } = run(red(), [
      { type: "action", action: "approve" },
      { type: "action", action: "denyWithReason" },
      { type: "cancelFeedback" },
      { type: "action", action: "approve" },
    ]);
    expect(effects.every((e) => e === null)).toBe(true);
    expect(ui.confirming).toBe(true);
  });

  test("vermelho não oferece sempre-permitir", () => {
    expect(approvalActions("red").map((a) => a.id)).toEqual([
      "approve",
      "deny",
      "denyWithReason",
    ]);
    const { effects } = run(red(), [{ type: "action", action: "alwaysAllow" }]);
    expect(effects[0]).toBeNull();
  });
});

describe("panelStep — negar e dizer o que fazer", () => {
  test("o campo aparece e envia o texto como recusa", () => {
    const { ui, effects } = run(red(), [
      { type: "action", action: "denyWithReason" },
      { type: "typeFeedback", text: "use o script sem sudo" },
      { type: "sendFeedback" },
    ]);
    expect(effects[0]).toBeNull();
    expect(ui.feedbackOpen).toBe(true);
    expect(effects[2]).toEqual({
      type: "resolve",
      requestId: 1,
      decision: "denied",
      feedback: "use o script sem sudo",
    });
  });

  test("cancelar fecha o campo e limpa o texto sem decidir", () => {
    const { ui, effects } = run(red(), [
      { type: "action", action: "denyWithReason" },
      { type: "typeFeedback", text: "x" },
      { type: "cancelFeedback" },
    ]);
    expect(effects.every((e) => e === null)).toBe(true);
    expect(ui).toEqual(INITIAL_PANEL_UI);
  });
});

describe("Esc e a saída do painel", () => {
  test("Esc não decide nem muda o painel", () => {
    const armed = run(red(), [{ type: "action", action: "approve" }]).ui;
    const step = panelStep(red(), armed, { type: "escape" });
    expect(step.effect).toBeNull();
    expect(step.ui).toBe(armed);
  });

  test("o painel só some quando approvals://resolved tira o pedido", () => {
    const approvals = [red({ id: 1 })];
    expect(panelRequestsBySession(approvals).get("s1")?.id).toBe(1);
    expect(panelRequestsBySession(withoutResolved(approvals, 99)).size).toBe(1);
    expect(panelRequestsBySession(withoutResolved(approvals, 1)).size).toBe(0);
  });
});

describe("toast e motivo", () => {
  test("toast de vermelho é só ponteiro; amarelo continua decidindo", () => {
    expect(toastIsPointerOnly(red())).toBe(true);
    expect(toastIsPointerOnly(red({ risk: "yellow" }))).toBe(false);
  });

  test("motivo traduzido por chave, com o caminho no fora-do-worktree", () => {
    expect(redReasonText({ key: "sudo" })).toEqual({
      key: "redReasonSudo",
      params: { path: "", action: "" },
    });
    expect(
      redReasonText({ key: "outside_worktree", arg: "/etc/hosts" }),
    ).toEqual({
      key: "redReasonOutsideWorktree",
      params: { path: "/etc/hosts", action: "/etc/hosts" },
    });
    expect(
      redReasonText({ key: "forge_write", arg: "gh pr merge" }).key,
    ).toBe("redReasonForgeWrite");
    expect(
      redReasonText({ key: "forge_write", arg: "gh pr merge" }).params.action,
    ).toBe("gh pr merge");
    expect(redReasonText(null).key).toBe("redReasonUnknown");
  });
});
