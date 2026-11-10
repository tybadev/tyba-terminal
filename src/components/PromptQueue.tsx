import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  Check,
  DotsSixVertical,
  Pause,
  Play,
  Queue,
  Trash,
  X,
} from "@phosphor-icons/react";

import { translateError } from "../lib/errors";
import {
  agentsDrawerCollapse,
  agentsDrawerOpen,
  agentsDrawerPresence,
  agentsDrawerTab,
  onPromptQueueChanged,
  promptQueueClear,
  promptQueueEnqueue,
  promptQueueList,
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
  drawerPill,
  drawerTab,
  pendingSubagentApprovals,
  queueGoesToAgents,
  roundProgress,
  type PillAgents,
} from "../lib/agentsDrawer";
import {
  queueIndicator,
  reorderIds,
  slotAt,
  type QueueState,
  type QueueTone,
} from "../lib/promptQueue";
import { toastError } from "../lib/toast";
import { AgentsPanel } from "./AgentsPanel";
import { AgentIcon } from "./icons/AgentIcon";
import { PLATE, PLATE_FLOAT, PLATE_LINE } from "./plate";

interface Props {
  session: Session;
  /** Distância da gaveta (e da pílula) ao rodapé do pane: fica acima do
   *  campo de entrada. */
  bottomPx: number;
  /** Aberta/pílula e a aba do dono, decididos pelo core. */
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
 * Quadros 4A′–4E (GavetaFilaAgentes): a pílula e a gaveta Fila/Agentes presas
 * ao canto inferior direito do pane do agente. A pílula é a gaveta recolhida,
 * no mesmo canto. Abrir, recolher e a aba do dono são do core
 * (`agents-drawer://changed`); a fila também (`prompt-queue://changed`).
 * Daqui só saem intenções.
 */
export function PromptQueue({
  session,
  bottomPx,
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
  const [queue, setQueue] = useState<QueueState | null>(null);
  const [draft, setDraft] = useState("");
  const [drag, setDrag] = useState<{ from: number; slot: number } | null>(null);
  const [hover, setHover] = useState(false);
  const [focusWithin, setFocusWithin] = useState(false);
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

  const open = drawer?.open ?? false;
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
  const pill = drawerPill(subagents, pending, drawer?.outcome_seen ?? false);
  const tab = drawerTab({
    owner: drawer?.tab ?? null,
    hasSubagents,
    pending,
    running: progress?.running ?? false,
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

  if (!open) {
    return (
      <DrawerPill
        bottomPx={bottomPx}
        queueCount={prompts.length}
        agents={pill}
        onOpen={() => void agentsDrawerOpen(sessionId).catch(() => {})}
      />
    );
  }

  return (
    // Acima da lista de blocos (z-10) e da busca do terminal (z-20).
    <div
      role="region"
      aria-label={t(hasSubagents ? "agentsDrawerLabel" : "promptQueueTitle")}
      className={`absolute right-[18px] z-30 w-[430px] max-w-[calc(100%-36px)] overflow-hidden ${PLATE} ${PLATE_LINE} ${PLATE_FLOAT}`}
      style={{ bottom: bottomPx }}
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
        {hasSubagents ? (
          <>
            <DrawerTabs
              tab={tab}
              queueCount={prompts.length}
              done={progress?.done ?? 0}
              total={progress?.total ?? 0}
              pending={pending}
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
}

/** As abas do topo da gaveta (4C): o mesmo segmento das abas da janela. */
function DrawerTabs({
  tab,
  queueCount,
  done,
  total,
  pending,
  onTab,
}: {
  tab: DrawerTab;
  queueCount: number;
  done: number;
  total: number;
  pending: number;
  onTab: (tab: DrawerTab) => void;
}) {
  const { t } = useTranslation();
  const item = (on: boolean) =>
    `flex h-[26px] items-center gap-1.5 whitespace-nowrap rounded-[5px] px-2.5 text-[12px] ${
      on ? "tyba-pill-on text-tyba-text" : "text-tyba-text-faint hover:text-tyba-text-muted"
    }`;
  return (
    <div
      role="tablist"
      className="flex gap-0.5 rounded-[7px] bg-[color-mix(in_srgb,var(--tyba-text)_5%,transparent)] p-0.5"
    >
      <button
        type="button"
        role="tab"
        aria-selected={tab === "queue"}
        className={item(tab === "queue")}
        onClick={() => onTab("queue")}
      >
        <Queue size={13} className="shrink-0 text-tyba-violet" />
        {t("agentsDrawerQueue")}
        <span className="text-[11px] text-tyba-text-faint">{queueCount}</span>
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={tab === "agents"}
        className={item(tab === "agents")}
        onClick={() => onTab("agents")}
      >
        <AgentIcon size={13} className="shrink-0 text-tyba-amber" />
        {t("agentsTitle")}
        <span className="font-mono text-[11px] text-tyba-text-faint">
          {done}/{total}
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
    </div>
  );
}

/**
 * A gaveta recolhida (4A′/4D): sempre "Fila · N"; com subagentes, "Agentes
 * x/y" e o estado da rodada. Pede atenção com o contorno inteiro em âmbar.
 */
function DrawerPill({
  bottomPx,
  queueCount,
  agents,
  onOpen,
}: {
  bottomPx: number;
  queueCount: number;
  agents: PillAgents | null;
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const attn = agents?.state?.kind === "attn";
  const state = agents?.state;
  return (
    // Acima da lista de blocos (z-10), abaixo da busca do terminal (z-20).
    <button
      type="button"
      onClick={onOpen}
      aria-expanded={false}
      className={`absolute right-[18px] z-[15] inline-flex h-[30px] items-center gap-2 whitespace-nowrap rounded-full pl-[11px] pr-3 text-[12px] text-tyba-text ${PLATE_FLOAT} ${
        attn
          ? "[box-shadow:inset_0_0_0_1.5px_color-mix(in_srgb,var(--tyba-amber-fill)_75%,transparent),var(--tyba-shadow-lg)]"
          : "[box-shadow:var(--tyba-plate-shadow,var(--tyba-edge),var(--tyba-shadow-lg))] hover:[box-shadow:inset_0_0_0_1px_var(--tyba-border-strong),var(--tyba-shadow-lg)]"
      }`}
      style={{ bottom: bottomPx }}
    >
      <Queue size={13} className="shrink-0 text-tyba-violet" />
      <span>{t("promptQueueButton", { count: queueCount })}</span>
      {agents && (
        <>
          <span className="h-3.5 w-px bg-tyba-border-strong" />
          <AgentIcon size={13} className="shrink-0 text-tyba-amber" />
          <span>
            {t("agentsTitle")}{" "}
            <span className="font-mono text-tyba-text-muted">
              {agents.done}/{agents.total}
            </span>
          </span>
          {state?.kind === "run" && (
            <span className="inline-flex items-center gap-1.5 text-[11.5px] text-tyba-green">
              <span className="size-[7px] rounded-full bg-tyba-green-fill [box-shadow:var(--tyba-glow-green)]" />
              {t("subagentsStatusRunning")}
            </span>
          )}
          {state?.kind === "ok" && (
            <span className="inline-flex items-center gap-1.5 text-[11.5px] text-tyba-green">
              <Check size={12} />
              {t("subagentsStatusDone")}
            </span>
          )}
          {state?.kind === "attn" && (
            <span className="inline-flex items-center gap-1.5 text-[11.5px] text-tyba-amber">
              <span className="size-[7px] rounded-full bg-tyba-amber-fill" />
              {t("agentsDrawerAsksYou", { count: state.count })}
            </span>
          )}
          {state?.kind === "fail" && (
            <span className="inline-flex items-center gap-1.5 text-[11.5px] text-tyba-red">
              <span className="size-[7px] rounded-full bg-tyba-red-fill" />
              {t("agentsCountFailed", { count: state.count })}
            </span>
          )}
        </>
      )}
    </button>
  );
}
