import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowsClockwise,
  ChatTeardrop,
  GithubLogo,
  GitMerge,
  GitPullRequest,
  MagnifyingGlass,
  Plus,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { useRelative } from "./GitHistoryView";
import { OpenPrDialog } from "./OpenPrDialog";
import { CiTab } from "./ForgePanel";
import { translateError } from "../lib/errors";
import { primaryDeliveryAction } from "../lib/forge";
import {
  forgePrPage,
  forgeStatus as fetchForgeStatus,
  type ForgeStatus,
  type PrFilter,
  type PrListItem,
  type PrPage,
  type Session,
  type SessionId,
} from "../lib/ipc";
import { ciCell, initials, matchesPr, reviewChips, type ReviewChip, type Tone } from "../lib/prs";

const TONE_TEXT: Record<Tone, string> = {
  green: "text-tyba-green",
  amber: "text-tyba-amber",
  red: "text-tyba-red",
  muted: "text-tyba-text-faint",
  violet: "text-tyba-violet",
};

const CHIP: Record<ReviewChip["tone"], string> = {
  ok: "bg-tyba-green/[.12] text-tyba-green",
  wait: "bg-tyba-amber/[.12] text-tyba-amber",
  changes: "bg-tyba-red/[.12] text-tyba-red",
  draft: "bg-tyba-text/[.06] text-tyba-text-muted",
};

/** Grade da lista (mockup GitPRs): ícone · título · CI · revisão · comentários · alterações. */
const GRID =
  "grid grid-cols-[16px_minmax(0,1fr)_130px_210px_44px_96px] items-center gap-x-3.5 px-4";

/** "ci" é da tela (runs de CI por branch, o que o popover antigo mostrava); não vai ao core. */
type ListFilter = PrFilter | "ci";

const FILTERS: { key: ListFilter; label: string; count?: keyof PrPage["counts"] }[] = [
  { key: "open", label: "prFilterOpen", count: "open" },
  { key: "mine", label: "prFilterMine", count: "mine" },
  { key: "review", label: "prFilterReview", count: "review" },
  { key: "closed", label: "prFilterClosed" },
  { key: "ci", label: "prFilterCi" },
];

const REFRESH_MS = 60_000;

function StateIcon({ state }: { state: PrListItem["state"] }) {
  if (state === "merged") return <GitMerge size={16} className="text-tyba-violet" aria-hidden />;
  const tone =
    state === "draft" ? "text-tyba-text-faint" : state === "closed" ? "text-tyba-red" : "text-tyba-green";
  return <GitPullRequest size={16} className={tone} aria-hidden />;
}

function Avatar({ name, agent }: { name: string; agent: boolean }) {
  return (
    <span
      className={`flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full px-1 text-[9px] ${
        agent ? "bg-tyba-amber/[.16] text-tyba-amber" : "bg-tyba-blue/[.18] text-tyba-blue"
      }`}
    >
      {initials(name)}
    </span>
  );
}

/**
 * Aba "git · pull requests" (mockup GitPRs). Só leitura: a lista vem do core
 * com cache; abrir uma linha abre a aba do PR. Criar PR usa o diálogo que já
 * existe (aprovação do usuário toda vez).
 */
