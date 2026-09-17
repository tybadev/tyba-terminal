import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

import {
  canoDisplay,
  connectionTestCopy,
  failureCopy,
  knownHostsRemoveCommand,
  nextInDrop,
} from "./canoFailure";
import type { ConnectionTest, FailureReason } from "./ipc";

const REASONS: FailureReason[] = [
  "auth_refused",
  "host_key_changed",
  "host_key_rejected",
  "host_unresolved",
  "no_route",
  "unknown",
];

describe("failureCopy", () => {
  test("cada motivo tem um texto próprio", () => {
    const keys = REASONS.map((r) => failureCopy(r).titleKey);
    expect(new Set(keys).size).toBe(REASONS.length);
  });

  // Regra 21: digital trocada pode ser ataque — é o único motivo em vermelho.
  test("só a digital trocada é perigo", () => {
    const danger = REASONS.filter((r) => failureCopy(r).tone === "danger");
    expect(danger).toEqual(["host_key_changed"]);
  });
});

describe("knownHostsRemoveCommand", () => {
  test("porta padrão usa o nome puro", () => {
    expect(knownHostsRemoveCommand("vps.example.test", null)).toBe(
      "ssh-keygen -R vps.example.test",
    );
    expect(knownHostsRemoveCommand("203.0.113.7", 22)).toBe(
      "ssh-keygen -R 203.0.113.7",
    );
  });

  // O known_hosts grava `[host]:porta` fora da 22, e `[` é glob no zsh: sem
  // aspas o comando colado falha com "no matches found".
  test("porta própria usa colchetes entre aspas", () => {
    expect(knownHostsRemoveCommand("203.0.113.7", 2222)).toBe(
      "ssh-keygen -R '[203.0.113.7]:2222'",
    );
  });
});

describe("canoDisplay", () => {
  // O prompt de senha/passphrase aparece no pane durante `connecting`: cobrir
  // o pane ali esconde justamente o que o dono precisa responder.
  test("conectando pela primeira vez é faixa, não cobre o pane", () => {
    expect(canoDisplay("connecting", null, false)).toEqual({
      kind: "strip",
      labelKey: "sshConnecting",
    });
  });

  test("sessão viva ou que não é SSH não mostra nada", () => {
    expect(canoDisplay("live", null, false)).toEqual({ kind: "none" });
    expect(canoDisplay(undefined, null, false)).toEqual({ kind: "none" });
  });

  // Dentro de uma queda a tela alterna faixa (tentativa em curso) e overlay
  // (espera entre tentativas); o texto é o mesmo para não piscar sentido.
  test("na queda, esperar e tentar dizem a mesma coisa", () => {
    const waiting = canoDisplay("reconnecting", null, true);
    const trying = canoDisplay("connecting", null, true);
    expect(waiting).toEqual({
      kind: "overlay",
      labelKey: "sshReconnecting",
      spinner: true,
      retry: false,
    });
    expect(trying).toEqual({ kind: "strip", labelKey: "sshReconnecting" });
  });

  test("desistiu: overlay parado com o convite de reconectar", () => {
    expect(canoDisplay("dropped", null, true)).toEqual({
      kind: "overlay",
      labelKey: "sshDropped",
      spinner: false,
      retry: true,
    });
  });

  test("falha de conexão vira cartão com o motivo do core", () => {
    const failure = {
      reason: "auth_refused" as const,
      detail: "root@203.0.113.7: Permission denied (publickey).",
    };
    expect(canoDisplay("failed", failure, false)).toEqual({
      kind: "failure",
      failure,
    });
  });

  // `connection` e `connection_failure` chegam no mesmo evento, mas um core
  // antigo ou um refresh parcial pode trazer a fase sem o motivo.
  test("falha sem motivo ainda mostra o cartão, como desconhecido", () => {
    expect(canoDisplay("failed", null, false)).toEqual({
      kind: "failure",
      failure: { reason: "unknown", detail: "" },
    });
  });
});

describe("nextInDrop", () => {
  test("a espera entre tentativas abre a queda e a tentativa a mantém", () => {
    let inDrop = nextInDrop(false, "live");
    expect(inDrop).toBe(false);
    inDrop = nextInDrop(inDrop, "reconnecting");
    expect(inDrop).toBe(true);
    inDrop = nextInDrop(inDrop, "connecting");
    expect(inDrop).toBe(true);
  });

  test("login, desistência ou falha encerram a queda", () => {
    expect(nextInDrop(true, "live")).toBe(false);
    expect(nextInDrop(true, "dropped")).toBe(false);
    expect(nextInDrop(true, "failed")).toBe(false);
  });

  // "Tentar de novo" a partir de `dropped` é tentativa do usuário, não da
  // queda: a faixa volta a dizer "conectando".
  test("primeira conexão e nova tentativa do usuário não são queda", () => {
    expect(nextInDrop(false, "connecting")).toBe(false);
    expect(nextInDrop(nextInDrop(true, "dropped"), "connecting")).toBe(false);
  });
});

