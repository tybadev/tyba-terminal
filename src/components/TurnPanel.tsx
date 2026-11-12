import { useTranslation } from "react-i18next";
import { ArrowLineRight, Check, Folder, GitBranch, Minus, Pause } from "@phosphor-icons/react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

import {
  splitPath,
  turnAgo,
  turnBand,
  turnMeta,
  turnRows,
  type TurnBand,
} from "../lib/agentsDrawer";
import { hasTurnDiff, openGitScreen } from "../lib/gitScreen";
import { agentsDrawerCollapse, type FileStatus, type SessionId, type TurnDigest } from "../lib/ipc";
import { useNow } from "./ui/loading";

const BAND: Record<TurnBand, string> = {
  changed: "text-tyba-green bg-tyba-green-tint",
  failed: "text-tyba-red bg-tyba-red-tint",
  interrupted: "text-tyba-text-muted bg-tyba-neutral-tint",
  none: "text-tyba-text-muted bg-tyba-neutral-tint",
  no_repo: "text-tyba-text-muted bg-tyba-neutral-tint",
  unavailable: "text-tyba-text-muted bg-tyba-neutral-tint",
};

export const TURN_LETTER: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  type_changed: "T",
  unmerged: "U",
  other: "?",
};

export const TURN_LETTER_COLOR: Record<FileStatus, string> = {
  added: "text-tyba-green",
  modified: "text-tyba-amber",
  deleted: "text-tyba-red",
  renamed: "text-tyba-blue",
  copied: "text-tyba-blue",
  type_changed: "text-tyba-amber",
  unmerged: "text-tyba-red",
  other: "text-tyba-text-faint",
};

const HANDOFF_BTN =
  "inline-flex h-6 items-center gap-1.5 whitespace-nowrap rounded-[6px] px-[9px] text-[11.5px] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)]";

/**
 * A aba Turno da gaveta (quadros T2 e T3 de ResumoDoTurno): o que mudou na
 * pasta entre o prompt e o fim do último turno. Só nomes e contagens, como
 * chegam do core em `Session.last_turn`. O conteúdo fica na tela de Git:
 * "Ver diff do turno" e o clique numa linha abrem a seção Turno dela.
 */
