import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, Cube } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { Select, type SelectOption } from "@/components/ui/select";
import {
  dockerListContainers,
  listSessionTunnels,
  listSessions,
  servicesSessionPorts,
} from "@/lib/ipc";
import {
  SERVICE_SOURCES,
  emptySource,
  onServiceLogs,
  serviceLogsStart,
  sourceDetail,
  urlPort,
  type NodeKind,
  type ServiceLogLine,
  type ServiceSource,
  type ServiceSourceType,
} from "@/lib/maestro";
import { NodeFrame, fieldClass, useMaestro, type MaestroFlowNode } from "./NodeFrame";

type ServiceKind = Extract<NodeKind, { type: "service" }>;

/** Uma opção do seletor e a fonte que ela vira. */
interface Target {
  option: SelectOption;
  source: ServiceSource;
}

const keyOf = (s: ServiceSource): string => {
  switch (s.type) {
    case "container":
      return s.containerId;
    case "port":
      return `${s.sessionId ?? ""}|${s.port}`;
    case "tunnel":
      return `${s.sessionId}|${s.tunnelId}`;
  }
};

/** O que existe agora para cada tipo de fonte, pelas IPC que o TYBA já tem. */
async function loadTargets(type: ServiceSourceType): Promise<Target[]> {
  if (type === "container") {
    const list = await dockerListContainers(null, false);
    return list.map((c) => ({
      option: { value: c.id, label: c.ports ? `${c.name} · ${c.ports}` : c.name },
      source: { type, containerId: c.id, name: c.name },
    }));
  }
  const sessions = (await listSessions()).value;
  const title = new Map(sessions.map((s) => [s.id, s.title]));
  if (type === "port") {
    const ports = await servicesSessionPorts();
    return Object.entries(ports).flatMap(([sessionId, list]) =>
      list.map((port) => {
        const source: ServiceSource = { type, sessionId, port };
        return {
          option: { value: keyOf(source), label: `${title.get(sessionId) ?? "?"} · :${port}` },
          source,
        };
      }),
    );
  }
  const ssh = sessions.filter((s) => s.kind.type === "ssh");
  const lists = await Promise.all(
    ssh.map((s) => listSessionTunnels(s.id).catch(() => [])),
  );
  return lists.flat().flatMap((tn) => {
    // Só o -L serve página aqui (o core recusa o resto na execução).
    if (tn.kind !== "local") return [];
    const source: ServiceSource = { type, sessionId: tn.session_id, tunnelId: tn.id };
    const target = `${tn.target_host ?? "?"}:${tn.target_port ?? "?"}`;
    return [
      {
        option: {
          value: keyOf(source),
          label: `${title.get(tn.session_id) ?? "ssh"} · :${tn.listen_port} → ${target}`,
        },
        source,
      },
    ];
  });
}

const LOG_KEEP = 200;

/** Log ao vivo do contêiner: o que o seguidor já leu e o que chega por evento. */
function useContainerLog(containerId: string | null): { lines: ServiceLogLine[]; error: string | null } {
  const [lines, setLines] = useState<ServiceLogLine[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setLines([]);
    setError(null);
    if (!containerId) return;
    let alive = true;
    let unlisten: (() => void) | null = null;
    const add = (more: ServiceLogLine[]) =>
      setLines((prev) => {
        const seen = new Set(prev.map((l) => l.seq));
        return [...prev, ...more.filter((l) => !seen.has(l.seq))].slice(-LOG_KEEP);
      });
    void onServiceLogs(containerId, (batch) => alive && add(batch.lines))
      .then((un) => {
        if (!alive) return un();
        unlisten = un;
        return serviceLogsStart(containerId).then((snap) => alive && add(snap.lines));
      })
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [containerId]);
  return { lines, error };
}

/**
 * Nó de serviço (quadro MaestroNavegador): liga num serviço que já roda —
 * contêiner, porta de uma sessão ou túnel `-L`. Saídas log e url. O contêiner
 * mostra o log ao vivo; quem decide se o alvo existe é o core, na execução.
 */
