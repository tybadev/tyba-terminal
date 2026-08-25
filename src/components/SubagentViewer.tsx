import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CircleNotch, Gear, X } from "@phosphor-icons/react";

import { AgentIcon } from "./icons/AgentIcon";

import type {
  SessionId,
  SubagentRun,
  SubagentSnapshot,
  TranscriptEntry,
} from "@/lib/ipc";
import { onSubagentTranscript, subagentTranscript } from "@/lib/ipc";

const MAX_ENTRIES = 1000;

interface RectStyle {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface Props {
  sessionId: SessionId;
  snapshot: SubagentSnapshot | null;
  sessionEnded: boolean;
  rect: RectStyle;
  onClose: () => void;
  onFocus: () => void;
}

function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${seconds.toString().padStart(2, "0")}s`;
}

function useLiveClock(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

function StatusDot({ status }: { status: SubagentRun["status"] }) {
  if (status === "running")
    return (
      <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-tyba-green [box-shadow:var(--tyba-glow-green)]" />
    );
  if (status === "starting")
    return (
      <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-tyba-amber" />
    );
  return <span className="size-1.5 shrink-0 rounded-full bg-tyba-text-faint" />;
}

function EntryRow({ entry }: { entry: TranscriptEntry }) {
  if (entry.kind === "user_text")
    return (
      <div className="rounded-[5px] border border-tyba-border bg-tyba-raised/40 px-2.5 py-1.5">
        <p className="whitespace-pre-wrap break-words text-[12px] leading-relaxed text-tyba-text-muted">
          {entry.text}
        </p>
      </div>
    );

  if (entry.kind === "tool_use")
    return (
      <div className="flex items-start gap-1.5 px-1 text-[11px] text-tyba-text-muted">
        <Gear size={12} className="mt-0.5 shrink-0 text-tyba-text-faint" />
        <span className="min-w-0 break-words">
          {entry.tool_name && (
            <span className="font-mono text-tyba-text">{entry.tool_name}</span>
          )}
          {entry.text && (
            <span className="text-tyba-text-faint">
              {entry.tool_name ? ": " : ""}
              {entry.text}
            </span>
          )}
        </span>
      </div>
    );

  if (entry.kind === "tool_result")
    return (
      <p className="whitespace-pre-wrap break-words px-1 pl-[18px] text-[11px] leading-snug text-tyba-text-faint">
        {entry.text}
      </p>
    );

  return (
    <p className="whitespace-pre-wrap break-words px-1 text-[12px] leading-relaxed text-tyba-text">
      {entry.text}
    </p>
  );
}

export function SubagentViewer({
  sessionId,
  snapshot,
  sessionEnded,
  rect,
  onClose,
  onFocus,
}: Props) {
  const { t } = useTranslation();
  const focusedId = snapshot?.focused ?? null;
  const focused = useMemo(
    () => snapshot?.subagents.find((s) => s.agent_id === focusedId) ?? null,
    [snapshot, focusedId],
  );

  const [entries, setEntries] = useState<TranscriptEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [trimmed, setTrimmed] = useState(false);
  const cursorRef = useRef(0);
  const receivedRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);

  const running = focused?.status === "running" || focused?.status === "starting";
  const now = useLiveClock(Boolean(running) && !sessionEnded);
  const duration = focused
    ? formatDuration((focused.ended_at_ms ?? now) - focused.started_at_ms)
    : "";

  useEffect(() => {
    if (!focusedId) {
      setEntries([]);
      cursorRef.current = 0;
      return;
    }
    let cancelled = false;
    setLoading(true);
    setEntries([]);
    setTrimmed(false);
    cursorRef.current = 0;
    receivedRef.current = 0;
    stickRef.current = true;
    void subagentTranscript(sessionId, focusedId, null)
      .then((res) => {
        if (cancelled) return;
        if (res.cursor < cursorRef.current) return;
        cursorRef.current = res.cursor;
        receivedRef.current = res.entries.length;
        setTrimmed(res.entries.length > MAX_ENTRIES);
        setEntries(
          res.entries.length > MAX_ENTRIES
            ? res.entries.slice(res.entries.length - MAX_ENTRIES)
            : res.entries,
        );
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, focusedId]);

  useEffect(() => {
    if (!focusedId) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onSubagentTranscript((p) => {
      if (p.session_id !== sessionId || p.agent_id !== focusedId) return;
      if (p.cursor <= cursorRef.current) return;
      cursorRef.current = p.cursor;
      receivedRef.current += p.entries.length;
      if (receivedRef.current > MAX_ENTRIES) setTrimmed(true);
      setEntries((prev) => {
        const merged = [...prev, ...p.entries];
        return merged.length > MAX_ENTRIES
          ? merged.slice(merged.length - MAX_ENTRIES)
          : merged;
      });
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [sessionId, focusedId]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [entries]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    stickRef.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 24;
  };

  const statusLabel = focused
    ? sessionEnded && focused.status !== "done"
      ? t("subagentsViewerEnded")
      : t(
          focused.status === "running"
            ? "subagentsStatusRunning"
            : focused.status === "starting"
              ? "subagentsStatusStarting"
              : "subagentsStatusDone",
        )
    : "";

  return (
    <div
      onMouseDown={onFocus}
      className="absolute overflow-hidden rounded-[4px] border border-tyba-border bg-tyba-bg"
      style={{
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        height: `${rect.height}%`,
      }}
    >
      <div className="flex h-full min-h-0 flex-col">
        <header className="flex h-8 shrink-0 items-center gap-2 tyba-divide-b px-3">
          <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-tyba-amber/[.1] text-tyba-amber">
            <AgentIcon size={11} />
          </span>
          <span className="shrink-0 truncate text-[12px] font-medium text-tyba-text">
            {focused?.agent_type || t("subagentsTitle")}
          </span>
          {focused?.description && (
            <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-faint">
              {focused.description}
            </span>
          )}
          <div className="flex-1" />
          {focused && (
            <span className="flex shrink-0 items-center gap-1.5 text-[11px] text-tyba-text-muted">
              <StatusDot status={focused.status} />
              {statusLabel}
              {duration && (
                <span className="font-mono text-tyba-text-faint">
                  {duration}
                </span>
              )}
            </span>
          )}
          <button
            onClick={onClose}
            aria-label={t("subagentsViewerClose")}
            className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
          >
            <X size={13} />
          </button>
        </header>

        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="min-h-0 flex-1 overflow-y-auto p-2"
        >
          {!focused ? (
            <p className="px-1 py-3 text-[12px] text-tyba-text-faint">
              {t("subagentsViewerEmpty")}
            </p>
          ) : (
            <>
              {focused.status === "done" && focused.summary && (
                <div className="mb-2 rounded-[5px] border border-tyba-green/40 bg-tyba-green/[.06] px-2.5 py-1.5">
                  <p className="mb-0.5 text-[10px] uppercase tracking-wide text-tyba-green">
                    {t("subagentsSummary")}
                  </p>
                  <p className="whitespace-pre-wrap break-words text-[12px] leading-relaxed text-tyba-text">
                    {focused.summary}
                  </p>
                </div>
              )}
              {loading && entries.length === 0 && (
                <div className="flex items-center gap-1.5 px-1 py-3 text-[12px] text-tyba-text-faint">
                  <CircleNotch size={13} className="animate-spin" />
                  {t("subagentsViewerLoading")}
                </div>
              )}
              {!loading && entries.length === 0 && (
                <p className="px-1 py-3 text-[12px] text-tyba-text-faint">
                  {t("subagentsViewerNoTranscript")}
                </p>
              )}
              {trimmed && (
                <p className="pb-1.5 text-center text-[10px] text-tyba-text-faint">
                  {t("subagentsViewerTrimmed")}
                </p>
              )}
              <div className="flex flex-col gap-1.5">
                {entries.map((entry) => (
                  <EntryRow key={`${focusedId}-${entry.seq}`} entry={entry} />
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
