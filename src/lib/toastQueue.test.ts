import { describe, expect, test } from "bun:test";

import {
  addApprovalToast,
  removeApprovalToast,
  visibleApprovalToasts,
} from "./toastQueue";
import type { ApprovalRequest } from "./ipc";

const makeApproval = (
  overrides: Partial<ApprovalRequest> = {},
): ApprovalRequest => ({
  id: 1,
  session_id: "session-1",
  command: "git push",
  cwd: null,
  risk: "yellow",
  context: null,
  requested_at_ms: 0,
  ...overrides,
});

describe("addApprovalToast", () => {
  test("adiciona um toast novo", () => {
    const approval = makeApproval({ id: 7 });
    expect(addApprovalToast([], approval)).toEqual([
      { id: 7, approval },
    ]);
  });

  test("não duplica o mesmo pedido nascendo duas vezes", () => {
    const approval = makeApproval({ id: 7 });
    const first = addApprovalToast([], approval);
    const second = addApprovalToast(first, approval);
    expect(second).toEqual(first);
    expect(second).toHaveLength(1);
  });
});

describe("removeApprovalToast", () => {
  test("some quando o pedido é resolvido por outra superfície", () => {
    const approval = makeApproval({ id: 9 });
    const toasts = addApprovalToast([], approval);
    expect(removeApprovalToast(toasts, 9)).toEqual([]);
  });

  test("é no-op quando o id não está na fila", () => {
    const toasts = addApprovalToast([], makeApproval({ id: 1 }));
    expect(removeApprovalToast(toasts, 999)).toEqual(toasts);
  });
});

describe("visibleApprovalToasts", () => {
  test("some quando o painel de notificações está aberto", () => {
    const toasts = addApprovalToast([], makeApproval({ id: 1 }));
    expect(visibleApprovalToasts(toasts, true)).toEqual([]);
  });

  test("continua visível quando o painel está fechado", () => {
    const toasts = addApprovalToast([], makeApproval({ id: 1 }));
    expect(visibleApprovalToasts(toasts, false)).toEqual(toasts);
  });
});
