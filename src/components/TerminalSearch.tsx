import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, CaretUp, X } from "@phosphor-icons/react";

import { captureState } from "@/lib/keys";
import { getTerm } from "@/lib/termRegistry";
import type { SessionId } from "@/lib/ipc";

const DECORATIONS = {
  matchBackground: "#7cc5444d",
  matchBorder: "#7cc54480",
  matchOverviewRuler: "#7cc544",
  activeMatchBackground: "#f5a93b66",
  activeMatchBorder: "#f5a93b",
  activeMatchColorOverviewRuler: "#f5a93b",
};

interface Props {
  sessionId: SessionId | null;
  onClose: () => void;
}

export function TerminalSearch({ sessionId, onClose }: Props) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [result, setResult] = useState({ index: -1, count: 0 });
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    captureState.active = true;
    const entry = getTerm(sessionId);
    const selection = entry?.term.getSelection() ?? "";
    if (selection && !selection.includes("\n")) setQuery(selection);
    requestAnimationFrame(() => inputRef.current?.select());
    return () => {
      captureState.active = false;
      getTerm(sessionId)?.search.clearDecorations();
    };
  }, [sessionId]);

  useEffect(() => {
    const entry = getTerm(sessionId);
    if (!entry) return;
    const sub = entry.search.onDidChangeResults(({ resultIndex, resultCount }) =>
      setResult({ index: resultIndex, count: resultCount }),
    );
    return () => sub.dispose();
  }, [sessionId]);

  useEffect(() => {
    const entry = getTerm(sessionId);
    if (!entry) return;
    if (!query) {
      entry.search.clearDecorations();
      setResult({ index: -1, count: 0 });
      return;
    }
    const timer = window.setTimeout(() => {
      entry.search.findNext(query, {
        incremental: true,
        decorations: DECORATIONS,
      });
    }, 90);
    return () => window.clearTimeout(timer);
  }, [query, sessionId]);

  const step = (back: boolean) => {
    const entry = getTerm(sessionId);
    if (!entry || !query) return;
    const options = { decorations: DECORATIONS };
    if (back) entry.search.findPrevious(query, options);
    else entry.search.findNext(query, options);
  };

  const close = () => {
    getTerm(sessionId)?.term.focus();
    onClose();
  };

  const counter = query
    ? result.count > 0
      ? `${result.index + 1}/${result.count}`
      : t("searchNoResults")
    : "";

  return (
    <div className="absolute right-3 top-3 z-20 flex items-center gap-1 rounded-[6px] border border-tyba-border-strong bg-tyba-surface px-2 py-1 shadow-2xl">
      <input
        ref={inputRef}
        value={query}
        placeholder={t("searchPlaceholder")}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            close();
          } else if (e.key === "Enter") {
            e.preventDefault();
            step(e.shiftKey);
          }
        }}
        className="w-48 bg-transparent font-mono text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
      />
      <span className="min-w-14 shrink-0 text-right font-mono text-[10px] text-tyba-text-faint">
        {counter}
      </span>
      <button
        type="button"
        aria-label={t("searchPlaceholder")}
        onClick={() => step(true)}
        className="rounded-[3px] p-1 text-tyba-text-faint hover:bg-tyba-hover hover:text-tyba-text"
      >
        <CaretUp size={12} weight="bold" />
      </button>
      <button
        type="button"
        aria-label={t("searchPlaceholder")}
        onClick={() => step(false)}
        className="rounded-[3px] p-1 text-tyba-text-faint hover:bg-tyba-hover hover:text-tyba-text"
      >
        <CaretDown size={12} weight="bold" />
      </button>
      <button
        type="button"
        aria-label={t("hintClose")}
        onClick={close}
        className="rounded-[3px] p-1 text-tyba-text-faint hover:bg-tyba-hover hover:text-tyba-text"
      >
        <X size={12} weight="bold" />
      </button>
    </div>
  );
}