export function ServiceNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run } = useMaestro();
  const node = data.node;
  const { source } = node.kind as ServiceKind;
  const nodeRun = run?.nodes[id];
  const runUrl = nodeRun?.url ?? null;

  const [targets, setTargets] = useState<Target[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    let alive = true;
    setLoadError(null);
    loadTargets(source.type)
      .then((list) => alive && setTargets(list))
      .catch((e) => {
        if (!alive) return;
        setTargets([]);
        setLoadError(String(e));
      });
    return () => {
      alive = false;
    };
  }, [source.type, refresh]);

  const setSource = (next: ServiceSource) =>
    updateNode(id, (n) => ({ ...n, kind: { type: "service", source: next } }));

  const containerId =
    source.type === "container" && source.containerId ? source.containerId : null;
  const log = useContainerLog(containerId);
  // Rola só a caixa do log (`scrollIntoView` rolaria o canvas junto).
  const logBox = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const box = logBox.current;
    if (box) box.scrollTop = box.scrollHeight;
  }, [log.lines.length]);

  const port = urlPort(runUrl) ?? (source.type === "port" && source.port > 0 ? source.port : null);
  const detail = [sourceDetail(source), port && source.type !== "port" ? `:${port}` : ""]
    .filter(Boolean)
    .join(" · ");
  const options = targets.map((x) => x.option);
  const current = keyOf(source);
  const known = targets.some((x) => x.option.value === current);

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Cube size={13} weight="bold" />}
      markTone="text-tyba-blue"
      status={
        nodeRun?.status === "running" && runUrl ? (
          <span className="shrink-0 text-[11.5px] text-tyba-green">{t("maestroLive")}</span>
        ) : undefined
      }
      meta={detail}
      details={[
        { label: t("maestroServiceSource"), value: t(`maestroSource_${source.type}`) },
        { label: t("maestroDetailTarget"), value: sourceDetail(source) },
        { label: t("maestroDetailPort"), value: port !== null ? `:${port}` : "" },
        { label: t("maestroDetailUrl"), value: runUrl ?? "" },
      ]}
    >
      <div className="flex shrink-0 items-center gap-1.5 px-3.5 pb-1.5 text-[11px]">
        <div role="radiogroup" aria-label={t("maestroServiceSource")} className="flex gap-1.5">
          {SERVICE_SOURCES.map((type) => (
            <button
              key={type}
              type="button"
              role="radio"
              aria-checked={source.type === type}
              onClick={() => source.type !== type && setSource(emptySource(type))}
              className={`nodrag rounded-[4px] border px-2 leading-[20px] ${
                source.type === type
                  ? "border-tyba-blue bg-tyba-raised text-tyba-text"
                  : "border-tyba-border text-tyba-text-faint hover:text-tyba-text"
              }`}
            >
              {t(`maestroSource_${type}`)}
            </button>
          ))}
        </div>
        <span className="flex-1" />
        <span className="truncate text-tyba-text-muted">{detail}</span>
      </div>
      <div className="flex shrink-0 items-center gap-1.5 border-b border-tyba-border px-3.5 pb-2 text-[11px]">
        <div className="nodrag min-w-0 flex-1">
          <Select
            value={known ? current : ""}
            options={
              options.length
                ? options
                : [{ value: "", label: t(`maestroServiceNone_${source.type}`) }]
            }
            onChange={(v) => {
              const found = targets.find((x) => x.option.value === v);
              if (found) setSource(found.source);
            }}
            className="h-6 w-full text-[11px]"
          />
        </div>
        {source.type === "port" && (
          <input
            aria-label={t("maestroServicePort")}
            placeholder={t("maestroServicePort")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            inputMode="numeric"
            value={source.port > 0 ? String(source.port) : ""}
            onChange={(e) => {
              const n = Number(e.target.value.replace(/\D/g, "").slice(0, 5));
              setSource({ type: "port", sessionId: null, port: n > 0 && n <= 65535 ? n : 0 });
            }}
            className={`${fieldClass} w-16`}
          />
        )}
        <button
          type="button"
          aria-label={t("maestroServiceRefresh")}
          title={t("maestroServiceRefresh")}
          onClick={() => setRefresh((n) => n + 1)}
          className="nodrag shrink-0 rounded-[4px] p-1 text-tyba-text-faint hover:bg-tyba-raised hover:text-tyba-text"
        >
          <ArrowClockwise size={12} />
        </button>
      </div>
      <div
        ref={logBox}
        role="log"
        className="nodrag nowheel min-h-0 flex-1 overflow-y-auto bg-tyba-sunken px-3.5 py-2.5 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted"
      >
        {loadError && <p className="text-tyba-amber">{loadError}</p>}
        {containerId ? (
          <>
            {log.error && <p className="text-tyba-amber">{log.error}</p>}
            {log.lines.length === 0 && !log.error && (
              <p className="text-tyba-text-ghost">{t("maestroServiceNoLog")}</p>
            )}
            {log.lines.map((l) => (
              <div
                key={l.seq}
                className={`whitespace-pre-wrap break-all ${
                  l.level === "error" ? "text-tyba-red" : ""
                }`}
              >
                {l.text}
              </div>
            ))}
          </>
        ) : (
          <p className="text-tyba-text-faint">
            {runUrl ?? t(source.type === "container" ? "maestroServicePick" : "maestroServiceUrlHint")}
          </p>
        )}
      </div>
    </NodeFrame>
  );
}