export function TurnPanel({
  sessionId,
  turn,
  turnOpen,
  onHandoff,
}: {
  sessionId: SessionId;
  turn: TurnDigest;
  /** Turno aberto: o agente trabalha e "Passar para…" fica desligado (H1). */
  turnOpen: boolean;
  /** Abre a folha "Passar para outro agente" (HandoffAgentes H2). */
  onHandoff: () => void;
}) {
  const { t, i18n } = useTranslation();
  const now = useNow(true);
  const band = turnBand(turn);
  const { rows, more } = turnRows(turn);
  const hasFiles = turn.files_total > 0;
  const canDiff = hasTurnDiff({ last_turn: turn });
  // A tela de Git cobre o pane: a gaveta recolhe para não ficar por cima.
  const seeDiff = (path: string | null) => {
    void openGitScreen(sessionId, "turn", null, path)
      .then(() => agentsDrawerCollapse(sessionId))
      .catch(() => {});
  };
  const since = new Date(turn.started_at_ms).toLocaleTimeString(i18n.language, {
    hour: "2-digit",
    minute: "2-digit",
  });
  const totals = hasFiles && (
    <span className="ml-auto inline-flex shrink-0 gap-[5px] whitespace-nowrap font-mono text-[11.5px] tabular-nums">
      <span className="text-tyba-green">+{turn.insertions}</span>
      <span className="text-tyba-red">−{turn.deletions}</span>
    </span>
  );

  return (
    <>
      <div className="mx-3 flex items-baseline gap-2 px-0.5 pt-0.5 text-[12.5px]">
        <span className="text-tyba-text-faint">&gt;</span>
        <span className="min-w-0 flex-1 truncate text-tyba-text" title={turn.prompt ?? undefined}>
          {turn.prompt}
        </span>
        <span className="shrink-0 text-[11px] text-tyba-text-faint">
          {turnAgo(turn, now, t)}
        </span>
      </div>
      <div className="mx-3 mb-[9px] mt-0.5 truncate pl-[18px] pr-0.5 font-mono text-[11px] text-tyba-text-faint">
        {turnMeta(turn, i18n.language, t)}
      </div>
      <div
        className={`mx-3 mb-2 flex items-center gap-2 rounded-[6px] px-2.5 py-[7px] text-[12px] ${BAND[band]}`}
      >
        {band === "failed" ? (
          <span className="size-[7px] shrink-0 rounded-full bg-tyba-red-fill" />
        ) : band === "interrupted" ? (
          <Pause size={13} className="shrink-0" />
        ) : band === "no_repo" ? (
          <Folder size={13} className="shrink-0" />
        ) : band === "unavailable" ? (
          <Minus size={13} className="shrink-0" />
        ) : (
          <Check size={13} className="shrink-0" />
        )}
        <span className="shrink-0 whitespace-nowrap">
          {band === "failed"
            ? t("pillTurnFailed")
            : band === "interrupted"
              ? t("turnInterrupted")
              : band === "changed"
                ? t("turnChanged", { count: turn.files_total })
                : band === "none"
                  ? t("turnNone")
                  : band === "no_repo"
                    ? t("turnNoRepo")
                    : t("turnUnavailable")}
        </span>
        {band === "failed"
          ? turn.failure && (
              <span className="ml-auto min-w-0 truncate text-[11px] text-tyba-text-muted">
                {turn.failure}
              </span>
            )
          : band !== "none" && totals}
      </div>
      {rows.length > 0 && (
        <div className="flex flex-col gap-px px-1.5">
          {rows.map((file) => {
            const { dir, name } = splitPath(file.path);
            return (
              <button
                key={file.path}
                type="button"
                disabled={!canDiff}
                onClick={() => seeDiff(file.path)}
                className="grid h-[26px] grid-cols-[14px_minmax(0,1fr)_44px_40px] items-center gap-2 rounded-[6px] px-2 text-[12px] enabled:hover:bg-tyba-text/[.06]"
              >
                <span
                  className={`text-center font-mono text-[11px] font-medium ${TURN_LETTER_COLOR[file.status]}`}
                >
                  {TURN_LETTER[file.status]}
                </span>
                {/* Corta pela esquerda: o nome do arquivo nunca some. */}
                <span
                  className="min-w-0 truncate text-left font-mono [direction:rtl]"
                  title={file.path}
                >
                  <span className="text-tyba-text-faint [direction:ltr] [unicode-bidi:embed]">
                    {dir}
                  </span>
                  <span className="text-tyba-text [direction:ltr] [unicode-bidi:embed]">
                    {name}
                  </span>
                </span>
                <span className="text-right font-mono text-[11.5px] tabular-nums text-tyba-green">
                  {file.insertions ? `+${file.insertions}` : ""}
                </span>
                <span className="text-right font-mono text-[11.5px] tabular-nums text-tyba-red">
                  {file.deletions ? `−${file.deletions}` : ""}
                </span>
              </button>
            );
          })}
          {more > 0 && (
            <div className="pl-[30px] pr-2 pt-[5px] text-[11.5px] text-tyba-text-faint">
              {t("turnMoreFiles", { count: more })}
            </div>
          )}
        </div>
      )}
      {/* Turno falho ou interrompido sem medição não pode parecer "nada mudou". */}
      {(band === "none" ||
        turn.diff === "no_repo" ||
        (turn.diff === "unavailable" && band !== "unavailable")) && (
        <p className="mx-3 px-0.5 pb-1 pt-2.5 text-[12px] leading-[18px] text-tyba-text-faint">
          {t(
            turn.diff === "no_repo"
              ? "turnNoRepoHint"
              : turn.diff === "unavailable"
                ? "turnUnavailable"
                : "turnNoneHint",
          )}
        </p>
      )}
      {/* O rodapé "desde HH:MM" fica inteiro: sem espaço, desce de linha. */}
      <div
        className={`flex flex-wrap items-center gap-x-2 gap-y-1.5 px-3 pb-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-border-soft)] ${
          hasFiles ? "mt-1.5" : "mt-0.5"
        }`}
      >
        {canDiff && (
          <button
            type="button"
            onClick={() => seeDiff(null)}
            className="inline-flex h-6 items-center gap-1.5 whitespace-nowrap rounded-[6px] px-[9px] text-[11.5px] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)]"
          >
            <GitBranch size={12} />
            {t("turnSeeDiff")}
          </button>
        )}
        {turnOpen ? (
          <Tooltip>
            <TooltipTrigger asChild>
              {/* Botão desligado não recebe hover: a dica mora no invólucro. */}
              <span tabIndex={0} className="inline-flex">
                <button type="button" disabled className={`${HANDOFF_BTN} opacity-[.42]`}>
                  <ArrowLineRight size={12} />
                  {t("handoffPassTo")}
                </button>
              </span>
            </TooltipTrigger>
            <TooltipContent side="top">{t("handoffWaitTurn")}</TooltipContent>
          </Tooltip>
        ) : (
          <button type="button" onClick={onHandoff} className={HANDOFF_BTN}>
            <ArrowLineRight size={12} />
            {t("handoffPassTo")}
          </button>
        )}
        <span className="flex-1" />
        <span className="whitespace-nowrap text-[11px] text-tyba-text-faint">
          {t(hasFiles ? "turnSince" : "turnStartedAt", { time: since })}
        </span>
      </div>
    </>
  );
}
