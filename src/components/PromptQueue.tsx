import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretUp,
  DotsSixVertical,
  Pause,
  Play,
  PlusMinus,
  Rows,
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
import { handoffSource, requestHandoff } from "../lib/handoff";
import { queueIndicator, reorderIds, slotAt, type QueueTone } from "../lib/promptQueue";
import { toastError } from "../lib/toast";
import { formatElapsed } from "../lib/agentTree";
import { useNow } from "./ui/loading";
import { AgentsPanel, RobotIcon } from "./AgentsPanel";
import { PLATE, PLATE_FLOAT, PLATE_LINE } from "./plate";
import { GLASS_BTN, TurnPanel } from "./TurnPanel";
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
  /** Não entra mais na gaveta: o agente principal usa o robô (quadro Agentes). */
  runner: AgentRunner | null;
  /** Subagente com o detalhe aberto na lateral (4E). */
  selectedSubagent: string | null;
  onSelectSubagent: (agentId: string) => void;
}

// Quadro Agentes (g6-state): "Envia quando o turno terminar" em verde sobre a
// tinta azul; as pausas seguem na cor do motivo.
const TONE: Record<QueueTone, string> = {
  send: "text-tyba-green bg-tyba-blue-tint",
  approval: "text-tyba-amber bg-tyba-amber-tint",
  failed: "text-tyba-red bg-tyba-red-tint",
  stopped: "text-tyba-text-muted bg-tyba-neutral-tint",
};

