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
  test("some quando o próprio id está no conjunto escondido", () => {
    const toasts = addApprovalToast([], makeApproval({ id: 1 }));
    expect(visibleApprovalToasts(toasts, new Set([1]))).toEqual([]);
  });

  test("continua visível quando o conjunto escondido está vazio", () => {
    const toasts = addApprovalToast([], makeApproval({ id: 1 }));
    expect(visibleApprovalToasts(toasts, new Set())).toEqual(toasts);
  });

  test("filtra só quem está no conjunto — não é tudo ou nada", () => {
    // A fila de agentes esconde só o que ela de fato mostra: sessão com dois
    // pedidos pendentes, a fila colapsa pro mais antigo (#10) e o mais novo
    // (#11) precisa continuar com o toast, senão fica sem ponto de ação
    // nenhum enquanto a fila está aberta.
    const toasts = addApprovalToast(
      addApprovalToast([], makeApproval({ id: 10 })),
      makeApproval({ id: 11 }),
    );
    expect(visibleApprovalToasts(toasts, new Set([10]))).toEqual([
      { id: 11, approval: expect.objectContaining({ id: 11 }) },
    ]);
  });
});
