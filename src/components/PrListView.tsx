import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { Input } from "@/components/ui/input";
import { SegmentedControl } from "@/components/ui/segmented";
import { useRelative } from "./GitHistoryView";
import { OpenPrDialog } from "./OpenPrDialog";
import { CiTab } from "./ForgePanel";
import { ForgeLoading } from "./ForgeLoading";
import { translateError } from "../lib/errors";
import { primaryDeliveryAction } from "../lib/forge";
import {
  forgePrPage,
  forgeStatus as fetchForgeStatus,
  onForgeChanged,
  type ForgeStatus,
  type PrFilter,
  type PrListItem,
  type PrPage,
  type Session,
  type SessionId,
} from "../lib/ipc";
import {
  ciCell,
  forgeChangeHits,
  initials,
  linkedSession,
  matchesPr,
  nextPrLimit,
  pageVisible,
  PR_PAGE_LIMIT,
  prCountLabel,
  rateLimitNotice,
  reviewChips,
  type ReviewChip,
  type Tone,
} from "../lib/prs";

const TONE_TEXT: Record<Tone, string> = {
  green: "text-tyba-green",
  amber: "text-tyba-amber",
  red: "text-tyba-red",
  muted: "text-tyba-text-faint",
  violet: "text-tyba-violet",
};

const CHIP: Record<ReviewChip["tone"], string> = {
  ok: "bg-tyba-green/[.12] text-tyba-green",
  wait: "bg-tyba-amber-tint text-tyba-amber",
  changes: "bg-tyba-red-tint text-tyba-red",
  draft: "bg-tyba-text/[.06] text-tyba-text-muted",
};

/** Grade da lista (mockup GitPRs): ícone · título · CI · revisão · comentários · alterações. */
const GRID =
  "grid grid-cols-[16px_minmax(0,1fr)_130px_210px_44px_96px] items-center gap-x-3.5 px-2";

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