/** Botão fantasma só de ícone do quadro (limpar, recolher). */
const VBTN =
  "inline-flex size-[34px] shrink-0 items-center justify-center rounded-[10px] text-tyba-text-muted outline-none hover:bg-tyba-hover hover:text-tyba-text focus-visible:[box-shadow:var(--tyba-focus-ring)] disabled:pointer-events-none disabled:opacity-45";

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
      <CaretDown size={16} />
    </button>
  );

  const pauseButton = paused ? (
    <button
      type="button"
      className={GLASS_BTN}
      onClick={() => void promptQueueResume(sessionId).catch(fail)}
    >
      <Play size={16} className="shrink-0" />
      {t("promptQueueResume")}
    </button>
  ) : (
    <button
      type="button"
      disabled={prompts.length === 0}
      className={GLASS_BTN}
      onClick={() => void promptQueuePause(sessionId).catch(fail)}
    >
      <Pause size={16} className="shrink-0" />
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
      aria-label={t(
        hasSubagents ? "agentsDrawerLabel" : turn ? "turnDrawerLabel" : "promptQueueTitle",
      )}
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
      {/* Quadro Agentes (g4-qh): as abas no segmento; na Fila, Pausar e
          limpar; nas outras, recolher. */}
      <div className="flex items-center gap-2 px-3 pb-2 pt-2.5">
        <DrawerTabs
          tab={tab}
          queueCount={prompts.length}
          agents={hasSubagents ? progress : null}
          turnCount={turn ? turnTabCount(turn) : undefined}
          pending={pending}
          // Na Fila, Pausar e limpar dividem a linha: com as três abas os
          // rótulos não cabem nos 430px, e as abas ficam só com ícone e número
          // (o nome no tooltip), como na gaveta estreita.
          compact={box.box.compact || (tab === "queue" && hasSubagents && turn !== null)}
          onTab={setTab}
        />
        <span className="flex-1" />
        {tab === "queue" ? (
          <span className="flex shrink-0 gap-1">
            {pauseButton}
            <button
              type="button"
              disabled={clearDisabled}
              aria-label={t("promptQueueClear")}
              title={t("promptQueueClear")}
              className={VBTN}
              onClick={clear}
            >
              <Trash size={16} />
            </button>
          </span>
        ) : (
          collapseButton
        )}
      </div>
      {tab === "agents" ? (
        <AgentsPanel
          session={session}
          snapshot={snapshot}
          approvals={approvals}
          orchestratorTitle={orchestratorTitle ?? agentName}
          selectedId={selectedSubagent}
          onSelect={onSelectSubagent}
        />
      ) : tab === "turn" && turn ? (
        <TurnPanel
          sessionId={sessionId}
          turn={turn}
          turnOpen={Boolean(session.turn_open)}
          onHandoff={
            handoffSource(runner)
              ? () => requestHandoff({ sessionId, from: agentName, turn })
              : undefined
          }
        />
      ) : (
        <>
          <div
            className={`mx-2.5 mb-1 mt-0.5 flex items-center gap-2 rounded-[10px] px-3 py-[9px] text-[12.5px] font-semibold ${TONE[indicator.tone]}`}
          >
            {indicator.tone === "send" ? (
              <span className="size-[7px] shrink-0 rounded-full bg-tyba-green-fill" />
            ) : (
              <Pause size={13} className="shrink-0" />
            )}
            <span className="shrink-0">{t(indicator.label)}</span>
            {goToAgents ? (
              <button
                type="button"
                onClick={() => setTab("agents")}
                className="ml-auto shrink-0 whitespace-nowrap text-[11px] font-normal text-tyba-amber underline underline-offset-2"
              >
                {t("agentsDrawerSeeAgents")}
              </button>
            ) : (
              indicator.detail && (
                <span className="ml-auto min-w-0 truncate text-[11px] font-normal text-tyba-text-muted">
                  {"text" in indicator.detail
                    ? indicator.detail.text
                    : t(indicator.detail.key, indicator.detail.params)}
                </span>
              )
            )}
          </div>
          {prompts.length === 0 ? (
            <p className="mx-3.5 mb-2.5 mt-1 text-[12.5px] leading-[19px] text-tyba-text-muted">
              {t("promptQueueEmpty", { agent: agentName })}
            </p>
          ) : (
            <div className="mt-1 flex max-h-[40vh] flex-col gap-px overflow-y-auto px-1.5 pb-1.5">
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
          {/* Quadro Agentes (g6-foot): o campo do Muriki, com o atalho dentro. */}
          <div className="px-3 pb-3 pt-2.5 shadow-[inset_0_1px_0_var(--tyba-line)]">
            <div className="flex h-10 items-center gap-2 rounded-[10px] bg-[var(--tyba-glass-piece)] pl-4 pr-3 shadow-[0_0_0_1px_var(--tyba-line)] focus-within:[box-shadow:var(--tyba-focus-ring)]">
              <textarea
                rows={1}
                // O que sai daqui vai para o agente como foi escrito.
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
                value={draft}
                aria-label={t("promptQueueAdd")}
                placeholder={t("promptQueueAdd")}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    add();
                  }
                }}
                className="h-5 min-w-0 flex-1 resize-none bg-transparent text-[14px] font-medium leading-5 text-tyba-text outline-none placeholder:text-tyba-text-faint"
              />
              <kbd className="min-w-[18px] rounded-[4px] bg-[color-mix(in_srgb,var(--tyba-text)_6%,transparent)] px-1.5 text-center font-mono text-[11px] font-medium leading-[18px] text-tyba-text-faint">
                ⌘⏎
              </kbd>
            </div>
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
  // Quadro Agentes (g4-seg): segmento em pílula, a placa branca no escolhido.
  const item = (on: boolean) =>
    `flex h-7 min-w-0 items-center whitespace-nowrap rounded-full text-[12px] font-semibold transition-colors focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
      compact ? "gap-[5px] px-2.5" : "gap-1.5 px-3"
    } ${on ? "tyba-pill-on text-tyba-text" : "text-tyba-text-muted hover:text-tyba-text"}`;
  const queueName = `${t("agentsDrawerQueue")} · ${queueCount}`;
  const agentsName = agents && `${t("agentsTitle")} · ${agents.done}/${agents.total}`;
  const turnName = `${t("pillTurn")} · ${turnCount ?? "–"}`;
  return (
    <div role="tablist" className="tyba-segment min-w-0 rounded-full! p-1!">
      <button
        type="button"
        role="tab"
        aria-selected={tab === "queue"}
        aria-label={compact ? queueName : undefined}
        title={compact ? queueName : undefined}
        className={item(tab === "queue")}
        onClick={() => onTab("queue")}
      >
        <Rows size={15} className="shrink-0" />
        {compact ? queueCount : (
          <>
            <span className="truncate">{t("agentsDrawerQueue")}</span>
            <span className="shrink-0">· {queueCount}</span>
          </>
        )}
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
          <RobotIcon size={15} />
          {compact ? (
            <span className="font-mono">
              {agents.done}/{agents.total}
            </span>
          ) : (
            <>
              <span className="truncate">{t("agentsTitle")}</span>
              <span className="shrink-0">
                · {agents.done}/{agents.total}
              </span>
            </>
          )}
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
          <PlusMinus size={15} className="shrink-0" />
          {compact ? (turnCount ?? "–") : (
            <>
              <span className="truncate">{t("pillTurn")}</span>
              <span className="shrink-0">· {turnCount ?? "–"}</span>
            </>
          )}
        </button>
      )}
    </div>
  );
}

const DOT = "size-[6px] shrink-0 rounded-full";

