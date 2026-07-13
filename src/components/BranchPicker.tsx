import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowsClockwise,
  Check,
  CloudArrowDown,
  GitBranch,
} from "@phosphor-icons/react";

import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  sessionBranches,
  sessionFetch,
  type BranchList,
  type SessionId,
} from "../lib/ipc";

interface Props {
  sessionId: SessionId;
  browsing: string | null;
  label: string;
  onBrowse: (branch: string | null) => void;
}

export function BranchPicker({ sessionId, browsing, label, onBrowse }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<BranchList | null>(null);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    sessionBranches(sessionId)
      .then(setList)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [sessionId]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    load();
  }, [open, load]);

  const remoteFetch = useCallback(() => {
    if (fetching) return;
    setFetching(true);
    setError(null);
    sessionFetch(sessionId)
      .then(load)
      .catch((e) => setError(String(e)))
      .finally(() => setFetching(false));
  }, [sessionId, fetching, load]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const branches = list?.branches ?? [];
    if (!q) return branches;
    return branches.filter(
      (b) =>
        b.name.toLowerCase().includes(q) ||
        b.subject.toLowerCase().includes(q),
    );
  }, [list, query]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          title={t("branchPickerTitle")}
          className={`flex min-w-0 shrink items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-[10px] transition-colors ${
            browsing
              ? "border-tyba-violet/50 text-tyba-violet hover:bg-tyba-violet/10"
              : "border-tyba-border text-tyba-text-faint hover:bg-tyba-text/[.04] hover:text-tyba-text"
          }`}
        >
          <GitBranch size={10} className="shrink-0" />
          <span className="truncate">{label}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-96 p-1">
        <div className="flex items-center gap-1 p-1">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("branchPickerSearch")}
            className="h-7 min-w-0 flex-1 rounded-[4px] border border-tyba-border bg-black/20 px-2 font-mono text-[11px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
          />
          <button
            title={t("branchFetch")}
            aria-label={t("branchFetch")}
            disabled={fetching}
            onClick={remoteFetch}
            className="flex size-7 shrink-0 items-center justify-center rounded-[4px] text-tyba-text-faint hover:bg-tyba-text/[.06] hover:text-tyba-text disabled:opacity-50"
          >
            {fetching ? (
              <ArrowsClockwise size={13} className="animate-spin" />
            ) : (
              <CloudArrowDown size={13} />
            )}
          </button>
        </div>
        {browsing && (
          <button
            onClick={() => {
              onBrowse(null);
              setOpen(false);
            }}
            className="mx-1 mb-1 flex w-[calc(100%-8px)] items-center gap-2 rounded-md bg-tyba-text/[.04] p-2 text-left text-[11px] text-tyba-text hover:bg-tyba-text/[.08]"
          >
            <ArrowsClockwise size={12} className="shrink-0" />
            {t("branchBackToSession")}
          </button>
        )}
        <div className="max-h-72 overflow-y-auto">
          {loading && (
            <div className="px-2 py-4 text-center text-[11px] text-tyba-text-faint">
              {t("diffLoading")}
            </div>
          )}
          {!loading && error && (
            <div className="px-2 py-4 text-center text-[11px] text-tyba-red">
              {error}
            </div>
          )}
          {!loading &&
            !error &&
            filtered.map((b) => {
              const active = browsing === b.name;
              return (
                <button
                  key={b.name}
                  onClick={() => {
                    onBrowse(b.is_current ? null : b.name);
                    setOpen(false);
                  }}
                  className={`flex w-full items-center gap-2 rounded-md p-2 text-left text-[11px] hover:bg-tyba-text/[.04] ${
                    active ? "bg-tyba-text/[.06]" : ""
                  }`}
                >
                  <GitBranch
                    size={12}
                    className={`shrink-0 ${b.is_current ? "text-tyba-green" : "text-tyba-text-faint"}`}
                  />
                  <span className="min-w-0 flex-1 truncate font-mono">
                    {b.name}
                  </span>
                  {b.is_remote && (
                    <span className="shrink-0 rounded-full bg-tyba-text/[.06] px-1.5 text-[9px] text-tyba-text-faint">
                      {t("branchRemote")}
                    </span>
                  )}
                  {b.is_current && (
                    <Check size={12} className="shrink-0 text-tyba-green" />
                  )}
                  <span className="max-w-40 shrink-0 truncate text-tyba-text-faint">
                    {b.subject}
                  </span>
                </button>
              );
            })}
          {!loading && !error && (list?.truncated ?? 0) > 0 && (
            <div className="px-2 py-2 text-center text-[10px] text-tyba-text-faint">
              {t("branchTruncated", { count: list?.truncated ?? 0 })}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
