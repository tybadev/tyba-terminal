// Tradução do que o core decidiu sobre o Cano (o `ssh` local) em texto e
// gravidade. Nada aqui classifica: o motivo já chega pronto do core.

import type {
  CanoFailure,
  ConnectionState,
  ConnectionTest,
  FailureReason,
} from "./ipc";

export type FailureTone = "danger" | "warning";

export interface FailureCopy {
  titleKey: string;
  hintKey: string;
  tone: FailureTone;
}

const COPY: Record<FailureReason, FailureCopy> = {
  auth_refused: {
    titleKey: "canoFailAuthRefused",
    hintKey: "canoFailAuthRefusedHint",
    tone: "warning",
  },
  host_key_changed: {
    titleKey: "canoFailHostKeyChanged",
    hintKey: "canoFailHostKeyChangedHint",
    tone: "danger",
  },
  host_key_rejected: {
    titleKey: "canoFailHostKeyRejected",
    hintKey: "canoFailHostKeyRejectedHint",
    tone: "warning",
  },
  host_unresolved: {
    titleKey: "canoFailHostUnresolved",
    hintKey: "canoFailHostUnresolvedHint",
    tone: "warning",
  },
  no_route: {
    titleKey: "canoFailNoRoute",
    hintKey: "canoFailNoRouteHint",
    tone: "warning",
  },
  unknown: {
    titleKey: "canoFailUnknown",
    hintKey: "canoFailUnknownHint",
    tone: "warning",
  },
};

export const failureCopy = (reason: FailureReason): FailureCopy =>
  COPY[reason] ?? COPY.unknown;

const SHELL_SAFE = /^[A-Za-z0-9._:@%+-]+$/;

const shellQuote = (value: string): string =>
  SHELL_SAFE.test(value) ? value : `'${value.replace(/'/g, `'\\''`)}'`;

/**
 * O comando que o dono cola para esquecer a digital antiga. O TYBA nunca o
 * executa (regra 21): só mostra.
 */
export const knownHostsRemoveCommand = (
  hostname: string,
  port: number | null,
): string => {
  const entry =
    port === null || port === 22 ? hostname : `[${hostname}]:${port}`;
  return `ssh-keygen -R ${shellQuote(entry)}`;
};

export type CanoDisplay =
  | { kind: "none" }
  | { kind: "strip"; labelKey: string }
  | { kind: "overlay"; labelKey: string; spinner: boolean; retry: boolean }
  | { kind: "failure"; failure: CanoFailure };

export const canoDisplay = (
  connection: ConnectionState | undefined,
  failure: CanoFailure | null | undefined,
  inDrop: boolean,
): CanoDisplay => {
  if (connection === "connecting") {
    return {
      kind: "strip",
      labelKey: inDrop ? "sshReconnecting" : "sshConnecting",
    };
  }
  if (connection === "reconnecting") {
    return {
      kind: "overlay",
      labelKey: "sshReconnecting",
      spinner: true,
      retry: false,
    };
  }
  if (connection === "dropped") {
    return { kind: "overlay", labelKey: "sshDropped", spinner: false, retry: true };
  }
  if (connection === "failed") {
    return {
      kind: "failure",
      failure: failure ?? { reason: "unknown", detail: "" },
    };
  }
  return { kind: "none" };
};

/**
 * A tela não sabe se um `connecting` é a primeira conexão ou uma tentativa
 * dentro de uma queda: o core não manda isso. A diferença é a fase anterior —
 * só `reconnecting` abre uma queda.
 */
export const nextInDrop = (
  inDrop: boolean,
  connection: ConnectionState | undefined,
): boolean => {
  if (connection === "reconnecting") return true;
  if (connection === "connecting") return inDrop;
  return false;
};

export type TestTone = "ok" | "info" | "warning" | "danger";

export interface TestCopy {
  tone: TestTone;
  titleKey: string;
  hintKey?: string;
  params: Record<string, string>;
}

const formatElapsed = (ms: number, locale: string): string =>
  `${new Intl.NumberFormat(locale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(ms / 1000)} s`;

export const connectionTestCopy = (
  test: ConnectionTest,
  locale: string,
): TestCopy => {
  switch (test.outcome) {
    case "ok":
      return {
        tone: "ok",
        titleKey: "hostTestOk",
        params: { user: test.user, elapsed: formatElapsed(test.elapsed_ms, locale) },
      };
    case "host_unknown":
      return {
        tone: "info",
        titleKey: "hostTestHostUnknown",
        hintKey: "hostTestHostUnknownHint",
        params: { keyType: test.key_type, fingerprint: test.fingerprint },
      };
    case "passphrase_required":
      return {
        tone: "warning",
        titleKey: "hostTestPassphrase",
        hintKey: "hostTestPassphraseHint",
        params: {},
      };
    case "password_accepted":
      return {
        tone: "ok",
        titleKey: "hostTestPasswordAccepted",
        hintKey: "hostTestPasswordAcceptedHint",
        params: { elapsed: formatElapsed(test.elapsed_ms, locale) },
      };
    case "password_not_offered":
      return {
        tone: "warning",
        titleKey: "hostTestPasswordNotOffered",
        params: { methods: test.methods.join(", ") },
      };
    case "failed": {
      const copy = failureCopy(test.reason);
      return {
        tone: copy.tone,
        titleKey: copy.titleKey,
        hintKey: copy.hintKey,
        params: { detail: test.detail },
      };
    }
    case "timed_out":
      return {
        tone: "warning",
        titleKey: "hostTestTimedOut",
        hintKey: "hostTestTimedOutHint",
        params: {},
      };
  }
};