describe("connectionTestCopy", () => {
  test("ok diz com que usuário entrou e em quanto tempo", () => {
    expect(
      connectionTestCopy({ outcome: "ok", user: "root", elapsed_ms: 1234 }, "pt-BR"),
    ).toEqual({
      tone: "ok",
      titleKey: "hostTestOk",
      params: { user: "root", elapsed: "1,2 s" },
    });
  });

  // Regra 20: o TYBA não aceita digital. O texto explica que a confirmação
  // acontece no prompt do próprio ssh, na primeira conexão.
  test("host novo mostra tipo e digital e é informativo", () => {
    expect(
      connectionTestCopy(
        {
          outcome: "host_unknown",
          key_type: "ED25519",
          fingerprint: "SHA256:AAAAexampleexampleexampleexampleexample00",
        },
        "pt-BR",
      ),
    ).toEqual({
      tone: "info",
      titleKey: "hostTestHostUnknown",
      hintKey: "hostTestHostUnknownHint",
      params: {
        keyType: "ED25519",
        fingerprint: "SHA256:AAAAexampleexampleexampleexampleexample00",
      },
    });
  });

  test("passphrase necessária é aviso com orientação", () => {
    expect(
      connectionTestCopy({ outcome: "passphrase_required" }, "pt-BR"),
    ).toEqual({
      tone: "warning",
      titleKey: "hostTestPassphrase",
      hintKey: "hostTestPassphraseHint",
      params: {},
    });
  });

  test("senha aceita diz que o servidor pede senha, sem ter tentado", () => {
    expect(
      connectionTestCopy({ outcome: "password_accepted", elapsed_ms: 800 }, "en"),
    ).toEqual({
      tone: "ok",
      titleKey: "hostTestPasswordAccepted",
      hintKey: "hostTestPasswordAcceptedHint",
      params: { elapsed: "0.8 s" },
    });
  });

  test("senha não oferecida lista o que o servidor aceita", () => {
    expect(
      connectionTestCopy(
        { outcome: "password_not_offered", methods: ["publickey", "gssapi-with-mic"] },
        "pt-BR",
      ),
    ).toEqual({
      tone: "warning",
      titleKey: "hostTestPasswordNotOffered",
      params: { methods: "publickey, gssapi-with-mic" },
    });
  });

  test("falha usa o texto do motivo e carrega a última linha do ssh", () => {
    expect(
      connectionTestCopy(
        {
          outcome: "failed",
          reason: "no_route",
          detail: "ssh: connect to host 203.0.113.7 port 22: Connection refused",
        },
        "pt-BR",
      ),
    ).toEqual({
      tone: "warning",
      titleKey: "canoFailNoRoute",
      hintKey: "canoFailNoRouteHint",
      params: {
        detail: "ssh: connect to host 203.0.113.7 port 22: Connection refused",
      },
    });
  });

  test("digital trocada no teste também é perigo", () => {
    const copy = connectionTestCopy(
      { outcome: "failed", reason: "host_key_changed", detail: "x" },
      "pt-BR",
    );
    expect(copy.tone).toBe("danger");
    expect(copy.titleKey).toBe("canoFailHostKeyChanged");
  });

  // Aprovação lenta no 1Password estoura o prazo de 30 s; o texto precisa
  // apontar para ela, senão parece servidor fora do ar.
  test("tempo esgotado aponta para a aprovação do agente", () => {
    expect(connectionTestCopy({ outcome: "timed_out" }, "pt-BR")).toEqual({
      tone: "warning",
      titleKey: "hostTestTimedOut",
      hintKey: "hostTestTimedOutHint",
      params: {},
    });
  });
});

// O módulo de i18n lê `localStorage` ao carregar; sem DOM, a checagem é pelo
// texto do arquivo. Chave ausente vira a própria chave na tela.
describe("todo texto usado existe em pt-BR e en", () => {
  const source = readFileSync(new URL("../i18n/index.ts", import.meta.url), "utf8");
  const occurrences = (key: string) =>
    source.split(new RegExp(`\\n\\s+"?${key}"?:`)).length - 1;

  test("motivos, desfechos do teste e fases do pane", () => {
    const keys = new Set<string>(["sshConnecting", "sshReconnecting", "sshDropped"]);
    for (const reason of REASONS) {
      keys.add(failureCopy(reason).titleKey);
      keys.add(failureCopy(reason).hintKey);
    }
    const samples: ConnectionTest[] = [
      { outcome: "ok", user: "u", elapsed_ms: 1 },
      { outcome: "host_unknown", key_type: "k", fingerprint: "f" },
      { outcome: "passphrase_required" },
      { outcome: "password_accepted", elapsed_ms: 1 },
      { outcome: "password_not_offered", methods: [] },
      { outcome: "timed_out" },
    ];
    for (const sample of samples) {
      const copy = connectionTestCopy(sample, "en");
      keys.add(copy.titleKey);
      if (copy.hintKey) keys.add(copy.hintKey);
    }
    const missing = [...keys].filter((k) => occurrences(k) !== 2);
    expect(missing).toEqual([]);
  });
});
