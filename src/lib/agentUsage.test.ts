import { describe, expect, test } from "bun:test";

import type { AgentUsage, Session } from "./ipc";
import { agentAlertToast, sameAgentUsage, usageSummary } from "./agentUsage";
import { mergeSessionUpdate } from "./sessionStatus";

// Tradutor falso com os mesmos textos pt-BR do i18n — o teste é da montagem.
const PT: Record<string, string> = {
  agentsUsageContext: "contexto {{pct}}%",
  agentsUsageTokens: "{{tokens}} tokens",
  agentAlertStopFailure: "O agente parou: {{reason}}",
  agentAlertLimit5h: "Limite de 5h em {{pct}}%",
  agentAlertLimit5hResets: "Limite de 5h em {{pct}}%, reseta às {{time}}",
  agentAlertLimit7d: "Limite de 7 dias em {{pct}}%",
  agentAlertLimit7dResets: "Limite de 7 dias em {{pct}}%, reseta em {{time}}",
};
const t = (key: string, options: Record<string, unknown> = {}) =>
  (PT[key] ?? key).replace(/\{\{(\w+)\}\}/g, (_, k) => String(options[k]));

const nbsp = (s: string | null) => s?.replace(/ /g, " ") ?? null;

const full: AgentUsage = {
  model: "Opus 5.5",
  context_pct: 13.4,
  tokens_in: 45_000,
  tokens_out: 3_210,
  cost_usd: 0.42,
  duration_ms: 252_000,
};

describe("linha de uso do painel de agentes", () => {
  test("a linha do mockup, com tudo presente", () => {
    expect(nbsp(usageSummary(full, "pt-BR", t))).toBe(
      "Opus 5.5 — contexto 13%, 48 mil tokens, US$ 0,42, 4m12s",
    );
  });

  test("só os campos presentes", () => {
    expect(usageSummary({ model: "Opus 5.5" }, "pt-BR", t)).toBe("Opus 5.5");
    expect(usageSummary({ context_pct: 50 }, "pt-BR", t)).toBe("contexto 50%");
    expect(nbsp(usageSummary({ model: "Sonnet", cost_usd: 1.5 }, "pt-BR", t))).toBe(
      "Sonnet — US$ 1,50",
    );
  });

  test("sem uso, nada", () => {
    expect(usageSummary(null, "pt-BR", t)).toBeNull();
    expect(usageSummary(undefined, "pt-BR", t)).toBeNull();
    expect(usageSummary({}, "pt-BR", t)).toBeNull();
  });
});

describe("toast de agent://alert", () => {
  test("falha de turno é erro e fica até fechar", () => {
    const toast = agentAlertToast(
      {
        kind: "stop_failure",
        session_id: "s1",
        error: "rate_limit",
        reason: "limite de uso atingido",
      },
      "rio-api",
      "pt-BR",
      t,
    );
    expect(toast.tone).toBe("error");
    expect(toast.sticky).toBe(true);
    expect(toast.title).toBe("O agente parou: limite de uso atingido");
    expect(toast.detail).toBe("rio-api");
  });

  test("limite de 5h com hora de reset", () => {
    const resets = new Date(2026, 9, 12, 18, 0, 0).getTime() / 1000;
    const toast = agentAlertToast(
      { kind: "usage_limit", session_id: "s1", window: "five_hour", pct: 86.6, resets_at: resets },
      null,
      "pt-BR",
      t,
    );
    expect(toast.tone).toBe("warning");
    expect(toast.title).toBe("Limite de 5h em 87%, reseta às 18:00");
    expect(toast.detail).toBeUndefined();
  });

  test("limite de 7 dias traz a data; sem reset, só o percentual", () => {
    const resets = new Date(2026, 9, 14, 9, 30, 0).getTime() / 1000;
    const withDate = agentAlertToast(
      { kind: "usage_limit", session_id: "s1", window: "seven_day", pct: 90, resets_at: resets },
      null,
      "pt-BR",
      t,
    );
    expect(withDate.title).toBe("Limite de 7 dias em 90%, reseta em 14/10 09:30");
    const bare = agentAlertToast(
      { kind: "usage_limit", session_id: "s1", window: "seven_day", pct: 90, resets_at: null },
      null,
      "pt-BR",
      t,
    );
    expect(bare.title).toBe("Limite de 7 dias em 90%");
  });
});

describe("o uso atravessa o merge de sessão", () => {
  const session = (over: Partial<Session> = {}): Session =>
    ({
      id: "s1",
      kind: { type: "agent", runner: "claude_code" },
      title: "rio-api",
      repo_root: null,
      worktree: null,
      status: { state: "running" },
      attention: false,
      created_at: "",
      ...over,
    }) as Session;

  test("update que só mexe no uso não é descartado", () => {
    const merged = mergeSessionUpdate(session(), session({ agent_usage: full }));
    expect(merged?.agent_usage).toEqual(full);
  });

  test("mesmo uso não é novidade", () => {
    expect(
      mergeSessionUpdate(
        session({ agent_usage: full }),
        session({ agent_usage: { ...full } }),
      ),
    ).toBeNull();
  });

  test("igualdade por campo, janelas incluídas", () => {
    const a: AgentUsage = { rate_5h: { pct: 10, resets_at: 1 } };
    expect(sameAgentUsage(a, { rate_5h: { pct: 10, resets_at: 1 } })).toBe(true);
    expect(sameAgentUsage(a, { rate_5h: { pct: 11, resets_at: 1 } })).toBe(false);
    expect(sameAgentUsage(null, undefined)).toBe(true);
    expect(sameAgentUsage(a, null)).toBe(false);
  });
});