function Avatar({ name }: { name: string }) {
  return (
    <span className="flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-tyba-blue/[.14] px-1 text-[8.5px] font-bold text-tyba-blue">
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
  const { t, i18n } = useTranslation();
  const relative = useRelative();
  const [filter, setFilter] = useState<ListFilter>("open");
  const [page, setPage] = useState<PrPage | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // Só o clique em "carregar mais" acende o rótulo de "buscando mais": o poll
  // e o `forge://changed` também ligam `loading`, sem o usuário ter pedido.
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadAt, setLoadAt] = useState(() => Date.now());
  const [query, setQuery] = useState("");
  const [now, setNow] = useState(() => Date.now());
  const [status, setStatus] = useState<ForgeStatus | null>(null);
  const [creating, setCreating] = useState(false);
  // "Carregar mais" sobe o limit de 100 em 100; trocar de filtro volta a 100.
  const [limit, setLimit] = useState(PR_PAGE_LIMIT);
  // Só a resposta do pedido mais novo entra: trocar de filtro rápido não deixa
  // a lista de um filtro aparecer sob o rótulo de outro.
  const requestSeq = useRef(0);

  const load = useCallback(
    (fresh: boolean) => {
      const seq = ++requestSeq.current;
      setLoading(true);
      setLoadAt(Date.now());
      forgePrPage(session.id, filter === "ci" ? "open" : filter, fresh, limit)
        .then((p) => {
          if (seq !== requestSeq.current) return;
          setPage(p);
          setError(null);
        })
        .catch((e) => seq === requestSeq.current && setError(translateError(e, t)))
        .finally(() => {
          if (seq !== requestSeq.current) return;
          setLoading(false);
          setLoadingMore(false);
        });
    },
    [session.id, filter, limit, t],
  );

  // O poll pede o cache (`fresh: false`): o core responde na hora e, se o
  // dado venceu, revalida em segundo plano e avisa por `forge://changed`.
  // Só roda com esta tela montada (tela de Git, ou aba antiga `pr-list`
  // restaurada do layout salvo) e com a janela visível.
  useEffect(() => {
    load(false);
    const timer = window.setInterval(() => {
      if (pageVisible()) load(false);
    }, REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [load]);

  const repo = page?.repo ?? null;
  useEffect(() => {
    if (repo === null) return;
    const unlisten = onForgeChanged((change) => {
      if (forgeChangeHits(change, repo, null)) load(false);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [repo, load]);

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
  const rateNotice = page?.rate_limit ? rateLimitNotice(page.rate_limit, i18n.language) : null;
  const drafts = items.filter((i) => i.state === "draft").length;

  const row = (item: PrListItem, dim = false) => {
    const ci = ciCell(item.ci);
    const chips = reviewChips(item);
    // O autor real aparece sempre (segurança: quem escreveu o PR não some
    // atrás do nome de uma sessão). A sessão é complemento, e só vale quando o
    // PR não vem de fork.
    const linked = linkedSession(item);
    const agent = linked?.agent ?? null;
    const author = item.author_name ?? item.author;
    return (
      <div
        key={item.number}
        role="button"
        tabIndex={0}
        onClick={() => onOpenPr(item.number)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onOpenPr(item.number);
        }}
        className={`${GRID} group/pr cursor-pointer shadow-[inset_0_-1px_0_var(--tyba-line)] py-3 text-[13.5px] outline-none hover:bg-[color-mix(in_srgb,var(--tyba-glass-active)_57%,transparent)] focus-visible:bg-[color-mix(in_srgb,var(--tyba-glass-active)_57%,transparent)] ${
          dim ? "opacity-75" : ""
        }`}
      >
        <StateIcon state={item.state} />
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2 font-semibold">
            <span
              className={`truncate ${item.state === "draft" ? "text-tyba-text-muted" : "text-tyba-text"}`}
              title={item.title}
            >
              {item.title}
            </span>
            <span className="shrink-0 font-medium text-tyba-text-faint">#{item.number}</span>
            {agent && (
              <span className="shrink-0 rounded-full bg-tyba-amber-tint px-2 py-px text-[11px] font-semibold text-tyba-amber">
                {agent}
              </span>
            )}
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-1.5 whitespace-nowrap text-[12px] text-tyba-text-faint">
            <Avatar name={author} />
            <span className="truncate">
              {author}
              {linked && <> · {t("prBySession", { title: linked.title })}</>}
            </span>
            <span className="truncate font-mono text-[11.5px] text-tyba-blue">
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
            {linked && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onFocusSession(linked.session);
                }}
                className="hidden shrink-0 text-tyba-blue hover:underline group-hover/pr:inline"
              >
                {t("prOpenInTyba")}
              </button>
            )}
          </div>
        </div>
        <span className={`truncate text-[12.5px] font-semibold ${TONE_TEXT[ci.tone]}`}>
          {t(ci.key, { count: ci.count, total: ci.total })}
        </span>
        <span className="flex min-w-0 flex-wrap gap-1.5">
          {chips.map((chip) => (
            <span key={chip.key} className={`rounded-full px-[9px] py-0.5 text-[11.5px] font-semibold ${CHIP[chip.tone]}`}>
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
          <span className="font-semibold text-tyba-green">+{item.additions}</span>{" "}
          <span className="font-semibold text-tyba-red">−{item.deletions}</span>
        </span>
      </div>
    );
  };

  return (
    <div className="absolute inset-0 flex min-w-0 flex-col">
      <header className="flex h-[52px] shrink-0 items-center gap-3 whitespace-nowrap shadow-[inset_0_-1px_0_var(--tyba-line)] px-[18px] text-[12px] text-tyba-text-muted">
        {/* Quadro PRs: o filtro é o segmentado do Muriki, com a contagem no rótulo. */}
        <SegmentedControl
          tone="tint"
          aria-label={t("notifFilter")}
          value={filter}
          onChange={(key) => {
            setFilter(key);
            setLimit(PR_PAGE_LIMIT);
          }}
          options={FILTERS.map((f) => {
            const count = f.count && page ? page.counts[f.count] : null;
            return {
              value: f.key,
              label: (
                <>
                  {t(f.label)}
                  {count !== null && count > 0 && (
                    <span className={f.key === "review" ? "text-tyba-amber" : "text-tyba-text-muted"}>
                      {prCountLabel(count, page?.capped ?? false)}
                    </span>
                  )}
                </>
              ),
            };
          })}
        />
        <Input
          icon={<MagnifyingGlass size={16} aria-hidden />}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("prSearchPlaceholder")}
          aria-label={t("prSearchPlaceholder")}
          className="w-[240px] shrink-0"
        />
        {status && (
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus weight="bold" />
            {t("prNew")}
          </Button>
        )}
        <span className="flex-1" />
        {page && (
          <span className="flex min-w-0 items-center gap-1.5 text-[12.5px]" title={`GitHub · ${page.repo}`}>
            <GithubLogo size={13} className="shrink-0" aria-hidden />
            <span className="truncate">{page.repo.split("/").pop()}</span>
          </span>
        )}
        {page && (
          <button
            onClick={() => load(true)}
            disabled={loading}
            title={t("prRefresh")}
            className="flex shrink-0 items-center gap-[5px] text-[12px] text-tyba-text-faint hover:text-tyba-text"
          >
            <ArrowsClockwise size={12} className={loading ? "animate-spin" : undefined} aria-hidden />
            {t("prFetchedAgo", { ago: relative(page.fetched_at, now) })}
          </button>
        )}
      </header>

      {rateNotice && (
        <p role="status" className="shrink-0 shadow-[inset_0_-1px_0_var(--tyba-line)] px-4 py-1.5 text-[11px] text-tyba-amber">
          {t(rateNotice.key, rateNotice.params)}
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-[18px] py-1">
        {page === undefined && (
          <ForgeLoading
            step={
              status
                ? t(status.kind === "gitlab" ? "loadForgeMrs" : "loadForgePrs", {
                    forge: status.kind === "gitlab" ? "GitLab" : "GitHub",
                  })
                : t("loadForgePrsAny")
            }
            em={["GitHub", "GitLab"]}
            startedAt={loadAt}
            error={error}
            onRetry={() => {
              setError(null);
              load(true);
            }}
            shape="prs"
            rows={5}
          />
        )}
        {error && page !== undefined && <p className="p-6 text-[12px] text-tyba-red">{error}</p>}
        {page === null && !error && (
          <p className="p-6 text-[12px] text-tyba-text-faint">{t("prListUnsupported")}</p>
        )}
        {page && filter === "ci" && <CiTab sessionId={session.id} wide />}
        {page && filter !== "ci" && (
          <>
            <div className={`${GRID} shadow-[inset_0_-1px_0_var(--tyba-line)] pb-2 pt-2.5 text-[11.5px] text-tyba-text-faint`}>
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
              <p className="px-2 py-6 text-[12px] text-tyba-text-faint">{t("prListEmpty")}</p>
            )}
            {page.capped && nextPrLimit(limit) !== null && (
              <div className="flex justify-center pb-1.5 pt-3.5">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={loading}
                  onClick={() => {
                    setLoadingMore(true);
                    setLimit((l) => nextPrLimit(l) ?? l);
                  }}
                >
                  {loadingMore
                    ? t("loadForgeMore", { count: (nextPrLimit(limit) ?? limit) - limit })
                    : t("prLoadMore")}
                </Button>
              </div>
            )}
            {merged.length > 0 && (
              <>
                <div className="px-2 pb-1 pt-3.5 text-[12px] font-bold text-tyba-text-muted">{t("prRecentMerged")}</div>
                {merged.map((item) => row(item, true))}
              </>
            )}
          </>
        )}
      </div>

      <footer className="flex min-h-9 shrink-0 items-center justify-between gap-4 shadow-[inset_0_1px_0_var(--tyba-line)] px-[18px] py-2 text-[11.5px] text-tyba-text-faint">
        <span className="min-w-0 truncate" title={t("prListNote")}>
          {t("prListNote")}
        </span>
        <span className="shrink-0 whitespace-nowrap">{t("prListNotePush")}</span>
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