export function PrListView({
  session,
  onFocusSession,
  onOpenPr,
}: {
  session: Session;
  onFocusSession: (id: SessionId) => void;
  onOpenPr: (number: number) => void;
}) {
  const { t } = useTranslation();
  const relative = useRelative();
  const [filter, setFilter] = useState<ListFilter>("open");
  const [page, setPage] = useState<PrPage | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [now, setNow] = useState(() => Date.now());
  const [status, setStatus] = useState<ForgeStatus | null>(null);
  const [creating, setCreating] = useState(false);

  const load = useCallback(
    (fresh: boolean) => {
      setLoading(true);
      forgePrPage(session.id, filter === "ci" ? "open" : filter, fresh)
        .then((p) => {
          setPage(p);
          setError(null);
        })
        .catch((e) => setError(translateError(e, t)))
        .finally(() => setLoading(false));
    },
    [session.id, filter, t],
  );

  useEffect(() => {
    load(false);
    const timer = window.setInterval(() => load(true), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [load]);

  // Relógio do "atualizado há".
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 10_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    void fetchForgeStatus(session.id)
      .then(setStatus)
      .catch(() => setStatus(null));
  }, [session.id]);

  const items = useMemo(
    () => (page?.items ?? []).filter((item) => matchesPr(item, query)),
    [page, query],
  );
  const merged = useMemo(
    () => (page?.recent_merged ?? []).filter((item) => matchesPr(item, query)),
    [page, query],
  );
  const drafts = items.filter((i) => i.state === "draft").length;

  const row = (item: PrListItem, dim = false) => {
    const ci = ciCell(item.ci);
    const chips = reviewChips(item);
    const agent = item.session?.agent ?? null;
    return (
      <div
        key={item.number}
        role="button"
        tabIndex={0}
        onClick={() => onOpenPr(item.number)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onOpenPr(item.number);
        }}
        className={`${GRID} group/pr cursor-pointer tyba-divide-b py-3 text-[13px] outline-none hover:bg-tyba-raised hover:shadow-[var(--tyba-row-raised)] focus-visible:bg-tyba-raised ${
          dim ? "opacity-75" : ""
        }`}
      >
        <StateIcon state={item.state} />
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <span
              className={`truncate ${item.state === "draft" ? "text-tyba-text-muted" : "text-tyba-text"}`}
              title={item.title}
            >
              {item.title}
            </span>
            <span className="shrink-0 text-tyba-text-faint">#{item.number}</span>
            {agent && (
              <span className="shrink-0 rounded-[4px] bg-tyba-amber/[.12] px-1.5 text-[10px] text-tyba-amber">
                {agent}
              </span>
            )}
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-2.5 text-[11px] text-tyba-text-faint">
            <Avatar name={agent ? agent : (item.author_name ?? item.author)} agent={Boolean(agent)} />
            <span className="truncate">
              {item.session && agent
                ? t("prBySession", { title: item.session.title })
                : (item.author_name ?? item.author)}
            </span>
            <span className="truncate font-mono text-tyba-blue/80">
              {item.head} → {item.base}
            </span>
            <span className="shrink-0">
              {item.state === "merged" && item.merged_at
                ? t("prMergedAgo", {
                    ago: relative(item.merged_at, now),
                    by: item.merged_by ?? "",
                  })
                : t("prUpdatedAgo", { ago: relative(item.updated_at, now) })}
            </span>
            {item.session && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  if (item.session) onFocusSession(item.session.session);
                }}
                className="hidden shrink-0 text-tyba-blue hover:underline group-hover/pr:inline"
              >
                {t("prOpenInTyba")}
              </button>
            )}
          </div>
        </div>
        <span className={`truncate text-[12px] ${TONE_TEXT[ci.tone]}`}>
          {t(ci.key, { count: ci.count, total: ci.total })}
        </span>
        <span className="flex min-w-0 flex-wrap gap-1.5">
          {chips.map((chip) => (
            <span key={chip.key} className={`rounded-[4px] px-1.5 text-[11px] ${CHIP[chip.tone]}`}>
              {t(chip.key, { count: chip.count ?? 0 })}
            </span>
          ))}
        </span>
        <span
          className={`flex items-center gap-1 text-[12px] ${
            item.comments > 0 ? "text-tyba-text-muted" : "text-tyba-text-faint"
          }`}
        >
          <ChatTeardrop size={12} aria-hidden />
          {item.comments}
        </span>
        <span className="text-right font-mono text-[12px]">
          <span className="text-tyba-green">+{item.additions}</span>{" "}
          <span className="text-tyba-red">−{item.deletions}</span>
        </span>
      </div>
    );
  };

  return (
    <div className="absolute inset-0 flex min-w-0 flex-col bg-tyba-sunken">
      <header className="flex h-12 shrink-0 items-center gap-2.5 tyba-divide-b bg-[var(--tyba-panel-bg)] px-4 text-[12px] text-tyba-text-muted">
        <div role="tablist" className="flex gap-0.5 rounded-[7px] bg-tyba-sunken p-0.5 shadow-[inset_0_0_0_1px_var(--tyba-border)]">
          {FILTERS.map((f) => {
            const on = filter === f.key;
            const count = f.count && page ? page.counts[f.count] : null;
            return (
              <button
                key={f.key}
                role="tab"
                aria-selected={on}
                onClick={() => setFilter(f.key)}
                className={`flex items-center gap-1.5 rounded-[5px] px-2.5 py-1 ${
                  on ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]" : "hover:text-tyba-text"
                }`}
              >
                {t(f.label)}
                {count !== null && count > 0 && (
                  <span className={f.key === "review" ? "text-tyba-amber" : "text-tyba-text-muted"}>{count}</span>
                )}
              </button>
            );
          })}
        </div>
        <label className="flex w-[280px] items-center gap-1.5 rounded-[6px] bg-tyba-sunken px-2.5 shadow-[inset_0_0_0_1px_var(--tyba-border-field)]">
          <MagnifyingGlass size={12} className="shrink-0 text-tyba-text-faint" aria-hidden />
          <input
            type="text"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("prSearchPlaceholder")}
            className="min-w-0 flex-1 bg-transparent py-1.5 text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </label>
        {status && (
          <Button
            size="sm"
            className="h-auto gap-1.5 rounded-[6px] bg-tyba-green px-3 py-1.5 text-[12px] font-normal text-tyba-text-inverse hover:bg-tyba-green/90"
            onClick={() => setCreating(true)}
          >
            <Plus size={12} weight="bold" />
            {t("prNew")}
          </Button>
        )}
        <span className="flex-1" />
        {page && (
          <span className="flex min-w-0 items-center gap-1.5 truncate">
            <GithubLogo size={13} className="shrink-0" aria-hidden />
            GitHub · <span className="truncate text-tyba-text">{page.repo}</span>
          </span>
        )}
        {page && (
          <button
            onClick={() => load(true)}
            disabled={loading}
            title={t("prRefresh")}
            className="flex shrink-0 items-center gap-1 text-[11px] text-tyba-text-faint hover:text-tyba-text"
          >
            {loading && <ArrowsClockwise size={11} className="animate-spin" />}
            {t("prFetchedAgo", { ago: relative(page.fetched_at, now) })}
          </button>
        )}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {page === undefined && !error && (
          <p className="p-6 text-[12px] text-tyba-text-faint">{t("diffLoading")}</p>
        )}
        {error && <p className="p-6 text-[12px] text-tyba-red">{error}</p>}
        {page === null && !error && (
          <p className="p-6 text-[12px] text-tyba-text-faint">{t("prListUnsupported")}</p>
        )}
        {page && filter === "ci" && <CiTab sessionId={session.id} wide />}
        {page && filter !== "ci" && (
          <>
            <div className={`${GRID} tyba-divide-b py-2 text-[11px] text-tyba-text-faint`}>
              <span />
              <span>
                {filter === "open"
                  ? t("prListSummary", { count: items.length, drafts })
                  : t("prListCount", { count: items.length })}
              </span>
              <span>CI</span>
              <span>{t("prColReview")}</span>
              <span />
              <span className="text-right">{t("prColChanges")}</span>
            </div>
            {items.map((item) => row(item))}
            {items.length === 0 && (
              <p className="px-4 py-6 text-[12px] text-tyba-text-faint">{t("prListEmpty")}</p>
            )}
            {merged.length > 0 && (
              <>
                <div className="px-4 pb-1.5 pt-3.5 text-[11px] text-tyba-text-faint">{t("prRecentMerged")}</div>
                {merged.map((item) => row(item, true))}
              </>
            )}
          </>
        )}
      </div>

      <footer className="flex shrink-0 justify-between gap-4 tyba-divide-t bg-[var(--tyba-panel-bg)] px-4 py-2.5 text-[11px] text-tyba-text-faint">
        <span>{t("prListNote")}</span>
        <span>{t("prListNotePush")}</span>
      </footer>

      {status && (
        <OpenPrDialog
          session={session}
          status={status}
          action={primaryDeliveryAction(status) === "open_mr" ? "open_mr" : "open_pr"}
          open={creating}
          onClose={() => setCreating(false)}
          onCreated={(created) => {
            setCreating(false);
            load(true);
            onOpenPr(created.number);
          }}
        />
      )}
    </div>
  );
}
