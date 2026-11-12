import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLineRight, Folder, Lock, Sparkle, Warning, X } from "@phosphor-icons/react";

import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { PLATE, PLATE_FLOAT, PLATE_LINE } from "./plate";
import { ClaudeIcon } from "./icons/ClaudeIcon";
import { OpenAIIcon } from "./icons/OpenAIIcon";
import { translateError } from "../lib/errors";
import {
  closeHandoff,
  defaultTarget,
  previewBody,
  previewLines,
  REDACTION_MARK,
  subscribeHandoff,
  type HandoffRequest,
} from "../lib/handoff";
import {
  handoffTargets,
  sessionHandoffOpen,
  sessionHandoffPreview,
  type HandoffAgent,
  type HandoffPreview,
  type HandoffTarget,
  type Session,
  type SessionId,
  type TurnDigest,
} from "../lib/ipc";
import { toastError } from "../lib/toast";

/** Espera curta antes de pedir o texto de novo ao core enquanto o dono escreve. */
const PREVIEW_DEBOUNCE_MS = 250;

export function HandoffAgentIcon({ agent, size }: { agent: HandoffAgent; size: number }) {
  if (agent === "claude") {
    return <ClaudeIcon size={size} className="shrink-0" style={{ color: "#d97757" }} />;
  }
  if (agent === "codex") return <OpenAIIcon size={size} className="shrink-0 text-tyba-blue" />;
  return <Sparkle size={size} className="shrink-0" />;
}

interface BodyProps {
  from: string;
  turn: TurnDigest;
  targets: HandoffTarget[];
  target: HandoffAgent | null;
  onTarget: (agent: HandoffAgent) => void;
  preview: HandoffPreview | null;
  note: string;
  onNote: (note: string) => void;
  opening: boolean;
  onCancel: () => void;
  onOpen: () => void;
}

/**
 * A folha do quadro H2, sem o diálogo em volta: agente, o texto exato que vai
 * (só leitura, do core), a nota, o aviso e os botões.
 */
