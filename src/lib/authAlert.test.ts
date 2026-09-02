import { describe, expect, it } from "bun:test";

import type { AuthAlertKind, Session } from "./ipc";
import {
  authAlertMessageKey,
  authAlertToastInput,
  withRuntimeAuthAlert,
  withoutAuthAlert,
  withoutRecoveredAuthAlerts,
} from "./authAlert";

const ALL_KINDS: AuthAlertKind[] = [
  "NotLoggedIn",
  "TokenExpiredOrRevoked",
  "CreditBalanceLow",
  "InvalidApiKey",
];

describe("authAlertMessageKey", () => {
  // F4 do contrato: switch exaustivo -- cada kind precisa de uma chave, e
  // nenhuma pode ficar sem uma (o typecheck já reprova em compile-time se um
  // `case` faltar; este teste prova em runtime que o mapeamento existe pra
  // TODO kind vindo do core hoje).
  it("todo kind conhecido mapeia pra alguma chave authAlert*", () => {
    for (const kind of ALL_KINDS) {
      expect(authAlertMessageKey(kind, "runtime")).toStartWith("authAlert");
      expect(authAlertMessageKey(kind, "preflight")).toStartWith("authAlert");
    }
  });

  // R9/item de texto acionável: NotLoggedIn é o único kind cuja mensagem
  // muda com a fase -- preflight ainda não tem o toast de login (Entrega B)
  // na tela, runtime já abortou o turno e `/login` é a saída imediata.
  it("NotLoggedIn tem uma chave própria por fase", () => {
    expect(authAlertMessageKey("NotLoggedIn", "preflight")).toBe(
      "authAlertNotLoggedInPreflight",
    );
    expect(authAlertMessageKey("NotLoggedIn", "runtime")).toBe(
      "authAlertNotLoggedInRuntime",
    );
  });

  // Os outros três kinds só existem no runtime (P do preflight só produz
  // NotLoggedIn -- ver `classify_status_json` no core), mas a chave não
  // muda com a fase mesmo assim: o texto de "chave inválida" ou "sem
  // crédito" é o mesmo não importa quando foi detectado.
  it("os demais kinds ignoram a fase -- mesma chave nos dois casos", () => {
    for (const kind of ALL_KINDS.filter((k) => k !== "NotLoggedIn")) {
      expect(authAlertMessageKey(kind, "preflight")).toBe(
        authAlertMessageKey(kind, "runtime"),
      );
    }
  });

  it("cada kind tem uma chave própria, e nenhuma se repete (menos NotLoggedIn x2)", () => {
    const keys = ALL_KINDS.map((k) => authAlertMessageKey(k, "runtime"));
    expect(new Set(keys).size).toBe(ALL_KINDS.length);
  });
});

describe("authAlertToastInput (F1)", () => {
  it("é sempre warning, sticky e sem ação", () => {
    const input = authAlertToastInput("mensagem qualquer");
    expect(input.tone).toBe("warning");
    expect(input.sticky).toBe(true);
    expect(input.action).toBeUndefined();
    expect(input.title).toBe("mensagem qualquer");
  });
});

describe("withRuntimeAuthAlert / withoutAuthAlert (F2/F3)", () => {
  it("guarda o kind por session_id", () => {
    const next = withRuntimeAuthAlert(new Map(), "s1", "NotLoggedIn");
    expect(next.get("s1")).toBe("NotLoggedIn");
  });

  it("uma sessão com faixa que recebe kind novo troca de kind", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    const next = withRuntimeAuthAlert(prev, "s1", "CreditBalanceLow");
    expect(next.get("s1")).toBe("CreditBalanceLow");
    expect(next.size).toBe(1);
  });

  it("outra sessão não é afetada", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    const next = withRuntimeAuthAlert(prev, "s2", "InvalidApiKey");
    expect(next.get("s1")).toBe("NotLoggedIn");
    expect(next.get("s2")).toBe("InvalidApiKey");
  });

  it("dismiss remove só a sessão pedida", () => {
    const prev = new Map([
      ["s1", "NotLoggedIn" as AuthAlertKind],
      ["s2", "InvalidApiKey" as AuthAlertKind],
    ]);
    const next = withoutAuthAlert(prev, "s1");
    expect(next.has("s1")).toBe(false);
    expect(next.get("s2")).toBe("InvalidApiKey");
  });

  it("dismiss de sessão sem faixa devolve a MESMA referência (sem re-render à toa)", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    expect(withoutAuthAlert(prev, "s2")).toBe(prev);
  });
});

function sessionOf(id: string, state: string): Pick<Session, "id" | "status"> {
  return { id, status: { state } as Session["status"] };
}

describe("withoutRecoveredAuthAlerts (F3, metade recovery)", () => {
  it("remove a faixa da sessão que voltou a running", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    const next = withoutRecoveredAuthAlerts(prev, [sessionOf("s1", "running")]);
    expect(next.has("s1")).toBe(false);
  });

  it("idle/awaiting_input/exited NÃO contam como recovery -- a faixa fica", () => {
    for (const state of ["idle", "awaiting_input", "exited", "failed"]) {
      const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
      const next = withoutRecoveredAuthAlerts(prev, [sessionOf("s1", state)]);
      expect(next.has("s1")).toBe(true);
    }
  });

  it("sessão ausente da lista (fechada) não é tocada -- só `running` limpa", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    expect(withoutRecoveredAuthAlerts(prev, [])).toBe(prev);
  });

  it("nada muda: devolve a MESMA referência (sem re-render à toa)", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    const next = withoutRecoveredAuthAlerts(prev, [sessionOf("s1", "idle")]);
    expect(next).toBe(prev);
  });

  it("só a sessão que recuperou some -- as outras faixas sobrevivem", () => {
    const prev = new Map([
      ["s1", "NotLoggedIn" as AuthAlertKind],
      ["s2", "CreditBalanceLow" as AuthAlertKind],
    ]);
    const next = withoutRecoveredAuthAlerts(prev, [
      sessionOf("s1", "running"),
      sessionOf("s2", "idle"),
    ]);
    expect(next.has("s1")).toBe(false);
    expect(next.get("s2")).toBe("CreditBalanceLow");
  });
});
