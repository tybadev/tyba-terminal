import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, ChartBar } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { agentStats, type AgentStats, type RiskLevel } from "@/lib/ipc";
import {
  ALL_REPOS,
  STATS_PERIODS,
  formatCount,
  formatDuration,
  formatPercent,
  type StatsPeriod,
} from "@/lib/stats";
import { basename } from "@/lib/utils";

const RISK_KEY: Record<RiskLevel, string> = {
  green: "riskGreen",
  yellow: "riskYellow",
  red: "riskRed",
};

const RISK_CLASS: Record<RiskLevel, string> = {
  green: "text-tyba-green",
  yellow: "text-tyba-amber",
  red: "text-tyba-red",
};

const PERIOD_KEY: Record<string, string> = {
  "7": "statsPeriod7",
  "30": "statsPeriod30",
  all: "statsPeriodAll",
};

function periodKey(period: StatsPeriod): string {
  return period === null ? "all" : String(period);
}

function StatCard({
  label,
  value,
  detail,
  hint,
}: {
  label: string;
  value: string;
  detail?: string;
  hint?: string;
}) {
  return (
    <div className="flex flex-col gap-1 rounded-[8px] border border-tyba-border px-3.5 py-3">
      <span className="tyba-label">{label}</span>
      <span className="flex items-baseline gap-1.5">
        <span className="font-mono text-[20px] leading-none text-tyba-text">
          {value}
        </span>
        {detail && (
          <span className="font-mono text-[12px] text-tyba-text-muted">
            {detail}
          </span>
        )}
      </span>
      {hint && (
        <span className="text-[10px] leading-snug text-tyba-text-faint">
          {hint}
        </span>
      )}
    </div>
  );
}

/** Cabeçalho de tabela: as colunas de número alinham à direita. */
function HeadCell({
  children,
  numeric,
}: {
  children: React.ReactNode;
  numeric?: boolean;
}) {
  return (
    <span className={`tyba-label ${numeric ? "text-right" : ""}`}>
      {children}
    </span>
  );
}

