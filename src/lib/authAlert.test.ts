import { describe, expect, it } from "bun:test";

import type { AuthAlertKind, Session } from "./ipc";
import {
  authAlertExitedMessageKey,
  authAlertMessageKey,
  authAlertToastInput,
  exitedSessionNotice,
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

  // Review round 1, Fix 1 (achado do reviewer): o core pode emitir
  // `{Runtime, kind}` com o settle já vendo `sessions.get` == `None` --
  // "ausente" é um dos braços de R6 no core
  // (`absent_session_at_settle_time_emits_runtime_alert`), real quando o
  // dono fecha uma sessão travada antes dos 2500ms do settle. Sem limpar
  // aqui, essa entrada nunca sai do `Map` -- nenhuma sessão vai "voltar a
  // running" pra acionar o braço de recovery de uma sessão que não existe
  // mais. Cresce sem teto numa sessão longa com vários agentes
  // travados-e-fechados.
  it("sessão que sumiu da lista (fechada/descartada) é limpa -- entrada órfã não vaza", () => {
    const prev = new Map([["s1", "NotLoggedIn" as AuthAlertKind]]);
    const next = withoutRecoveredAuthAlerts(prev, []);
    expect(next.has("s1")).toBe(false);
  });

  // O contraste que dá sentido ao Fix 1: uma sessão `exited` mas AINDA
  // presente na lista (`sessions` guarda as mortas -- `SessionManager`)
  // NÃO é órfã -- é exatamente a entrada que `exitedSessionNotice` (Fix 2)
  // precisa pra mostrar a razão do auth na faixa de "saiu por quê". Só
  // quem SOME da lista de verdade é limpo.
  it("sessão exited mas AINDA na lista sobrevive -- ela alimenta a razão do Fix 2", () => {
    const prev = new Map([["s1", "CreditBalanceLow" as AuthAlertKind]]);
    const next = withoutRecoveredAuthAlerts(prev, [sessionOf("s1", "exited")]);
    expect(next.get("s1")).toBe("CreditBalanceLow");
  });

  it("entrada órfã some, entrada de sessão presente sobrevive -- no mesmo Map", () => {
    const prev = new Map([
      ["orphan", "NotLoggedIn" as AuthAlertKind],
      ["s2", "InvalidApiKey" as AuthAlertKind],
    ]);
    const next = withoutRecoveredAuthAlerts(prev, [sessionOf("s2", "exited")]);
    expect(next.has("orphan")).toBe(false);
    expect(next.get("s2")).toBe("InvalidApiKey");
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

describe("authAlertExitedMessageKey (review round 1, Fix 2)", () => {
  it("todo kind mapeia pra uma chave authAlertExited*, sem repetir", () => {
    const keys = ALL_KINDS.map(authAlertExitedMessageKey);
    for (const key of keys) expect(key).toStartWith("authAlertExited");
    expect(new Set(keys).size).toBe(ALL_KINDS.length);
  });

  it("é uma chave DIFERENTE da mensagem de runtime -- registro muda de 'aja agora' pra 'foi por isso'", () => {
    for (const kind of ALL_KINDS) {
      expect(authAlertExitedMessageKey(kind)).not.toBe(
        authAlertMessageKey(kind, "runtime"),
      );
    }
  });
});

describe("exitedSessionNotice (review round 1, Fix 2)", () => {
  // O teste que o reviewer pediu: sessão saída com auth-alert de runtime
  // mostra a RAZÃO -- não fica só no convite genérico de retomar.
  it("sessão saída com auth-alert: mostra a razão, não só o resume-invite genérico", () => {
    const notice = exitedSessionNotice("CreditBalanceLow", {
      binary: "claude",
    });
    expect(notice).not.toBeNull();
    expect(notice?.messageKey).toBe(
      authAlertExitedMessageKey("CreditBalanceLow"),
    );
    expect(notice?.messageKey).not.toBe("agentResumeNotice");
    expect(notice?.tone).toBe("red");
  });

  it("com auth-alert MAS sem convite de retomar (core não achou conversa retomável): razão aparece, sem ação", () => {
    const notice = exitedSessionNotice("InvalidApiKey", null);
    expect(notice?.messageKey).toBe(authAlertExitedMessageKey("InvalidApiKey"));
    expect(notice?.showResumeAction).toBe(false);
  });

  it("com auth-alert E convite: a razão manda, mas o botão de retomar continua disponível", () => {
    const notice = exitedSessionNotice("TokenExpiredOrRevoked", {
      binary: "claude",
    });
    expect(notice?.tone).toBe("red");
    expect(notice?.showResumeAction).toBe(true);
  });

  it("sem auth-alert, só convite: comportamento de sempre (cyan, texto de resume, com binary)", () => {
    const notice = exitedSessionNotice(null, { binary: "codex" });
    expect(notice).toEqual({
      tone: "cyan",
      messageKey: "agentResumeNotice",
      messageParams: { binary: "codex" },
      showResumeAction: true,
    });
  });

  it("nem auth-alert nem convite: nada pra mostrar", () => {
    expect(exitedSessionNotice(null, null)).toBeNull();
  });
});
