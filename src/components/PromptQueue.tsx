import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretUp,
  Check,
  DotsSixVertical,
  Pause,
  Play,
  PlusMinus,
  Queue,
  Trash,
  X,
} from "@phosphor-icons/react";

import { translateError } from "../lib/errors";
import {
  agentsDrawerCollapse,
  agentsDrawerPresence,
  agentsDrawerTab,
  promptQueueClear,
  promptQueueEnqueue,
  promptQueuePause,
  promptQueueRemove,
  promptQueueReorder,
  promptQueueResume,
  type AgentRunner,
  type ApprovalRequest,
  type DrawerState,
  type DrawerTab,
  type Session,
  type SubagentSnapshot,
} from "../lib/ipc";
import {
  drawerTab,
  pendingSubagentApprovals,
  pillTurnCount,
  pillTurnHead,
  pillTurnLabel,
  queueGoesToAgents,
  roundProgress,
  turnTabCount,
  type PillAgents,
  type PillTurn,
} from "../lib/agentsDrawer";
import { drawerBox, type DrawerBox } from "../lib/appFooter";
import { queueIndicator, reorderIds, slotAt, type QueueTone } from "../lib/promptQueue";
import { toastError } from "../lib/toast";
import { AgentsPanel } from "./AgentsPanel";
import { AgentIcon } from "./icons/AgentIcon";
import { PLATE, PLATE_FLOAT, PLATE_LINE } from "./plate";
import { TurnPanel } from "./TurnPanel";
import { usePromptQueue } from "./usePromptQueue";

interface Props {
  session: Session;
  /** O rodapé do app: a gaveta sobe dele (quadro R3). */
  footer: RefObject<HTMLElement | null>;
  /** Aberta/recolhida e a aba do dono, decididos pelo core. */
  drawer: DrawerState | null;
  snapshot: SubagentSnapshot | null;
  approvals: ApprovalRequest[];
  /** O binário que recebe a fila (`claude`, `codex`). */
  agentName: string;
  orchestratorTitle: string | null;
  runner: AgentRunner | null;
  /** Subagente com o detalhe aberto na lateral (4E). */
  selectedSubagent: string | null;
  onSelectSubagent: (agentId: string) => void;
}

const TONE: Record<QueueTone, string> = {
  send: "text-tyba-green bg-tyba-green-tint",
  approval: "text-tyba-amber bg-tyba-amber-tint",
  failed: "text-tyba-red bg-tyba-red-tint",
  stopped: "text-tyba-text-muted bg-tyba-neutral-tint",
};

const BTN =
  "inline-flex h-6 items-center gap-1.5 rounded-[6px] px-[9px] text-[11.5px] whitespace-nowrap disabled:pointer-events-none disabled:opacity-50";

const VBTN =
  "inline-flex size-6 shrink-0 items-center justify-center rounded-[5px] text-tyba-text-faint hover:text-tyba-text disabled:pointer-events-none disabled:opacity-50";

/**
 * A gaveta Fila/Agentes do pane ativo (conteúdo de GavetaFilaAgentes 4A′–4E;
 * posição de GavetaPaneEstreito R3/R4): popover que sobe do rodapé do app,
 * ancorado na borda direita do pane. Recolhida, não desenha nada sobre o pane
 * — a pílula mora no rodapé (`DrawerPill`). Abrir, recolher e a aba do dono
 * são do core (`agents-drawer://changed`); a fila também
 * (`prompt-queue://changed`). Daqui só saem intenções.
 */
