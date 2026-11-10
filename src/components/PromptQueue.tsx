import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  DotsSixVertical,
  Pause,
  Play,
  Queue,
  Trash,
  X,
} from "@phosphor-icons/react";

import { translateError } from "../lib/errors";
import {
  onPromptQueueChanged,
  promptQueueClear,
  promptQueueEnqueue,
  promptQueueList,
  promptQueuePause,
  promptQueueRemove,
  promptQueueReorder,
  promptQueueResume,
  type SessionId,
  type SessionStatus,
} from "../lib/ipc";
import {
  queueIndicator,
  reorderIds,
  slotAt,
  type QueueState,
  type QueueTone,
} from "../lib/promptQueue";
import { toastError } from "../lib/toast";
import { PLATE, PLATE_FLOAT, PLATE_LINE } from "./plate";

interface Props {
  sessionId: SessionId;
  status: SessionStatus;
  /** Distância da gaveta ao rodapé do pane: fica acima do campo de entrada. */
  bottomPx: number;
}

const TONE: Record<QueueTone, string> = {
  send: "text-tyba-green bg-tyba-green-tint",
  approval: "text-tyba-amber bg-tyba-amber-tint",
  failed: "text-tyba-red bg-tyba-red-tint",
  stopped: "text-tyba-text-muted bg-tyba-neutral-tint",
};

const BTN =
  "inline-flex h-6 items-center gap-1.5 rounded-[6px] px-[9px] text-[11.5px] whitespace-nowrap disabled:pointer-events-none disabled:opacity-50";

/**
 * Quadros 4A/4B (SshVariaveisFila): o botão "Fila · N" e a gaveta flutuante
 * presa ao canto inferior direito do pane do agente. O estado é o do core
 * (`prompt-queue://changed`); daqui só saem intenções.
 */