/**
 * A pílula da gaveta, chip do rodapé do app (quadro Agentes, g3-pill): pílula
 * neutra "Fila · N | Agentes x/y | ± Turno estado ⌄". O estado da rodada
 * fica no ponto ao lado da contagem; pede você: contorno inteiro amarelo.
 * Aberta, a placa do selecionado. Compacta (degrau 6 da escada): só ícones e
 * números, o texto inteiro no tooltip e no `aria-label` — o estado (cor,
 * contorno, número) nunca sai. Turno que falhou: trecho vermelho.
 */
export function DrawerPill({
  queueCount,
  agents,
  turn,
  runningSince,
  open,
  compact,
  onToggle,
}: {
  queueCount: number;
  agents: PillAgents | null;
  turn: PillTurn | null;
  runningSince: number | null;
  open: boolean;
  compact: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  // O tempo do turno que roda, contado aqui a partir do começo que o core mandou.
  useNow(runningSince !== null);
  const elapsed = runningSince !== null ? formatElapsed(Date.now() - runningSince) : null;
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
    elapsed && `${t("pillTurnRunning")} ${elapsed}`,
    turn && pillTurnLabel(turn, t),
  ]
    .filter(Boolean)
    .join(" · ");
  const turnFailed = turn?.outcome === "failed";
  const ring = "inset 0 0 0 1.5px var(--tyba-amber-fill)";
  const shadow = open ? (attn ? `var(--tyba-pill-on), ${ring}` : undefined) : attn ? ring : undefined;
  const sep = <span className="h-3 w-px shrink-0 bg-[var(--tyba-line)]" />;
  const em = "font-mono font-normal text-tyba-text-faint";
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      aria-label={label}
      title={label}
      className={`inline-flex h-[26px] shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 font-semibold transition-colors ${
        open
          ? "tyba-pill-on text-tyba-text"
          : "bg-[color-mix(in_srgb,var(--tyba-text)_5%,transparent)] text-tyba-text-muted hover:text-tyba-text"
      }`}
      style={shadow ? { boxShadow: shadow } : undefined}
    >
      <Rows size={13} className="shrink-0" />
      <span className={compact ? "font-mono" : ""}>
        {compact ? queueCount : t("promptQueueButton", { count: queueCount })}
      </span>
      {agents && (
        <>
          {sep}
          {compact ? <RobotIcon size={13} /> : <span>{t("agentsTitle")}</span>}
          <span className={em}>
            {agents.done}/{agents.total}
          </span>
          {state?.kind === "run" && (
            <span className={`${DOT} bg-tyba-green-fill [box-shadow:var(--tyba-glow-green)]`} />
          )}
          {state?.kind === "attn" && (
            <span className="inline-flex items-center gap-[5px] text-tyba-amber">
              <span className={`${DOT} bg-tyba-amber-fill`} />
              <span className="font-mono">{state.count}</span>
            </span>
          )}
          {state?.kind === "fail" && (
            <span className="inline-flex items-center gap-[5px] text-tyba-red">
              <span className={`${DOT} bg-tyba-red-fill`} />
              <span className="font-mono">{state.count}</span>
            </span>
          )}
        </>
      )}
      {elapsed && (
        <>
          {sep}
          {!compact && <span>{t("pillTurnRunning")}</span>}
          <span className={`${em} tabular-nums`}>{elapsed}</span>
        </>
      )}
      {turn && (
        <>
          {sep}
          <span className={turnFailed ? "text-tyba-red" : ""}>
            ± {compact ? "" : pillTurnHead(turn, t)}
          </span>
          {compact ? (
            pillTurnCount(turn) !== null && (
              <span className={`${em} ${turnFailed ? "text-tyba-red" : ""}`}>
                {pillTurnCount(turn)}
              </span>
            )
          ) : (
            <>
              {turn.body.kind === "counts" && (
                <>
                  <span className={em}>{t("diffFilesSummary", { count: turn.body.files })}</span>
                  <span className="inline-flex gap-1 whitespace-nowrap font-mono font-normal tabular-nums">
                    <span className="text-tyba-green">+{turn.body.insertions}</span>
                    <span className="text-tyba-red">−{turn.body.deletions}</span>
                  </span>
                </>
              )}
              {turn.body.kind === "none" && <span className={em}>{t("pillTurnNoChanges")}</span>}
              {turn.body.kind === "no_repo" && <span className={em}>{t("pillTurnNoGit")}</span>}
            </>
          )}
        </>
      )}
      <span className="flex text-tyba-text-faint">
        {/* Quadro: recolhida mostra o caret para baixo. */}
        {open ? <CaretUp size={11} /> : <CaretDown size={11} />}
      </span>
    </button>
  );
}
