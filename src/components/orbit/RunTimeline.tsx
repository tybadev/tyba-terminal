import { Fragment } from "react";
import { useTranslation } from "react-i18next";
import { CaretRight, Warning } from "@phosphor-icons/react";

import {
  concurrentAgents,
  formatDuration,
  nodeElapsed,
  nodeStatusLabel,
  timelineSteps,
  type Graph,
  type Run,
} from "@/lib/orbit";
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
}: {
  graph: Pick<Graph, "nodes" | "edges">;
  run: Run | null;
  number: number;
  now: number;
}) {
  const { t, i18n } = useTranslation();
  const pair = concurrentAgents(graph);
  const steps = timelineSteps(graph);
  const title = (id: string) => graph.nodes.find((n) => n.id === id)?.title ?? id;

  const step = (id: string) => {
    const n = run?.nodes[id];
    const label = run ? nodeStatusLabel(graph, run, id) : null;
    const elapsed = nodeElapsed(n, now);
    const tone = n ? STATUS_TONE[n.status] : null;
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
      {run ? (
        <>
          <div className="flex shrink-0 flex-col gap-[3px]">
            <span className="text-[13px] font-medium text-tyba-text">
              {t("orbitRunNumber", { n: number })}{" "}
              <span className="font-normal text-tyba-text-faint">
                · {formatDuration((run.finishedAt ?? now) - run.startedAt)}
              </span>
            </span>
            <span className="text-[11px] text-tyba-text-faint">
              {run.status === "running"
                ? t("orbitRunStartedAt", {
                    time: new Date(run.startedAt).toLocaleTimeString(i18n.language, {
                      hour: "2-digit",
                      minute: "2-digit",
                    }),
                  })
                : t(`orbitRunStatus_${run.status}`)}
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