export function PromptQueue({ sessionId, status, bottomPx }: Props) {
  const { t } = useTranslation();
  const [queue, setQueue] = useState<QueueState | null>(null);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [drag, setDrag] = useState<{ from: number; slot: number } | null>(null);
  const rows = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => {
    let alive = true;
    const unlisten = onPromptQueueChanged((state) => {
      if (state.session_id === sessionId) setQueue(state);
    });
    void promptQueueList(sessionId)
      .then((state) => alive && setQueue(state))
      .catch(() => {});
    return () => {
      alive = false;
      void unlisten.then((f) => f());
    };
  }, [sessionId]);

  const fail = (e: unknown) =>
    toastError(t("promptQueueActionFailed"), translateError(e, t));

  const prompts = queue?.prompts ?? [];
  const paused = queue?.paused ?? null;
  const indicator = queueIndicator({ prompts, paused }, status);

  const add = () => {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    void promptQueueEnqueue(sessionId, text).catch(fail);
  };

  const slotFor = (y: number) =>
    slotAt(
      y,
      rows.current.slice(0, prompts.length).map((el) => {
        const r = el?.getBoundingClientRect();
        return r ? r.top + r.height / 2 : 0;
      }),
    );

  const dropLine = (slot: number) =>
    drag && drag.slot === slot && reorderIds(prompts.map((p) => p.id), drag.from, slot) ? (
      <div
        key={`drop-${slot}`}
        className="mx-1.5 h-[2px] shrink-0 rounded-[1px] bg-tyba-green-fill"
      />
    ) : null;

  return (
    <>
      {/* Acima da lista de blocos (z-10), abaixo da busca do terminal
          (z-20), que mora no mesmo canto. */}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={`absolute right-3 top-3 z-[15] ${BTN} text-tyba-violet ${
          open ? "tyba-nav-on" : `${PLATE_FLOAT} hover:text-tyba-violet`
        }`}
      >
        <Queue size={13} />
        <span className="text-tyba-text">
          {t("promptQueueButton", { count: prompts.length })}
        </span>
      </button>
      {open && (
        <div
          className={`absolute right-[18px] z-30 w-[430px] max-w-[calc(100%-36px)] overflow-hidden ${PLATE} ${PLATE_LINE} ${PLATE_FLOAT}`}
          style={{ bottom: bottomPx }}
        >
          <div className="flex items-center gap-2 px-3 pb-2 pt-2.5">
            <Queue size={15} className="shrink-0 text-tyba-violet" />
            <span className="text-[13px] font-medium text-tyba-text">
              {t("promptQueueTitle")}
            </span>
            <span className="text-[11.5px] text-tyba-text-faint">
              {prompts.length}
            </span>
            <span className="flex-1" />
            {paused ? (
              <button
                type="button"
                className={`${BTN} text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)]`}
                onClick={() => void promptQueueResume(sessionId).catch(fail)}
              >
                <Play size={11} />
                {t("promptQueueResume")}
              </button>
            ) : (
              <button
                type="button"
                disabled={prompts.length === 0}
                className={`${BTN} text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)]`}
                onClick={() => void promptQueuePause(sessionId).catch(fail)}
              >
                <Pause size={11} />
                {t("promptQueuePause")}
              </button>
            )}
            <button
              type="button"
              disabled={prompts.length === 0 && !paused}
              className={`${BTN} text-tyba-text-muted hover:text-tyba-text`}
              onClick={() => void promptQueueClear(sessionId).catch(fail)}
            >
              <Trash size={12} />
              {t("promptQueueClear")}
            </button>
          </div>
          <div
            className={`mx-3 mb-2 flex items-center gap-2 rounded-[6px] px-2.5 py-[7px] text-[12px] ${TONE[indicator.tone]}`}
          >
            {indicator.tone === "send" ? (
              <span className="size-[7px] shrink-0 rounded-full bg-tyba-green" />
            ) : (
              <Pause size={13} className="shrink-0" />
            )}
            <span className="shrink-0">{t(indicator.label)}</span>
            {indicator.detail && (
              <span className="ml-auto min-w-0 truncate text-[11px] text-tyba-text-muted">
                {"text" in indicator.detail
                  ? indicator.detail.text
                  : t(indicator.detail.key, indicator.detail.params)}
              </span>
            )}
          </div>
          <div className="flex max-h-[40vh] flex-col gap-px overflow-y-auto px-1.5">
            {prompts.map((p, i) => [
              dropLine(i),
              <div
                key={p.id}
                ref={(el) => {
                  rows.current[i] = el;
                }}
                className={`grid h-8 shrink-0 grid-cols-[16px_18px_1fr_22px] items-center gap-1.5 rounded-[6px] px-1.5 text-[12.5px] ${
                  drag?.from === i
                    ? "bg-[var(--tyba-glass-active)] shadow-[var(--tyba-pill-on),var(--tyba-shadow-md)]"
                    : ""
                }`}
              >
                <span className="text-right text-[11px] text-tyba-text-faint">
                  {i + 1}
                </span>
                <span
                  role="button"
                  aria-label={t("promptQueueDragHandle")}
                  className="flex cursor-grab touch-none text-tyba-text-ghost active:cursor-grabbing"
                  onPointerDown={(e) => {
                    e.currentTarget.setPointerCapture(e.pointerId);
                    setDrag({ from: i, slot: i });
                  }}
                  onPointerMove={(e) => {
                    if (!drag) return;
                    const slot = slotFor(e.clientY);
                    if (slot !== drag.slot) setDrag({ ...drag, slot });
                  }}
                  onPointerUp={() => {
                    if (!drag) return;
                    const order = reorderIds(
                      prompts.map((q) => q.id),
                      drag.from,
                      drag.slot,
                    );
                    setDrag(null);
                    if (order) void promptQueueReorder(sessionId, order).catch(fail);
                  }}
                  onPointerCancel={() => setDrag(null)}
                >
                  <DotsSixVertical size={14} />
                </span>
                <span className="truncate text-tyba-text" title={p.text}>
                  {p.text}
                </span>
                <button
                  type="button"
                  aria-label={t("promptQueueRemoveItem")}
                  className="inline-flex size-[22px] items-center justify-center rounded-[5px] text-tyba-text-faint hover:text-tyba-text"
                  onClick={() => void promptQueueRemove(sessionId, p.id).catch(fail)}
                >
                  <X size={12} />
                </button>
              </div>,
            ])}
            {dropLine(prompts.length)}
          </div>
          <div className="mt-1.5 flex items-center gap-2 px-3 pb-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-border-soft)]">
            <textarea
              rows={1}
              // O que sai daqui vai para o agente como foi escrito.
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              value={draft}
              placeholder={t("promptQueueAdd")}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  add();
                }
              }}
              className="h-[30px] min-w-0 flex-1 resize-none rounded-[5px] bg-[color-mix(in_srgb,var(--tyba-text)_2%,transparent)] px-[9px] py-[6px] text-[12px] leading-[18px] text-tyba-text outline-none shadow-[inset_0_0_0_1px_var(--tyba-border)] placeholder:text-tyba-text-ghost"
            />
            <kbd className="rounded-[4px] px-[5px] py-px text-[10.5px] text-tyba-text-faint shadow-[inset_0_0_0_1px_var(--tyba-border-field)]">
              ⌘⏎
            </kbd>
          </div>
        </div>
      )}
    </>
  );
}
