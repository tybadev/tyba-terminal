import type { AgentAlertPayload, AgentUsage, RateWindow } from "./ipc";
import type { ToastInput } from "./toast";
import { formatDuration } from "./blockLayout";

type Translate = (key: string, options?: Record<string, unknown>) => string;

/**
 * A linha de uso do resumo da sessão no painel de agentes:
 * "Opus 5.5 — contexto 13%, 48 mil tokens, US$ 0,42, 4m12s".
 *
 * Só os campos que o core mandou; nada é inventado. `null` quando não há
 * nada a mostrar. Os números chegam prontos do core — aqui é só formatação.
 */
export function usageSummary(
  usage: AgentUsage | null | undefined,
  locale: string,
  t: Translate,
): string | null {
  if (!usage) return null;
  const parts: string[] = [];
  if (usage.context_pct != null) {
    parts.push(t("agentsUsageContext", { pct: Math.round(usage.context_pct) }));
  }
  const tokens = (usage.tokens_in ?? 0) + (usage.tokens_out ?? 0);
  if (usage.tokens_in != null || usage.tokens_out != null) {
    parts.push(
      t("agentsUsageTokens", {
        count: tokens,
        tokens: new Intl.NumberFormat(locale, { notation: "compact" }).format(tokens),
      }),
    );
  }
  if (usage.cost_usd != null) {
    parts.push(
      new Intl.NumberFormat(locale, { style: "currency", currency: "USD" }).format(
        usage.cost_usd,
      ),
    );
  }
  if (usage.duration_ms != null) {
    const took = formatDuration(usage.duration_ms, locale, { whole: true });
    if (took) parts.push(took);
  }
  const rest = parts.join(", ");
  if (usage.model && rest) return `${usage.model} — ${rest}`;
  return usage.model || rest || null;
}

export const sameRateWindow = (
  a: RateWindow | null | undefined,
  b: RateWindow | null | undefined,
): boolean => {
  if (!a || !b) return !a && !b;
  return a.pct === b.pct && (a.resets_at ?? null) === (b.resets_at ?? null);
};

/** Igualdade rasa por campo — o merge de sessão usa para não perder uma
 * atualização que só mexeu no uso. */
export const sameAgentUsage = (
  a: AgentUsage | null | undefined,
  b: AgentUsage | null | undefined,
): boolean => {
  if (!a || !b) return !a && !b;
  return (
    a.model === b.model &&
    a.context_pct === b.context_pct &&
    a.context_window_size === b.context_window_size &&
    a.tokens_in === b.tokens_in &&
    a.tokens_out === b.tokens_out &&
    a.cost_usd === b.cost_usd &&
    a.duration_ms === b.duration_ms &&
    a.lines_added === b.lines_added &&
    a.lines_removed === b.lines_removed &&
    sameRateWindow(a.rate_5h, b.rate_5h) &&
    sameRateWindow(a.rate_7d, b.rate_7d)
  );
};

/** "18:00", ou "14/10 18:00" com `withDate` — a janela de 7 dias reseta
 * em outro dia; a de 5h, sempre nas próximas horas. Fuso local. */
export function formatResetAt(
  resetsAtSecs: number,
  locale: string,
  withDate: boolean,
): string {
  const at = new Date(resetsAtSecs * 1000);
  const time = new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(at);
  if (!withDate) return time;
  const day = new Intl.DateTimeFormat(locale, {
    day: "2-digit",
    month: "2-digit",
  }).format(at);
  return `${day} ${time}`;
}

/**
 * O toast de `agent://alert`. `sessionTitle` entra no detalhe para quem tem
 * várias sessões abertas saber de qual veio. Falha de turno é `error` e fica
 * até o dono fechar; limite de uso é `warning` e some sozinho.
 */
export function agentAlertToast(
  alert: AgentAlertPayload,
  sessionTitle: string | null,
  locale: string,
  t: Translate,
): ToastInput {
  if (alert.kind === "stop_failure") {
    return {
      tone: "error",
      title: t("agentAlertStopFailure", { reason: alert.reason }),
      detail: sessionTitle ?? undefined,
      sticky: true,
    };
  }
  const pct = Math.round(alert.pct);
  const fiveHour = alert.window === "five_hour";
  const windowKey = fiveHour ? "agentAlertLimit5h" : "agentAlertLimit7d";
  const title =
    alert.resets_at != null
      ? t(`${windowKey}Resets`, {
          pct,
          time: formatResetAt(alert.resets_at, locale, !fiveHour),
        })
      : t(windowKey, { pct });
  return {
    tone: "warning",
    title,
    detail: sessionTitle ?? undefined,
  };
}