export function PromptQueue({
  session,
  footer,
  drawer,
  snapshot,
  approvals,
  agentName,
  orchestratorTitle,
  runner,
  selectedSubagent,
  onSelectSubagent,
}: Props) {
  const { t } = useTranslation();
  const sessionId = session.id;
  const queue = usePromptQueue(sessionId);
  const [draft, setDraft] = useState("");
  const [drag, setDrag] = useState<{ from: number; slot: number } | null>(null);
  const [hover, setHover] = useState(false);
  const [focusWithin, setFocusWithin] = useState(false);
  const rows = useRef<(HTMLDivElement | null)[]>([]);

  const open = drawer?.open ?? false;
  const box = usePopoverBox(open, footer);
  // Mouse em cima ou foco de teclado dentro: o core não recolhe a gaveta aberta
  // sozinha enquanto o dono está nela (4D). Foco deixado por clique não conta:
  // o mouse sai e o botão clicado segue focado, e a gaveta nunca recolheria.
  const present = open && (hover || focusWithin);
  const sentPresent = useRef(false);
  useEffect(() => {
    if (present === sentPresent.current) return;
    sentPresent.current = present;
    void agentsDrawerPresence(sessionId, present).catch(() => {});
  }, [present, sessionId]);
  useEffect(() => {
    if (!open) {
      setHover(false);
      setFocusWithin(false);
    }
  }, [open]);

  const fail = (e: unknown) =>
    toastError(t("promptQueueActionFailed"), translateError(e, t));

  const prompts = queue?.prompts ?? [];
  const paused = queue?.paused ?? null;
  const indicator = queueIndicator({ prompts, paused }, session.status);

  const subagents = snapshot?.subagents ?? [];
  const hasSubagents = subagents.length > 0;
  const pending = pendingSubagentApprovals(sessionId, subagents, approvals);
  const progress = roundProgress(subagents);
  const turn = session.last_turn ?? null;
  const tab = drawerTab({
    owner: drawer?.tab ?? null,
    hasSubagents,
    pending,
    running: progress?.running ?? false,
    turn: session,
  });
  const goToAgents = hasSubagents && queueGoesToAgents(paused, pending);

  const setTab = (next: DrawerTab) =>
    void agentsDrawerTab(sessionId, next).catch(() => {});
  const collapse = () => void agentsDrawerCollapse(sessionId).catch(() => {});

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

  const collapseButton = (
    <button
      type="button"
      onClick={collapse}
      aria-label={t("agentsDrawerCollapse")}
      title={t("agentsDrawerCollapse")}
      className={VBTN}
    >
      <CaretDown size={14} />
    </button>
  );

  const pauseButton = paused ? (
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
  );
  const clearDisabled = prompts.length === 0 && !paused;
  const clear = () => void promptQueueClear(sessionId).catch(fail);

  // Marcador invisível no pane: a gaveta mede por ele a borda direita do pane.
  const marker = <span ref={box.anchor} aria-hidden className="hidden" />;
  if (!open || !box.box) return marker;

  const panel = (
    // Fora do pane: ela pode crescer para a esquerda por cima da lateral do app.
    <div
      role="region"
      aria-label={t(hasSubagents ? "agentsDrawerLabel" : "promptQueueTitle")}
      className={`fixed z-30 overflow-hidden ${PLATE} ${PLATE_LINE} ${PLATE_FLOAT}`}
      style={{ left: box.box.left, width: box.box.width, bottom: box.box.bottom }}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onFocus={(e) => setFocusWithin(e.target.matches(":focus-visible"))}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
          setFocusWithin(false);
        }
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) {
          e.preventDefault();
          collapse();
        }
      }}
    >
      <div className="flex items-center gap-2 px-3 pb-2 pt-2.5">
        {hasSubagents || turn ? (
          <>
            <DrawerTabs
              tab={tab}
              queueCount={prompts.length}
              agents={progress}
              turnCount={turn ? turnTabCount(turn) : undefined}
              pending={pending}
              compact={box.box.compact}
              onTab={setTab}
            />
            <span className="flex-1" />
            {tab === "queue" && (
              <>
                {pauseButton}
                <button
                  type="button"
                  disabled={clearDisabled}
                  aria-label={t("promptQueueClear")}
                  title={t("promptQueueClear")}
                  className={VBTN}
                  onClick={clear}
                >
                  <Trash size={12} />
                </button>
              </>
            )}
          </>
        ) : (
          <>
            <Queue size={15} className="shrink-0 text-tyba-violet" />
            <span className="text-[13px] font-medium text-tyba-text">
              {t("promptQueueTitle")}
            </span>
            <span className="text-[11.5px] text-tyba-text-faint">
              {prompts.length}
            </span>
            <span className="flex-1" />
            {pauseButton}
            <button
              type="button"
              disabled={clearDisabled}
              className={`${BTN} text-tyba-text-muted hover:text-tyba-text`}
              onClick={clear}
            >
              <Trash size={12} />
              {t("promptQueueClear")}
            </button>
          </>
        )}
        {collapseButton}
      </div>
      {tab === "agents" ? (
        <AgentsPanel
          session={session}
          snapshot={snapshot}
          approvals={approvals}
          orchestratorTitle={orchestratorTitle}
          runner={runner}
          selectedId={selectedSubagent}
          onSelect={onSelectSubagent}
        />
      ) : tab === "turn" && turn ? (
        <TurnPanel turn={turn} />
      ) : (
        <>
          <div
            className={`mx-3 mb-2 flex items-center gap-2 rounded-[6px] px-2.5 py-[7px] text-[12px] ${TONE[indicator.tone]}`}
          >
            {indicator.tone === "send" ? (
              <span className="size-[7px] shrink-0 rounded-full bg-tyba-green" />
            ) : (
              <Pause size={13} className="shrink-0" />
            )}
            <span className="shrink-0">{t(indicator.label)}</span>
            {goToAgents ? (
              <button
                type="button"
                onClick={() => setTab("agents")}
                className="ml-auto shrink-0 whitespace-nowrap text-[11px] text-tyba-amber underline underline-offset-2"
              >
                {t("agentsDrawerSeeAgents")}
              </button>
            ) : (
              indicator.detail && (
                <span className="ml-auto min-w-0 truncate text-[11px] text-tyba-text-muted">
                  {"text" in indicator.detail
                    ? indicator.detail.text
                    : t(indicator.detail.key, indicator.detail.params)}
                </span>
              )
            )}
          </div>
          {prompts.length === 0 ? (
            <p className="mx-3 px-0.5 pb-1 pt-2.5 text-[12px] leading-[18px] text-tyba-text-faint">
              {t("promptQueueEmpty", { agent: agentName })}
            </p>
          ) : (
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
          )}
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
        </>
      )}
    </div>
  );

  return (
    <>
      {marker}
      {createPortal(panel, document.body)}
    </>
  );
}