export function HandoffSheetBody({
  from,
  turn,
  targets,
  target,
  onTarget,
  preview,
  note,
  onNote,
  opening,
  onCancel,
  onOpen,
}: BodyProps) {
  const { t, i18n } = useTranslation();
  const time = new Date(turn.started_at_ms).toLocaleTimeString(i18n.language, {
    hour: "2-digit",
    minute: "2-digit",
  });
  const chosen = targets.find((x) => x.agent === target && x.available) ?? null;
  const lines = preview ? previewLines(previewBody(preview.text, note)) : [];

  return (
    <>
      <div className="flex items-center gap-2 pb-1 pl-4 pr-3 pt-3">
        <ArrowLineRight size={15} className="shrink-0 text-tyba-blue" />
        <DialogTitle className="text-[13px] font-medium leading-normal text-tyba-text">
          {t("handoffTitle")}
        </DialogTitle>
        <span className="flex-1" />
        <button
          type="button"
          aria-label={t("cancel")}
          onClick={onCancel}
          className="inline-flex size-6 shrink-0 items-center justify-center rounded-[5px] text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </div>
      <DialogDescription asChild>
        <div className="px-4 pb-2.5 text-[11px] text-tyba-text-faint">
          {t("handoffFrom")} <span className="text-tyba-amber">{from}</span>
          {preview && ` · ${preview.folder}`} · {t("handoffTurnAt", { time })}
          {turn.diff === "ok" && turn.files_total > 0 && (
            <>
              {" · "}
              {t("diffFilesSummary", { count: turn.files_total })}{" "}
              <span className="font-mono">
                <span className="text-tyba-green">+{turn.insertions}</span>{" "}
                <span className="text-tyba-red">−{turn.deletions}</span>
              </span>
            </>
          )}
        </div>
      </DialogDescription>

      <div className="flex min-h-0 flex-col gap-2.5 px-4 pb-3">
        <div className="text-[10.5px] font-medium uppercase tracking-[.12em] text-tyba-text-faint">
          {t("handoffPickAgent")}
        </div>
        <div role="radiogroup" className="flex gap-1.5">
          {targets.map((x) => {
            const on = x.available && x.agent === target;
            return (
              <button
                key={x.agent}
                type="button"
                role="radio"
                aria-checked={on}
                disabled={!x.available}
                onClick={() => onTarget(x.agent)}
                className={`flex h-9 flex-1 items-center gap-[9px] rounded-[7px] px-2.5 text-[12.5px] ${
                  on
                    ? "bg-tyba-green/[.06] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-green)]"
                    : x.available
                      ? "text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border)]"
                      : "text-tyba-text-ghost shadow-[inset_0_0_0_1px_var(--tyba-border)]"
                }`}
              >
                <span
                  className={`size-3 shrink-0 rounded-full ${
                    on
                      ? "shadow-[inset_0_0_0_4px_var(--tyba-green)]"
                      : "shadow-[inset_0_0_0_1px_var(--tyba-border-strong)]"
                  }`}
                />
                <HandoffAgentIcon agent={x.agent} size={15} />
                {x.label}
                <span className="ml-auto text-[11px] text-tyba-text-faint">
                  {x.available ? t("handoffInstalled") : x.reason}
                </span>
              </button>
            );
          })}
        </div>

        <div className="flex items-center gap-2 text-[10.5px] font-medium uppercase tracking-[.12em] text-tyba-text-faint">
          {t("handoffPreviewLabel")}
          <span className="ml-auto inline-flex items-center gap-[5px] font-normal normal-case tracking-normal">
            <Lock size={11} />
            {preview && preview.redacted > 0 && (
              <>{t("handoffRedacted", { count: preview.redacted })} · </>
            )}
            {t("handoffReadOnly")}
          </span>
        </div>
        <div className="flex min-h-0 flex-col rounded-[6px] bg-tyba-text/[.02] font-mono text-[12px] leading-[19px] shadow-[inset_0_0_0_1px_var(--tyba-border)]">
          <pre
            data-testid="handoff-preview"
            className="max-h-[40vh] min-h-0 overflow-auto px-3 py-2.5 text-tyba-text-muted"
          >
            {lines.map((line, i) => (
              <span key={i} className={line.head ? "font-medium text-tyba-text" : undefined}>
                {line.parts.map((p, j) =>
                  p.kind === "mark" ? (
                    <span
                      key={j}
                      title={t("handoffRedactedMark")}
                      className="inline-flex items-center gap-[3px] rounded-[3px] bg-tyba-text/[.06] px-1 leading-[17px] text-tyba-text-faint"
                    >
                      <Lock size={10} />
                      {REDACTION_MARK}
                    </span>
                  ) : p.kind === "add" ? (
                    <span key={j} className="text-tyba-green">
                      {p.text}
                    </span>
                  ) : p.kind === "del" ? (
                    <span key={j} className="text-tyba-red">
                      {p.text}
                    </span>
                  ) : (
                    p.text
                  ),
                )}
                {i < lines.length - 1 && "\n"}
              </span>
            ))}
          </pre>
          <div className="px-3 pb-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-border-soft)]">
            <label htmlFor="handoff-note" className="mb-1.5 block font-medium text-tyba-text">
              {t("handoffNoteLabel")}
            </label>
            <textarea
              id="handoff-note"
              autoFocus
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              value={note}
              onChange={(e) => onNote(e.target.value)}
              className="block min-h-[46px] w-full resize-none rounded-[5px] bg-tyba-surface px-[9px] py-1.5 text-tyba-text outline-none shadow-[inset_0_0_0_1px_var(--tyba-border-field)] focus:shadow-[inset_0_0_0_1px_var(--tyba-green)]"
            />
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 pb-3 pl-4 pr-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-border-soft)]">
        <div className="flex min-w-0 flex-col gap-[3px] text-[11px]">
          <span className="flex items-center gap-1.5 whitespace-nowrap text-tyba-amber">
            <Warning size={12} />
            {t("handoffNoApprovals")}
          </span>
          {preview && (
            <span className="flex items-center gap-1.5 whitespace-nowrap text-tyba-text-faint">
              <Folder size={12} />
              {t("handoffSameFolder", { folder: preview.folder })}
            </span>
          )}
        </div>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onCancel}
          className="inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[6px] px-3 text-[12px] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)]"
        >
          {t("cancel")}
        </button>
        <button
          type="button"
          disabled={!chosen || !preview || opening}
          onClick={onOpen}
          className="inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[6px] bg-tyba-primary px-3 text-[12px] font-medium text-tyba-text-inverse disabled:opacity-50"
        >
          {chosen && <HandoffAgentIcon agent={chosen.agent} size={13} />}
          {t("handoffOpenIn", { agent: chosen?.label ?? "" })}
        </button>
      </div>
    </>
  );
}

