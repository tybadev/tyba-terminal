import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, CaretUp, X } from "@phosphor-icons/react";

import { getTerm } from "@/lib/termRegistry";
import { getTerminalTheme } from "@/theme";
import type { SessionId } from "@/lib/ipc";

const NO_RESULT = { index: -1, count: 0 };

interface Props {
  sessionId: SessionId | null;
  onClose: () => void;
}

export function TerminalSearch({ sessionId, onClose }: Props) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [result, setResult] = useState(NO_RESULT);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<number | null>(null);

  const decorations = useMemo(() => {
    const theme = getTerminalTheme();
    return {
      matchBackground: theme.selectionBackground,
      matchBorder: theme.green,
      matchOverviewRuler: theme.green ?? "#7cc544",
      activeMatchBackground: theme.yellow,
      activeMatchBorder: theme.yellow,
      activeMatchColorOverviewRuler: theme.yellow ?? "#f5a93b",
    };
  }, []);

  useEffect(() => {
    setQuery("");
    setResult(NO_RESULT);
    const entry = getTerm(sessionId);
    const selection = entry?.term.getSelection() ?? "";
    if (selection && !selection.includes("\n")) setQuery(selection);
    requestAnimationFrame(() => inputRef.current?.select());
    return () => {
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
      setResult(NO_RESULT);
      return;
    }
    debounceRef.current = window.setTimeout(() => {
      debounceRef.current = null;
      entry.search.findNext(query, { incremental: true, decorations });
    }, 90);
    return () => {
      if (debounceRef.current !== null) {
        window.clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
    };
  }, [query, sessionId, decorations]);

  const step = useCallback(
    (back: boolean) => {
      const entry = getTerm(sessionId);
      if (!entry || !query) return;
      if (debounceRef.current !== null) {
        window.clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      if (back) entry.search.findPrevious(query, { decorations });
      else entry.search.findNext(query, { decorations });
    },
    [sessionId, query, decorations],
  );

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
