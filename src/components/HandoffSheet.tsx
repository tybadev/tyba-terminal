import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLineRight, Folder, Lock, Sparkle, Warning, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { PLATE, PLATE_FLOAT, PLATE_LINE } from "./plate";
import { ClaudeIcon } from "./icons/ClaudeIcon";
import { OpenAIIcon } from "./icons/OpenAIIcon";
import { translateError } from "../lib/errors";
import {
  closeHandoff,
  defaultTarget,
  noteAsSent,
  previewLines,
  REDACTION_MARK,
  subscribeHandoff,
  withMarks,
  type HandoffRequest,
  type PreviewPart,
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

/** O core manda o caminho inteiro; a folha mostra só o nome da pasta. */
const folderName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;

/** O core corta a nota nesse tamanho; o campo também não passa disso. */
const NOTE_MAX = 2000;

/** Espera curta antes de pedir o texto de novo ao core enquanto o dono escreve. */
const PREVIEW_DEBOUNCE_MS = 250;

export function HandoffAgentIcon({ agent, size }: { agent: HandoffAgent; size: number }) {
  if (agent === "claude") {
    return <ClaudeIcon size={size} className="shrink-0" style={{ color: "#d97757" }} />;
  }
  if (agent === "codex") return <OpenAIIcon size={size} className="shrink-0 text-tyba-blue" />;
  return <Sparkle size={size} className="shrink-0" />;
}

function Parts({ parts }: { parts: PreviewPart[] }) {
  const { t } = useTranslation();
  return parts.map((p, j) =>
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
  );
}

interface BodyProps {
  from: string;
  turn: TurnDigest;
  /** `null` enquanto o core não respondeu. */
  targets: HandoffTarget[] | null;
  target: HandoffAgent | null;
  onTarget: (agent: HandoffAgent) => void;
  preview: HandoffPreview | null;
  /** A nota com que o core montou `preview`. */
  previewNote: string | null;
  /** Erro do core (ex.: pasta sem git): fica no lugar do texto. */
  error: string | null;
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
  previewNote,
  error,
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
  const chosen = targets?.find((x) => x.agent === target && x.available) ?? null;
  const noAgent = targets !== null && !targets.some((x) => x.available);
  const lines = preview && !error ? previewLines(preview.body) : [];
  const sent = error ? null : noteAsSent(preview?.note, previewNote, note);

  return (
    <>
      <div className="flex items-center gap-2 pb-1 pl-4 pr-3 pt-3">
        <ArrowLineRight size={15} className="shrink-0 text-tyba-blue" />
        <DialogTitle className="text-[14px] font-semibold leading-normal text-tyba-text">
          {t("handoffTitle")}
        </DialogTitle>
        <span className="flex-1" />
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={t("cancel")}
          onClick={onCancel}
        >
          <X size={13} />
        </Button>
      </div>
      <DialogDescription asChild>
        <div className="px-4 pb-2.5 text-[11px] text-tyba-text-faint">
          {t("handoffFrom")} <span className="text-tyba-amber">{from}</span>
          {preview && ` · ${folderName(preview.folder)}`} · {t("handoffTurnAt", { time })}
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
        <div className="tyba-label">
          {t("handoffPickAgent")}
        </div>
        <div role="radiogroup" className="flex gap-1.5">
          {(targets ?? []).map((x) => {
            const on = x.available && x.agent === target;
            return (
              <button
                key={x.agent}
                type="button"
                role="radio"
                aria-checked={on}
                disabled={!x.available}
                onClick={() => onTarget(x.agent)}
                className={`flex h-10 flex-1 items-center gap-[9px] rounded-[10px] px-3 text-[13px] font-medium outline-none transition-colors focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
                  on
                    ? "tyba-pill-on text-tyba-text"
                    : x.available
                      ? "text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-tyba-hover"
                      : "text-tyba-text-ghost shadow-[inset_0_0_0_1px_var(--tyba-line)]"
                }`}
              >
                <span
                  className={`size-3 shrink-0 rounded-full ${
                    on
                      ? "shadow-[inset_0_0_0_4px_var(--primary)]"
                      : "shadow-[inset_0_0_0_1px_var(--tyba-control-line)]"
                  }`}
                />
                <HandoffAgentIcon agent={x.agent} size={15} />
                {x.label}
                <span className="ml-auto text-[11px] font-normal text-tyba-text-faint">
                  {x.available ? t("handoffInstalled") : x.reason}
                </span>
              </button>
            );
          })}
        </div>

        <div className="tyba-label flex items-center gap-2">
          {t("handoffPreviewLabel")}
          <span className="ml-auto inline-flex items-center gap-[5px] font-normal normal-case tracking-normal">
            <Lock size={11} />
            {preview && preview.redacted > 0 && (
              <>{t("handoffRedacted", { count: preview.redacted })} · </>
            )}
            {t("handoffReadOnly")}
          </span>
        </div>
        <div className="flex min-h-0 flex-col overflow-hidden rounded-[10px] bg-tyba-paper-solid font-mono text-[12px] leading-[19px] shadow-[inset_0_0_0_1px_var(--tyba-line)]">
          {error ? (
            <div
              role="alert"
              data-testid="handoff-error"
              className="flex items-start gap-2 px-3 py-2.5 font-sans text-tyba-red"
            >
              <Warning size={13} className="mt-[3px] shrink-0" />
              {error}
            </div>
          ) : (
            <pre
              data-testid="handoff-preview"
              className="max-h-[40vh] min-h-0 overflow-auto px-3 py-2.5 text-tyba-text-muted"
            >
              {lines.map((line, i) => (
                <span
                  key={i}
                  className={
                    line.head
                      ? "font-medium text-tyba-text"
                      : line.quote
                        ? "text-tyba-text-faint"
                        : undefined
                  }
                >
                  <Parts parts={line.parts} />
                  {i < lines.length - 1 && "\n"}
                </span>
              ))}
            </pre>
          )}
          <div className="px-3 pb-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-line)]">
            <label htmlFor="handoff-note" className="mb-1.5 block font-semibold text-tyba-text">
              {t("handoffNoteLabel")}
            </label>
            <Textarea
              id="handoff-note"
              autoFocus
              maxLength={NOTE_MAX}
              value={note}
              onChange={(e) => onNote(e.target.value)}
              className="block min-h-[46px] resize-none px-3 py-2 text-[12px]"
            />
            {sent !== null && (
              <div
                data-testid="handoff-note-as-sent"
                className="mt-1.5 whitespace-pre-wrap break-words text-[11.5px] text-tyba-text-faint"
              >
                {t("handoffNoteAsSent")} <Parts parts={withMarks(sent)} />
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 pb-3 pl-4 pr-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-line)]">
        <div className="flex min-w-0 flex-col gap-[3px] text-[11px]">
          <span className="flex items-center gap-1.5 whitespace-nowrap text-tyba-amber">
            <Warning size={12} />
            {t("handoffNoApprovals")}
          </span>
          {preview && (
            <span className="flex items-center gap-1.5 whitespace-nowrap text-tyba-text-faint">
              <Folder size={12} />
              {t("handoffSameFolder", { folder: folderName(preview.folder) })}
            </span>
          )}
        </div>
        <span className="flex-1" />
        <Button type="button" variant="outline" size="sm" onClick={onCancel}>
          {t("cancel")}
        </Button>
        <Button
          type="button"
          size="sm"
          disabled={!chosen || !preview || Boolean(error) || opening}
          onClick={onOpen}
        >
          {chosen && <HandoffAgentIcon agent={chosen.agent} size={15} />}
          {noAgent ? t("handoffNoAgent") : t("handoffOpenIn", { agent: chosen?.label ?? "" })}
        </Button>
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
  const [targets, setTargets] = useState<HandoffTarget[] | null>(null);
  const [target, setTarget] = useState<HandoffAgent | null>(null);
  const [note, setNote] = useState("");
  const [preview, setPreview] = useState<HandoffPreview | null>(null);
  const [previewNote, setPreviewNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const asked = useRef(0);

  useEffect(() => subscribeHandoff(setRequest), []);

  const sessionId = request?.sessionId ?? null;
  useEffect(() => {
    setTargets(null);
    setTarget(null);
    setNote("");
    setPreview(null);
    setPreviewNote(null);
    setError(null);
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
        if (live) setError(translateError(e, t));
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
          if (ticket !== asked.current) return;
          setPreview(p);
          setPreviewNote(note);
          setError(null);
        })
        .catch((e) => {
          if (ticket !== asked.current) return;
          setPreview(null);
          setPreviewNote(null);
          setError(translateError(e, t));
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
          previewNote={previewNote}
          error={error}
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