/**
 * A folha "Passar para outro agente" (H2), aberta pela aba Turno (H1) ou pela
 * paleta (H4). Pede ao core os destinos e o texto; abre com só o agente e a
 * nota. Quem chamou recebe a sessão nova para pôr em aba ao lado (H3).
 */
export function HandoffSheet({
  onOpened,
}: {
  onOpened: (opened: Session, origin: SessionId) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [request, setRequest] = useState<HandoffRequest | null>(null);
  const [targets, setTargets] = useState<HandoffTarget[]>([]);
  const [target, setTarget] = useState<HandoffAgent | null>(null);
  const [note, setNote] = useState("");
  const [preview, setPreview] = useState<HandoffPreview | null>(null);
  const [opening, setOpening] = useState(false);
  const asked = useRef(0);

  useEffect(() => subscribeHandoff(setRequest), []);

  const sessionId = request?.sessionId ?? null;
  useEffect(() => {
    setTargets([]);
    setTarget(null);
    setNote("");
    setPreview(null);
    setOpening(false);
    if (!sessionId) return;
    let live = true;
    handoffTargets(sessionId)
      .then((list) => {
        if (!live) return;
        setTargets(list);
        setTarget(defaultTarget(list));
      })
      .catch((e) => {
        if (live) toastError(t("handoffFailed"), translateError(e, t));
      });
    return () => {
      live = false;
    };
  }, [sessionId, t]);

  useEffect(() => {
    if (!sessionId || !target) return;
    const ticket = ++asked.current;
    const timer = window.setTimeout(() => {
      sessionHandoffPreview(sessionId, target, note)
        .then((p) => {
          if (ticket === asked.current) setPreview(p);
        })
        .catch((e) => {
          if (ticket === asked.current) toastError(t("handoffFailed"), translateError(e, t));
        });
    }, PREVIEW_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [sessionId, target, note, t]);

  const open = async () => {
    if (!request || !target) return;
    setOpening(true);
    try {
      const opened = await sessionHandoffOpen(request.sessionId, target, note);
      closeHandoff();
      await onOpened(opened, request.sessionId);
    } catch (e) {
      setOpening(false);
      toastError(t("handoffFailed"), translateError(e, t));
    }
  };

  if (!request) return null;
  return (
    <Dialog open onOpenChange={(o) => !o && closeHandoff()}>
      <DialogContent
        showCloseButton={false}
        className={`top-[52px] flex max-h-[calc(100vh-72px)] w-[620px] max-w-[calc(100%-2rem)] translate-y-0 flex-col gap-0 overflow-hidden p-0 ${PLATE} ${PLATE_LINE} ${PLATE_FLOAT}`}
      >
        <HandoffSheetBody
          from={request.from}
          turn={request.turn}
          targets={targets}
          target={target}
          onTarget={setTarget}
          preview={preview}
          note={note}
          onNote={setNote}
          opening={opening}
          onCancel={closeHandoff}
          onOpen={() => void open()}
        />
      </DialogContent>
    </Dialog>
  );
}
