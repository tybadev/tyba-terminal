import { Fragment, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretRight, Warning } from "@phosphor-icons/react";

import {
  concurrentAgents,
  formatDuration,
  isLiveRun,
  liveRunNodes,
  nodeElapsed,
  nodeStatusLabel,
  runElapsed,
  runStatusKey,
  shownStatus,
  timelineSteps,
  type Graph,
  type Run,
} from "@/lib/orbit";
import { waitedFor, waitingQueue, waitingYouOf } from "@/lib/orbitCard";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { STATUS_TONE } from "./nodes/NodeFrame";

/**
 * Linha da execução (quadro OrbitHarness, embaixo do canvas): o run à vista
 * com número e tempo, um passo por nó com estado e tempo — nós do mesmo nível
 * juntos, porque rodam em paralelo — e o aviso de agentes na mesma pasta.
 */
export function RunTimeline({
  graph,
  run,
  number,
  now,
  onOpen,
}: {
  graph: Pick<Graph, "nodes" | "edges">;
  run: Run | null;
  number: number;
  now: number;
  /** "Abrir" na fila de atenção: foca e abre o nó. */
  onOpen: (nodeId: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const queue = waitingQueue(run, graph.nodes);
  const [queueOpen, setQueueOpen] = useState(false);
  const pair = concurrentAgents(graph);
  const steps = timelineSteps(graph);
  const title = (id: string) => graph.nodes.find((n) => n.id === id)?.title ?? id;

  const step = (id: string) => {
    const n = run?.nodes[id];
    // Quadro OrbitEsperando: "rev ● esperando você 2m", ponto âmbar.
    const waiting = waitingYouOf(n);
    if (waiting) {
      return (
        <span key={id} className="flex items-center gap-2 text-[11.5px] whitespace-nowrap">
          <span className="text-tyba-text">{title(id)}</span>
          <i className="size-[7px] shrink-0 rounded-full bg-tyba-amber" />
          <span className="text-tyba-amber">{t("orbitWaitingYouStep")}</span>
          <span className="text-tyba-text-faint">{waitedFor(now - waiting.since, true)}</span>
        </span>
      );
    }
    const label = run ? nodeStatusLabel(graph, run, id) : null;
    const elapsed = nodeElapsed(n, now, run);
    const tone = n ? STATUS_TONE[shownStatus(run, n)] : null;
    return (
      <span key={id} className="flex items-center gap-2 text-[11.5px] whitespace-nowrap">
        <span className="text-tyba-text">{title(id)}</span>
        {tone && <i className={`size-[7px] shrink-0 rounded-full ${tone.dot}`} />}
        {label && (
          <span className={tone?.text}>
            {t(label.key, {
              ...label.params,
              ...(label.params?.kind ? { kind: t(`orbitEdge_${label.params.kind}`) } : {}),
            })}
          </span>
        )}
        {elapsed !== null && (
          <span className="text-tyba-text-faint">{formatDuration(elapsed)}</span>
        )}
      </span>
    );
  };

  return (
    <footer
      aria-label={t("orbitRunCurrent")}
      className="flex h-16 shrink-0 items-center gap-5 border-t border-tyba-border bg-tyba-surface px-[18px] font-mono"
    >
      {run && isLiveRun(run) ? (
        // Quadro OrbitTerminal 4: a sessão ao vivo do Terminal, sem número
        // nem passos do grafo — o Terminal e o que ele criou, cada um com o estado.
        <>
          <div className="flex shrink-0 flex-col gap-[3px]">
            <span className="text-[13px] font-medium text-tyba-green">{t("orbitLiveRun")}</span>
            <span className="text-[11px] text-tyba-text-faint">{t("orbitLiveRunHint")}</span>
          </div>
          <ol
            aria-label={t("orbitRunSteps")}
            className="m-0 flex min-w-0 flex-1 list-none items-center gap-5 overflow-x-auto p-0"
          >
            {liveRunNodes(graph, run).map((id) => (
              <li key={id} className="flex">
                {step(id)}
              </li>
            ))}
          </ol>
        </>
      ) : run ? (
        <>
          <div className="flex shrink-0 flex-col gap-[3px]">
            <span className="text-[13px] font-medium text-tyba-text">
              {t("orbitRunNumber", { n: number })}{" "}
              <span className="font-normal text-tyba-text-faint">
                · {formatDuration(runElapsed(run, now))}
              </span>
            </span>
            <span className="text-[11px] text-tyba-text-faint">
              {run.status === "running" && !run.settled
                ? t("orbitRunStartedAt", {
                    time: new Date(run.startedAt).toLocaleTimeString(i18n.language, {
                      hour: "2-digit",
                      minute: "2-digit",
                    }),
                  })
                : t(runStatusKey(run))}
            </span>
          </div>
          <ol
            aria-label={t("orbitRunSteps")}
            className="m-0 flex min-w-0 flex-1 list-none items-center gap-3 overflow-x-auto p-0"
          >
            {steps.map((group, i) => (
              <Fragment key={group.join(",")}>
                {i > 0 && (
                  <li aria-hidden className="flex shrink-0 text-tyba-text-faint">
                    <CaretRight size={12} />
                  </li>
                )}
                {group.length > 1 ? (
                  <li
                    aria-label={t("orbitRunParallel")}
                    className="flex flex-col gap-[5px] border-l-[3px] border-double border-tyba-text-faint pl-3"
                  >
                    {group.map(step)}
                  </li>
                ) : (
                  <li className="flex">{step(group[0])}</li>
                )}
              </Fragment>
            ))}
          </ol>
        </>
      ) : (
        <span className="flex-1 text-[12px] text-tyba-text-faint">{t("orbitRunNone")}</span>
      )}
      {queue.length > 0 && (
        <Popover open={queueOpen} onOpenChange={setQueueOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              className="shrink-0 rounded-[8px] border border-tyba-amber/50 bg-tyba-surface px-[11px] py-[5px] text-[11.5px] whitespace-nowrap text-tyba-amber hover:bg-tyba-raised"
            >
              {t("orbitWaitingYouCount", { count: queue.length })}
            </button>
          </PopoverTrigger>
          <PopoverContent
            side="top"
            align="end"
            sideOffset={8}
            className="w-[420px] overflow-hidden rounded-[14px] p-0 font-mono"
          >
            <div className="flex items-center justify-between px-3.5 py-3">
              <span className="font-sans text-[13px] font-semibold">{t("orbitWaitingYouTitle")}</span>
              <span className="text-[11px] text-tyba-text-faint">
                {t("orbitWaitingYouOrder", { n: number })}
              </span>
            </div>
            {queue.map((row) => (
              <div
                key={row.id}
                className="grid grid-cols-[10px_1fr_auto] items-center gap-2.5 border-t border-tyba-border px-3.5 py-2.5 text-[12px]"
              >
                <i className="size-2 rounded-full bg-tyba-amber" />
                <span className="min-w-0 truncate">
                  <span className="text-tyba-text">{row.title}</span>{" "}
                  <span className="text-tyba-text-muted">
                    ·{" "}
                    {row.hint
                      ? t("orbitWaitingYouReasonHint", {
                          reason: t(`orbitWaitReason_${row.reason}`),
                          hint: row.hint,
                        })
                      : t(`orbitWaitReason_${row.reason}`)}
                  </span>
                </span>
                <button
                  type="button"
                  onClick={() => {
                    setQueueOpen(false);
                    onOpen(row.id);
                  }}
                  className="rounded-[8px] border border-tyba-border bg-tyba-surface px-[11px] py-[5px] text-[11.5px] text-tyba-text hover:bg-tyba-raised"
                >
                  {t("orbitWaitingYouOpen")}
                </button>
              </div>
            ))}
            <div className="border-t border-tyba-border px-3.5 py-2.5 text-[11px] text-tyba-text-faint">
              {t("orbitWaitingYouLeaves")}
            </div>
          </PopoverContent>
        </Popover>
      )}
      {pair && (
        <span
          role="note"
          className="inline-flex shrink-0 items-center gap-2 rounded-[6px] border border-tyba-amber/45 bg-tyba-amber-tint px-[11px] py-1.5 text-[11.5px] whitespace-nowrap text-tyba-amber"
        >
          <Warning size={13} />
          {t("orbitParallelAgents", { a: pair[0].title, b: pair[1].title })}
        </span>
      )}
    </footer>
  );
}
