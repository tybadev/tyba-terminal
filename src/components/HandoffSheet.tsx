import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, FolderSimple, Lock, LockSimple, Warning, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
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

  // Quadro Handoff (`.ho-m`): uma coluna com padding 22/24/18 e gap 12 —
  // cabeçalho, legendas, opções, prévia e rodapé, sem faixas nem fios.
  return (
    <div className="flex min-h-0 flex-col gap-3 px-6 pb-[18px] pt-[22px]">
      <div className="flex items-center gap-2.5">
        {/* Quadro renderizado: a seta na cor do texto e o fechar logo depois
            do título. */}
        <ArrowRight size={18} className="shrink-0 text-tyba-text" />
        <DialogTitle className="text-[17px] font-bold leading-normal text-tyba-text">
          {t("handoffTitle")}
        </DialogTitle>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={t("close")}
          onClick={onCancel}
        >
          <X size={16} />
        </Button>
      </div>
      <DialogDescription asChild>
        <div className="text-[13px] text-tyba-text-muted">
          {t("handoffFrom")}{" "}
          <b className="font-bold" style={{ color: "#d97757" }}>
            {from}
          </b>
          {preview && ` · ${folderName(preview.folder)}`} · {t("handoffTurnAt", { time })}
        </div>
      </DialogDescription>

      <div className="tyba-label is-cap mt-1">{t("handoffPickAgent")}</div>
      <div role="radiogroup" className="grid grid-cols-2 gap-2.5">
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
              className={`flex h-12 min-w-0 items-center gap-2.5 rounded-[12px] px-3.5 text-[14px] outline-none transition-[background-color,box-shadow] focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
                on
                  ? "bg-[var(--tyba-control-bg)] font-semibold text-tyba-text [box-shadow:var(--tyba-knob-shadow),inset_0_0_0_1.5px_var(--tyba-blue)]"
                  : x.available
                    ? "bg-[var(--tyba-mk-glass-soft,transparent)] font-semibold text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-tyba-hover"
                    : "bg-[var(--tyba-mk-glass-soft,transparent)] font-medium text-tyba-text-faint shadow-[inset_0_0_0_1px_var(--tyba-line)]"
              }`}
            >
              <span
                className={`size-4 shrink-0 rounded-full ${
                  on
                    ? "shadow-[inset_0_0_0_5px_var(--tyba-blue)]"
                    : "shadow-[inset_0_0_0_1.5px_var(--tyba-text-faint)]"
                }`}
              />
              <span className="truncate">{x.label}</span>
              <span className="ml-auto shrink-0 text-[11.5px] font-medium text-tyba-text-faint">
                {x.available ? t("handoffInstalled") : x.reason}
              </span>
            </button>
          );
        })}
      </div>

      <div className="tyba-label is-cap mt-1 flex items-center gap-1.5">
        {t("handoffPreviewLabel")}
        <span className="ml-auto inline-flex items-center gap-1 font-medium normal-case tracking-normal">
          <LockSimple size={13} />
          {preview && preview.redacted > 0 && (
            <>{t("handoffRedacted", { count: preview.redacted })} · </>
          )}
          {t("handoffReadOnly")}
        </span>
      </div>
      <div className="flex min-h-0 flex-col overflow-hidden rounded-[12px] bg-[var(--tyba-control-bg)] font-mono text-[12px] leading-5 shadow-[inset_0_0_0_1px_var(--tyba-line)]">
        {error ? (
          <div
            role="alert"
            data-testid="handoff-error"
            className="flex items-start gap-2 px-4 py-3.5 [font-family:var(--tyba-font-ui)] text-tyba-red"
          >
            <Warning size={14} className="mt-[3px] shrink-0" />
            {error}
          </div>
        ) : (
          <pre
            data-testid="handoff-preview"
            className="max-h-[250px] min-h-0 overflow-auto px-4 py-3.5 text-tyba-text"
          >
            {lines.map((line, i) => (
              <span
                key={i}
                className={
                  line.head
                    ? "font-bold text-tyba-text"
                    : line.quote
                      ? "text-tyba-text-muted"
                      : undefined
                }
              >
                <Parts parts={line.parts} />
                {i < lines.length - 1 && "\n"}
              </span>
            ))}
          </pre>
        )}
        <div className="flex flex-col gap-2 px-4 pb-3.5 pt-3 shadow-[inset_0_1px_0_var(--tyba-line)]">
          <label htmlFor="handoff-note" className="block text-tyba-text">
            {t("handoffNoteLabel")}
          </label>
          {/* Quadro: Input de uma linha, na sans do campo, com a largura do
              campo do Muriki (`.mk-field`, 280px). */}
          <Input
            id="handoff-note"
            autoFocus
            maxLength={NOTE_MAX}
            value={note}
            onChange={(e) => onNote(e.target.value)}
            placeholder={t("handoffNotePlaceholder")}
            className="w-[280px] max-w-full [font-family:var(--tyba-font-ui)]"
          />
          {sent !== null && (
            <div
              data-testid="handoff-note-as-sent"
              className="whitespace-pre-wrap break-words text-[11.5px] text-tyba-text-faint"
            >
              {t("handoffNoteAsSent")} <Parts parts={withMarks(sent)} />
            </div>
          )}
        </div>
      </div>

      <div className="flex items-center gap-2.5 pt-1.5">
        <div className="flex min-w-0 flex-1 flex-col gap-1 text-[12px] leading-[normal]">
          <span className="flex items-center gap-1.5 text-tyba-amber">
            <Warning size={14} className="shrink-0" />
            {t("handoffNoApprovals")}
          </span>
          {preview && (
            <span className="flex items-center gap-1.5 text-tyba-text-muted">
              <FolderSimple size={14} className="shrink-0" />
              {t("handoffSameFolder", { folder: folderName(preview.folder) })}
            </span>
          )}
        </div>
        <Button type="button" variant="outline" onClick={onCancel}>
          {t("cancel")}
        </Button>
        <Button
          type="button"
          disabled={!chosen || !preview || Boolean(error) || opening}
          onClick={onOpen}
        >
          {chosen && <ArrowRight />}
          {noAgent ? t("handoffNoAgent") : t("handoffOpenIn", { agent: chosen?.label ?? "" })}
        </Button>
      </div>
    </div>
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
        // Quadro Handoff: centrada, vidro `is-float` (o da placa) com raio
        // 18, sobre o véu marinho .18 sem desfoque.
        overlayClassName="bg-[var(--tyba-mk-scrim,rgba(60,40,120,0.10))] backdrop-blur-[var(--tyba-mk-scrim-blur,2px)]"
        className="flex max-h-[calc(100vh-72px)] w-[620px] max-w-[calc(100%-2rem)] flex-col gap-0 overflow-hidden rounded-[18px] p-0"
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