/**
 * Onde a gaveta aberta fica (quadros R3/R4): mede a borda direita do pane pelo
 * marcador e o topo do rodapé, e refaz quando um dos dois ou a janela mudam.
 * Sem pane visível (painel lateral em tela cheia) não há gaveta.
 */
function usePopoverBox(open: boolean, footer: RefObject<HTMLElement | null>) {
  const anchor = useRef<HTMLSpanElement | null>(null);
  const [box, setBox] = useState<DrawerBox | null>(null);
  useLayoutEffect(() => {
    if (!open) {
      setBox(null);
      return;
    }
    const pane = anchor.current?.parentElement;
    if (!pane) return;
    const measure = () => {
      const r = pane.getBoundingClientRect();
      if (r.width === 0) {
        setBox(null);
        return;
      }
      const foot = footer.current?.getBoundingClientRect();
      const next = drawerBox({
        paneLeft: r.left,
        paneRight: r.right,
        footerTop: foot && foot.height > 0 ? foot.top : window.innerHeight,
        viewportHeight: window.innerHeight,
      });
      setBox((prev) =>
        prev &&
        prev.left === next.left &&
        prev.width === next.width &&
        prev.bottom === next.bottom
          ? prev
          : next,
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(pane);
    if (footer.current) ro.observe(footer.current);
    window.addEventListener("resize", measure);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [open, footer]);
  return { anchor, box };
}

/**
 * As abas do topo da gaveta (4C; T2 de ResumoDoTurno): o mesmo segmento das
 * abas da janela. Agentes só com subagentes; Turno depois do primeiro fim de
 * turno, com o número de arquivos (traço sem nenhum). Compactas (gaveta abaixo de 430, quadro R4): ícone + contagem, o nome no
 * tooltip e no `aria-label`.
 */
function DrawerTabs({
  tab,
  queueCount,
  agents,
  turnCount,
  pending,
  compact,
  onTab,
}: {
  tab: DrawerTab;
  queueCount: number;
  /** `null` sem subagentes: não há aba Agentes. */
  agents: { done: number; total: number } | null;
  /** `undefined` sem turno terminado: não há aba Turno. */
  turnCount: number | null | undefined;
  pending: number;
  compact: boolean;
  onTab: (tab: DrawerTab) => void;
}) {
  const { t } = useTranslation();
  const item = (on: boolean) =>
    `flex h-[26px] items-center whitespace-nowrap rounded-[5px] text-[12px] ${
      compact ? "gap-[5px] px-2" : "gap-1.5 px-2.5"
    } ${on ? "tyba-pill-on text-tyba-text" : "text-tyba-text-faint hover:text-tyba-text-muted"}`;
  const queueName = `${t("agentsDrawerQueue")} · ${queueCount}`;
  const agentsName = agents && `${t("agentsTitle")} · ${agents.done}/${agents.total}`;
  const turnName = `${t("pillTurn")} · ${turnCount ?? "–"}`;
  return (
    <div
      role="tablist"
      className="flex gap-0.5 rounded-[7px] bg-[color-mix(in_srgb,var(--tyba-text)_5%,transparent)] p-0.5"
    >
      <button
        type="button"
        role="tab"
        aria-selected={tab === "queue"}
        aria-label={compact ? queueName : undefined}
        title={compact ? queueName : undefined}
        className={item(tab === "queue")}
        onClick={() => onTab("queue")}
      >
        <Queue size={13} className="shrink-0 text-tyba-violet" />
        {!compact && t("agentsDrawerQueue")}
        <span className="text-[11px] text-tyba-text-faint">{queueCount}</span>
      </button>
      {agents && (
        <button
          type="button"
          role="tab"
          aria-selected={tab === "agents"}
          aria-label={compact ? (agentsName ?? undefined) : undefined}
          title={compact ? (agentsName ?? undefined) : undefined}
          className={item(tab === "agents")}
          onClick={() => onTab("agents")}
        >
          <AgentIcon size={13} className="shrink-0 text-tyba-amber" />
          {!compact && t("agentsTitle")}
          <span className="font-mono text-[11px] text-tyba-text-faint">
            {agents.done}/{agents.total}
          </span>
          {pending > 0 && (
            <span
              className="rounded-full bg-tyba-amber-tint px-1.5 font-mono text-[10px] leading-4 text-tyba-amber"
              title={t("agentsPendingApprovals", { count: pending })}
            >
              {pending}
            </span>
          )}
        </button>
      )}
      {turnCount !== undefined && (
        <button
          type="button"
          role="tab"
          aria-selected={tab === "turn"}
          aria-label={compact ? turnName : undefined}
          title={compact ? turnName : undefined}
          className={item(tab === "turn")}
          onClick={() => onTab("turn")}
        >
          <PlusMinus size={13} className="shrink-0 text-tyba-cyan" />
          {!compact && t("pillTurn")}
          <span className="font-mono text-[11px] text-tyba-text-faint">
            {turnCount ?? "–"}
          </span>
        </button>
      )}
    </div>
  );
}

const DOT = "size-[6px] shrink-0 rounded-full";

/**
 * A pílula da gaveta, chip do rodapé do app (quadro Pílulas, R1–R5): 18 px,
 * sem placa de vidro. Sempre "Fila · N"; com subagentes, "Agentes x/y" e o
 * estado da rodada. Pede você: contorno inteiro âmbar. A seta diz para onde a
 * gaveta vai; aberta, a pílula fica marcada. Compacta (degrau 6 da escada):
 * só ícones e números, o texto inteiro no tooltip e no `aria-label` — o estado
 * (cor, contorno, número) nunca sai.
 *
 * Turno terminado (quadro Pílulas de ResumoDoTurno): terceiro trecho,
 * "Turno · N arquivos +a −d", e o estado dos agentes encolhe para o ícone.
 * Falha: trecho vermelho, sem contorno.
 */
export function DrawerPill({
  queueCount,
  agents,
  turn,
  open,
  compact,
  onToggle,
}: {
  queueCount: number;
  agents: PillAgents | null;
  turn: PillTurn | null;
  open: boolean;
  compact: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const state = agents?.state ?? null;
  const attn = state?.kind === "attn";
  const stateText =
    state?.kind === "run"
      ? t("subagentsStatusRunning")
      : state?.kind === "ok"
        ? t("subagentsStatusDone")
        : state?.kind === "attn"
          ? t("agentsDrawerAsksYou", { count: state.count })
          : state?.kind === "fail"
            ? t("agentsCountFailed", { count: state.count })
            : null;
  const label = [
    t("promptQueueButton", { count: queueCount }),
    agents && `${t("agentsTitle")} ${agents.done}/${agents.total}`,
    stateText,
    turn && pillTurnLabel(turn, t),
  ]
    .filter(Boolean)
    .join(" · ");
  // Com o trecho do turno, o estado dos agentes fica só no ícone.
  const stateCompact = compact || turn !== null;
  const turnFailed = turn?.outcome === "failed";
  const ring = "inset 0 0 0 1px color-mix(in srgb, var(--tyba-amber-fill) 80%, transparent)";
  const shadow = open
    ? attn
      ? `var(--tyba-pill-on), ${ring}`
      : "var(--tyba-pill-on)"
    : attn
      ? ring
      : undefined;
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      aria-label={label}
      title={label}
      className={`inline-flex h-[18px] shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[4px] px-[7px] transition-colors ${
        open
          ? "bg-[color-mix(in_srgb,var(--tyba-text)_9%,transparent)] text-tyba-text"
          : "text-tyba-text-muted hover:bg-tyba-text/[.06]"
      }`}
      style={shadow ? { boxShadow: shadow } : undefined}
    >
      <Queue size={12} className="shrink-0 text-tyba-violet" />
      <span className={`text-tyba-text ${compact ? "font-mono" : ""}`}>
        {compact ? queueCount : t("promptQueueButton", { count: queueCount })}
      </span>
      {agents && (
        <>
          <span className="h-2.5 w-px shrink-0 bg-tyba-border-strong" />
          <AgentIcon size={12} className="shrink-0 text-tyba-amber" />
          {compact ? (
            <span className="font-mono">
              {agents.done}/{agents.total}
            </span>
          ) : (
            <span className="text-tyba-text">
              {t("agentsTitle")}{" "}
              <span className="font-mono text-tyba-text-muted">
                {agents.done}/{agents.total}
              </span>
            </span>
          )}
          {state?.kind === "run" && (
            <span className="inline-flex items-center gap-[5px] text-tyba-green">
              <span className={`${DOT} bg-tyba-green-fill [box-shadow:var(--tyba-glow-green)]`} />
              {!stateCompact && stateText}
            </span>
          )}
          {state?.kind === "ok" && (
            <span className="inline-flex items-center gap-[5px] text-tyba-green">
              <Check size={11} />
              {!stateCompact && stateText}
            </span>
          )}
          {state?.kind === "attn" && (
            <span className="inline-flex items-center gap-[5px] text-tyba-amber">
              <span className={`${DOT} bg-tyba-amber-fill`} />
              {stateCompact ? <span className="font-mono">{state.count}</span> : stateText}
            </span>
          )}
          {state?.kind === "fail" && (
            <span className="inline-flex items-center gap-[5px] text-tyba-red">
              <span className={`${DOT} bg-tyba-red-fill`} />
              {stateCompact ? <span className="font-mono">{state.count}</span> : stateText}
            </span>
          )}
        </>
      )}
      {turn && (
        <>
          <span className="h-2.5 w-px shrink-0 bg-tyba-border-strong" />
          <PlusMinus
            size={12}
            className={`shrink-0 ${turnFailed ? "text-tyba-red" : "text-tyba-cyan"}`}
          />
          {compact ? (
            pillTurnCount(turn) !== null && (
              <span className={`font-mono ${turnFailed ? "text-tyba-red" : ""}`}>
                {pillTurnCount(turn)}
              </span>
            )
          ) : (
            <>
              <span className={turnFailed ? "text-tyba-red" : "text-tyba-text"}>
                {pillTurnHead(turn, t)}
              </span>
              {turn.body.kind === "counts" && (
                <>
                  <span className="font-mono text-tyba-text-muted">
                    {t("diffFilesSummary", { count: turn.body.files })}
                  </span>
                  <span className="inline-flex gap-1 whitespace-nowrap font-mono tabular-nums">
                    <span className="text-tyba-green">+{turn.body.insertions}</span>
                    <span className="text-tyba-red">−{turn.body.deletions}</span>
                  </span>
                </>
              )}
              {turn.body.kind === "none" && (
                <span className="text-tyba-text-faint">{t("pillTurnNoChanges")}</span>
              )}
              {turn.body.kind === "no_repo" && (
                <span className="text-tyba-text-faint">{t("pillTurnNoGit")}</span>
              )}
            </>
          )}
        </>
      )}
      <span className="-ml-px flex text-tyba-text-faint">
        {open ? <CaretDown size={10} /> : <CaretUp size={10} />}
      </span>
    </button>
  );
}