export function StatsView() {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [period, setPeriod] = useState<StatsPeriod>(30);
  const [repo, setRepo] = useState<string>(ALL_REPOS);
  const [stats, setStats] = useState<AgentStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await agentStats(period, repo === ALL_REPOS ? null : repo);
      setStats(next);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [period, repo]);

  useEffect(() => {
    void load();
  }, [load]);

  // O repo escolhido pode não ter atividade na janela nova — sem isto o filtro
  // ficaria mostrando um repo que sumiu da lista, e a tela vazia pareceria bug.
  useEffect(() => {
    if (!stats || repo === ALL_REPOS) return;
    if (!stats.repos.includes(repo)) setRepo(ALL_REPOS);
  }, [stats, repo]);

  const totals = stats?.totals;
  const isEmpty =
    !loading &&
    error === null &&
    stats !== null &&
    stats.totals.requested === 0 &&
    stats.sessions.length === 0;

  // O nome da pasta basta para escolher, MENOS quando dois repos têm o mesmo —
  // dois clones de `tyba` em lugares diferentes virariam duas opções idênticas
  // e o filtro escolheria às cegas. Nesses, o caminho inteiro.
  const repoOptions = useMemo(() => {
    const roots = stats?.repos ?? [];
    const seen = new Map<string, number>();
    for (const root of roots) {
      const name = basename(root) || root;
      seen.set(name, (seen.get(name) ?? 0) + 1);
    }
    return [
      { value: ALL_REPOS, label: t("statsAllRepos") },
      ...roots.map((root) => {
        const name = basename(root) || root;
        return { value: root, label: (seen.get(name) ?? 0) > 1 ? root : name };
      }),
    ];
  }, [stats, t]);

  return (
    <div className="flex min-h-0 flex-1 justify-center overflow-y-auto">
      <div className="flex w-full max-w-5xl flex-col px-6 pt-5 pb-8">
        <div className="flex items-center justify-between pb-1">
          <span className="tyba-label">{t("statsTitle")}</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 gap-1.5 px-2 text-[11px] text-tyba-text-muted"
            onClick={() => void load()}
          >
            <ArrowClockwise size={12} />
            {t("statsRefresh")}
          </Button>
        </div>
        <p className="pb-4 text-[12px] text-tyba-text-faint">{t("statsHint")}</p>

        <div className="flex flex-wrap items-center gap-2 pb-4">
          <div className="flex items-center gap-px rounded-[5px] border border-tyba-border p-0.5">
            {STATS_PERIODS.map((option) => (
              <button
                key={periodKey(option)}
                type="button"
                aria-pressed={option === period}
                onClick={() => setPeriod(option)}
                className={`rounded-[3px] px-2 py-1 text-[11px] transition-colors ${
                  option === period
                    ? "bg-tyba-text/[.06] text-tyba-text"
                    : "text-tyba-text-faint hover:text-tyba-text"
                }`}
              >
                {t(PERIOD_KEY[periodKey(option)])}
              </button>
            ))}
          </div>
          <Select
            value={repo}
            options={repoOptions}
            onChange={setRepo}
            className="h-7 min-w-[180px] text-[12px]"
          />
        </div>

        {error !== null ? (
          <div className="flex flex-col items-center gap-3 px-3 py-8 text-center">
            <p className="text-xs text-tyba-text-faint">{t("statsLoadError")}</p>
            <p className="max-w-full truncate font-mono text-[10px] text-tyba-text-faint">
              {error}
            </p>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void load()}
              className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
            >
              {t("retry")}
            </Button>
          </div>
        ) : stats === null ? (
          <div className="flex flex-col gap-2">
            {[0, 1, 2].map((i) => (
              <div
                key={i}
                className="h-16 animate-pulse rounded-[8px] bg-tyba-text/[.03]"
              />
            ))}
          </div>
        ) : isEmpty ? (
          <div className="flex flex-col items-center gap-2 px-3 py-12 text-center">
            <ChartBar size={22} className="text-tyba-text-faint" />
            <p className="text-xs text-tyba-text-faint">{t("statsEmpty")}</p>
            <p className="max-w-xs text-[11px] text-tyba-text-faint">
              {t("statsEmptyHint")}
            </p>
          </div>
        ) : (
          totals && (
            <div className="flex flex-col gap-6">
              <div className="grid gap-2.5 [grid-template-columns:repeat(auto-fill,minmax(190px,1fr))]">
                <StatCard
                  label={t("statsRequested")}
                  value={formatCount(totals.requested, locale)}
                />
                <StatCard
                  label={t("statsAuto")}
                  value={formatCount(totals.auto_approved, locale)}
                  detail={formatPercent(totals.auto_approved_pct, locale)}
                  hint={t("statsAutoHint")}
                />
                <StatCard
                  label={t("statsHuman")}
                  value={formatCount(totals.human_decided, locale)}
                  detail={formatPercent(totals.human_decided_pct, locale)}
                  hint={t("statsHumanHint")}
                />
                <StatCard
                  label={t("statsDenied")}
                  value={formatCount(totals.denied, locale)}
                  detail={formatPercent(totals.denied_pct, locale)}
                />
                <StatCard
                  label={t("statsMedian")}
                  value={formatDuration(totals.median_human_ms, locale)}
                  hint={t("statsMedianHint")}
                />
              </div>

              <section className="flex flex-col gap-2">
                <span className="text-[13px] text-tyba-text">
                  {t("statsCommandsTitle")}
                </span>
                {stats.commands.length === 0 ? (
                  <div className="rounded-[8px] border border-dashed border-tyba-border px-4 py-6 text-center text-[12px] text-tyba-text-faint">
                    {t("statsNoCommands")}
                  </div>
                ) : (
                  <div className="overflow-hidden rounded-[8px] border border-tyba-border">
                    <div className="grid grid-cols-[1fr_auto_88px_110px] items-center gap-3 tyba-divide-b px-4 py-2">
                      <HeadCell>{t("statsColCommand")}</HeadCell>
                      <HeadCell numeric>{t("statsColRequests")}</HeadCell>
                      <HeadCell numeric>{t("statsColRisk")}</HeadCell>
                      <HeadCell numeric>{t("statsColApprovalRate")}</HeadCell>
                    </div>
                    <div className="divide-y divide-tyba-border">
                      {stats.commands.map((row) => (
                        <div
                          key={row.command}
                          className="grid grid-cols-[1fr_auto_88px_110px] items-center gap-3 px-4 py-2"
                        >
                          <span
                            title={row.command}
                            className="min-w-0 truncate font-mono text-[12px] text-tyba-text"
                          >
                            {row.command}
                          </span>
                          <span className="text-right font-mono text-[12px] text-tyba-text-muted">
                            {formatCount(row.requests, locale)}
                          </span>
                          <span
                            className={`text-right text-[11px] ${RISK_CLASS[row.risk]}`}
                          >
                            {t(RISK_KEY[row.risk])}
                          </span>
                          <span className="text-right font-mono text-[12px] text-tyba-text-muted">
                            {formatPercent(row.approval_rate, locale)}
                          </span>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </section>

              <section className="flex flex-col gap-2">
                <span className="text-[13px] text-tyba-text">
                  {t("statsSessionsTitle")}
                </span>
                {stats.sessions.length === 0 ? (
                  <div className="rounded-[8px] border border-dashed border-tyba-border px-4 py-6 text-center text-[12px] text-tyba-text-faint">
                    {t("statsNoSessions")}
                  </div>
                ) : (
                  <div className="overflow-hidden rounded-[8px] border border-tyba-border">
                    <div className="grid grid-cols-[1fr_96px_96px_96px] items-center gap-3 tyba-divide-b px-4 py-2">
                      <HeadCell>{t("statsColSession")}</HeadCell>
                      <HeadCell numeric>{t("statsColCommandsRun")}</HeadCell>
                      <HeadCell numeric>{t("statsColApprovals")}</HeadCell>
                      <HeadCell numeric>{t("statsColTotalTime")}</HeadCell>
                    </div>
                    <div className="divide-y divide-tyba-border">
                      {stats.sessions.map((row) => (
                        <div
                          key={row.session_id}
                          className="grid grid-cols-[1fr_96px_96px_96px] items-center gap-3 px-4 py-2"
                        >
                          <span
                            title={row.title}
                            className="min-w-0 truncate text-[12px] text-tyba-text"
                          >
                            {row.title}
                          </span>
                          <span className="text-right font-mono text-[12px] text-tyba-text-muted">
                            {formatCount(row.commands, locale)}
                          </span>
                          <span className="text-right font-mono text-[12px] text-tyba-text-muted">
                            {formatCount(row.approvals, locale)}
                          </span>
                          <span className="text-right font-mono text-[12px] text-tyba-text-muted">
                            {formatDuration(row.total_ms, locale)}
                          </span>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </section>
            </div>
          )
        )}
      </div>
    </div>
  );
}
