import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Paperclip, PaperPlaneRight, Warning } from "@phosphor-icons/react";

import { formatCombo, SEND_PROMPT_COMBO } from "@/lib/keys";
import {
  listWorktreeFiles,
  onSessionBracketedPaste,
  promptMentionsSensitive,
  sessionBracketedPaste,
  sessionRelPath,
  submitRichInput,
  type SessionId,
} from "../lib/ipc";
import {
  atQuery,
  enterAction,
  insertToken,
  type RichInputPref,
} from "../lib/richInput";

const MAX_HEIGHT_PX = 168;
const QUERY_DEBOUNCE_MS = 80;
const SUGGESTION_LIMIT = 20;

interface Props {
  sessionId: SessionId;
  pref: RichInputPref;
  focusNonce: number;
  openedExplicitly: boolean;
  onFocusChange: (focused: boolean) => void;
  onClose: () => void;
}

export function RichInput({
  sessionId,
  pref,
  focusNonce,
  openedExplicitly,
  onFocusChange,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);
  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  const [multiline, setMultiline] = useState(true);
  const [files, setFiles] = useState<string[]>([]);
  const [selected, setSelected] = useState(0);
  const [warnArmed, setWarnArmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [popoverLeft, setPopoverLeft] = useState(0);

  const active = useMemo(() => atQuery(text, caret), [text, caret]);
  const popoverOpen = active !== null && files.length > 0;

  const refreshMultiline = useCallback(() => {
    void sessionBracketedPaste(sessionId)
      .then(setMultiline)
      .catch(() => setMultiline(false));
  }, [sessionId]);

  useEffect(() => {
    refreshMultiline();
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void onSessionBracketedPaste(sessionId, setMultiline).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [sessionId, refreshMultiline]);

  const seenNonce = useRef(focusNonce);
  useEffect(() => {
    if (openedExplicitly) textareaRef.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (focusNonce === seenNonce.current) return;
    seenNonce.current = focusNonce;
    textareaRef.current?.focus();
  }, [focusNonce]);

  useEffect(() => {
    if (!active) {
      setFiles([]);
      setSelected(0);
      return;
    }
    const timer = window.setTimeout(() => {
      void listWorktreeFiles(sessionId, active.query, SUGGESTION_LIMIT)
        .then((found) => {
          setFiles(found);
          setSelected(0);
        })
        .catch(() => setFiles([]));
    }, QUERY_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [sessionId, active?.query, active !== null]);

  useEffect(() => {
    const el = textareaRef.current;
    const mirror = mirrorRef.current;
    if (!el || !mirror || !active) return;
    mirror.style.width = `${el.clientWidth}px`;
    mirror.textContent = text.slice(0, active.start);
    const marker = document.createElement("span");
    marker.textContent = "@";
    mirror.appendChild(marker);
    setPopoverLeft(Math.min(marker.offsetLeft, el.clientWidth - 240));
  }, [text, active]);

  const resize = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT_PX)}px`;
  }, []);

  const syncCaret = () => {
    const el = textareaRef.current;
    if (el) setCaret(el.selectionStart ?? 0);
  };

  const applyText = (nextText: string, nextCaret: number) => {
    setText(nextText);
    setCaret(nextCaret);
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (!el) return;
      el.setSelectionRange(nextCaret, nextCaret);
      resize();
    });
  };

  const pickFile = (file: string) => {
    if (!active) return;
    const next = insertToken(text, caret, active, file);
    applyText(next.text, next.caret);
  };

  const attach = async () => {
    const picked = await openDialog({ multiple: false }).catch(() => null);
    if (typeof picked !== "string") return;
    const rel = await sessionRelPath(sessionId, picked).catch(() => picked);
    const next = insertToken(text, caret, { start: caret, query: "" }, rel);
    applyText(next.text, next.caret);
    textareaRef.current?.focus();
  };

  const doSubmit = async () => {
    const value = text;
    if (!value.trim()) return;
    setError(null);
    if (pref.warnOnSensitivePrompt && !warnArmed) {
      const sensitive = await promptMentionsSensitive(value).catch(() => false);
      if (sensitive) {
        setWarnArmed(true);
        return;
      }
    }
    setWarnArmed(false);
    try {
      await submitRichInput(sessionId, value, true);
    } catch (e) {
      setError(String(e));
      return;
    }
    applyText("", 0);
    if (pref.autoDismiss) onClose();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (popoverOpen) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const delta = e.key === "ArrowDown" ? 1 : -1;
        setSelected((prev) => (prev + delta + files.length) % files.length);
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        pickFile(files[selected]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setFiles([]);
        return;
      }
    }
    if (e.key === "Enter") {
      const action = enterAction(
        { shift: e.shiftKey, ctrlOrMeta: e.ctrlKey || e.metaKey },
        pref.submitWithCtrlEnter,
      );
      if (action === "submit") {
        e.preventDefault();
        void doSubmit();
        return;
      }
      if (action === "none" || !multiline) {
        e.preventDefault();
        if (!multiline && action === "newline") {
          setError(t("richInputSingleLine"));
        }
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        applyText(`${text.slice(0, caret)}\n${text.slice(caret)}`, caret + 1);
        return;
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setText(e.target.value);
    setCaret(e.target.selectionStart ?? 0);
    setWarnArmed(false);
    setError(null);
    resize();
  };

  return (
    <div className="relative shrink-0 border-t border-tyba-border bg-tyba-sunken px-3 py-2">
      {popoverOpen && (
        <div
          className="absolute bottom-full z-20 mb-1 max-h-56 w-72 overflow-y-auto rounded-[6px] border border-tyba-border bg-tyba-raised py-1 shadow-lg"
          style={{ left: Math.max(12, popoverLeft) }}
        >
          {files.map((file, i) => (
            <button
              key={file}
              onMouseDown={(e) => {
                e.preventDefault();
                pickFile(file);
              }}
              className={`block w-full truncate px-2.5 py-1 text-left font-mono text-[12px] ${
                i === selected
                  ? "bg-tyba-green/15 text-tyba-text"
                  : "text-tyba-text-muted hover:bg-tyba-text/[.04]"
              }`}
            >
              {file}
            </button>
          ))}
        </div>
      )}
      <div
        ref={mirrorRef}
        aria-hidden
        className="pointer-events-none invisible absolute whitespace-pre-wrap break-words font-mono text-[13px]"
      />
      <div className="flex items-end gap-2">
        <button
          onClick={() => void attach()}
          title={t("richInputAttach")}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[4px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
        >
          <Paperclip size={15} />
        </button>
        <textarea
          ref={textareaRef}
          value={text}
          rows={1}
          placeholder={t("richInputPlaceholder", {
            combo: formatCombo(SEND_PROMPT_COMBO),
          })}
          onChange={onChange}
          onKeyDown={onKeyDown}
          onKeyUp={syncCaret}
          onClick={syncCaret}
          onFocus={() => {
            onFocusChange(true);
            refreshMultiline();
          }}
          onBlur={() => onFocusChange(false)}
          className="max-h-[168px] min-h-[28px] flex-1 resize-none rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 font-mono text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
        />
        <button
          onClick={() => void doSubmit()}
          title={warnArmed ? t("richInputSensitiveWarn") : t("richInputSend")}
          className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-[4px] transition-colors ${
            warnArmed
              ? "bg-tyba-amber/20 text-tyba-amber hover:bg-tyba-amber/30"
              : "text-tyba-text-muted hover:bg-tyba-text/[.06] hover:text-tyba-text"
          }`}
        >
          {warnArmed ? <Warning size={15} /> : <PaperPlaneRight size={15} />}
        </button>
      </div>
      {(warnArmed || error || !multiline) && (
        <div className="mt-1 flex items-center gap-2 px-9 text-[11px]">
          {warnArmed && (
            <span className="text-tyba-amber">
              {t("richInputSensitiveWarn")}
            </span>
          )}
          {!multiline && (
            <span className="text-tyba-text-faint">
              {t("richInputSingleLine")}
            </span>
          )}
          {error && <span className="text-tyba-red">{error}</span>}
        </div>
      )}
    </div>
  );
}
