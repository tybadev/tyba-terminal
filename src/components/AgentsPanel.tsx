import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsOutSimple, CaretDown, CaretRight } from "@phosphor-icons/react";

import {
  resolveApproval,
  type ApprovalDecision,
  type ApprovalRequest,
  type Session,
  type SubagentSnapshot,
} from "@/lib/ipc";
import {
  orchestratorVisual,
  subagentInlineApprovals,
  subagentLine,
} from "@/lib/agentsPanel";
import { formatDuration } from "@/lib/blockLayout";
import { usageSummary } from "@/lib/agentUsage";
import { decideApproval } from "@/lib/notifications";
import { withSubagentsReason } from "@/lib/sessionStatus";
import {
  LIGHT_CLASS,
  buildAgentTree,
  collapsedRows,
  formatElapsed,
  groupByRound,
  roundAge,
  type AgentLight,
  type AgentRound,
  type AgentTreeRow,
} from "@/lib/agentTree";

interface Props {
  session: Session;
  snapshot: SubagentSnapshot | null;
  approvals: ApprovalRequest[];
  /** Nome do orquestrador no topo da cascata: em sessão de shell é o binário
   * detectado (`claude`), não o título da sessão (que seria o shell). */
  orchestratorTitle?: string | null;
  /** O subagente com o detalhe aberto na lateral (4E): a linha fica marcada. */
  selectedId: string | null;
  /** Clicar na linha abre, troca ou (na marcada) fecha o detalhe. */
  onSelect: (agentId: string) => void;
}

const LIGHT_LABEL: Record<AgentLight, string> = {
  running: "subagentsStatusRunning",
  starting: "subagentsStatusStarting",
  awaiting: "agentsAwaitingApproval",
  done: "subagentsStatusDone",
  failed: "agentsInterrupted",
  blocked: "subagentsStatusBlocked",
};

/**
 * O robô do agente (decisão do dono, 2026-11-15: o mesmo do quadro, fora do
 * Phosphor), com o piscar de olho. As cores saem dos tokens: antena e boca no
 * violeta, corpo na tinta dele, olhos na cor do texto. Piscar só sem
 * "reduzir movimento".
 */
export function RobotIcon({ size = 15, className }: { size?: number; className?: string }) {
  const { t } = useTranslation();
  const eyes = useRef<(SVGRectElement | null)[]>([]);
  useEffect(() => {
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    const running = eyes.current.map((el) =>
      el?.animate?.(
        [
          { transform: "scaleY(1)", offset: 0 },
          { transform: "scaleY(1)", offset: 0.92 },
          { transform: "scaleY(0.1)", offset: 0.95 },
          { transform: "scaleY(1)", offset: 1 },
        ],
        { duration: 3400, iterations: Infinity },
      ),
    );
    return () => running.forEach((a) => a?.cancel());
  }, []);
  // Lilás dos quadros no claro (`--tyba-robot-*`); nos outros temas, o
  // violeta e a tinta do tema, como antes.
  const accent = { fill: "var(--tyba-robot-accent, var(--tyba-violet))" };
  const eye = { fill: "var(--tyba-robot-eye, var(--tyba-text))", transformBox: "fill-box", transformOrigin: "center" } as const;
  return (
    <svg
      viewBox="4 -8 76 92"
      width={size}
      height={size}
      role="img"
      aria-label={t("agentsTitle")}
      className={`shrink-0 ${className ?? ""}`}
    >
      <rect x="39" y="0" width="6" height="14" rx="3" style={accent} />
      <circle cx="42" cy="2" r="8" style={accent} />
      <rect
        x="6"
        y="14"
        width="72"
        height="58"
        rx="20"
        style={{ fill: "var(--tyba-robot-body, color-mix(in srgb, var(--tyba-violet) 22%, var(--tyba-paper-solid)))" }}
      />
      <rect ref={(el) => void (eyes.current[0] = el)} x="23" y="35" width="12" height="16" rx="6" style={eye} />
      <rect ref={(el) => void (eyes.current[1] = el)} x="49" y="35" width="12" height="16" rx="6" style={eye} />
      <rect x="22" y="74" width="40" height="8" rx="4" style={{ fill: "var(--tyba-robot-mouth, var(--tyba-violet))" }} />
    </svg>
  );
}

/** Estado "em andamento" do agente principal: no quadro, azul. */
const IN_PROGRESS = new Set(["sessionInProgress", "sessionSubagentsWorking"]);

function PendingBadge({ count }: { count: number }) {
  const { t } = useTranslation();
  if (count === 0) return null;
  return (
    <span
      className="shrink-0 rounded-full bg-tyba-amber-tint px-1.5 font-mono text-[10px] leading-4 text-tyba-amber"
      title={t("agentsPendingApprovals", { count })}
    >
      {count}
    </span>
  );
}

/**
 * A aba Agentes da gaveta (quadros 4B′/4C): resumo da sessão e a árvore
 * agente principal → subagentes, com luz de status por nível. O detalhe do
 * subagente abre na lateral (4E), não aqui.
 */
export function AgentsPanel({
  session,
  snapshot,
  approvals,
  orchestratorTitle,
  selectedId,
  onSelect,
}: Props) {
  const { t, i18n } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const subagents = useMemo(() => snapshot?.subagents ?? [], [snapshot]);
  // O pedido que cada linha mostra — a mesma derivação que o `App` usa para
  // tirar do toast o que este painel deixa aprovar.
  const inline = useMemo(
    () => subagentInlineApprovals(session.id, subagents, approvals),
    [session.id, subagents, approvals],
  );
  const tree = useMemo(
    () => buildAgentTree(session.id, subagents, approvals, now),
    [session.id, subagents, approvals, now],
  );

  // Rodando: relógio por segundo. Parado: devagar, só para o "há N min" dos
  // cabeçalhos de rodada não envelhecer errado.
  // O tempo de sessão do resumo também anda — por segundo enquanto há
  // subagente rodando, que é quando alguém olha para ele.
  const ticking = tree.active > 0;
  useEffect(() => {
    const timer = window.setInterval(
      () => setNow(Date.now()),
      ticking ? 1000 : 30_000,
    );
    return () => window.clearInterval(timer);
  }, [ticking]);

  const rounds = useMemo(() => groupByRound(tree.rows), [tree.rows]);
  // Só o que o usuário mexeu; o resto segue o padrão (a mais nova aberta).
  const [roundOpen, setRoundOpen] = useState<Record<number, boolean>>({});

  const visual = withSubagentsReason(
    orchestratorVisual(session.status, session.attention, subagents),
    session.subagents_working === true,
  );
  return (
    <div className="flex min-h-0 min-w-0 flex-col">
      {/* Resumo da sessão: quem, onde, em que estado; embaixo, o que o core
          sabe dela. Com o `statusLine` do Claude: modelo, contexto, tokens,
          custo e tempo. Sem ele (Codex, Claude sem sessão do TYBA): só o
          tempo de sessão — o que não chega do core não se inventa. */}
      <div className="mx-2.5 flex shrink-0 flex-col px-0.5 pb-3 pt-1.5 shadow-[inset_0_-1px_0_var(--tyba-line)]">
        {/* Quadro Agentes (g5): robô, o nome do agente em negrito e o estado
            à direita; embaixo, a linha de uso. */}
        <div className="flex min-w-0 items-center gap-2 text-[14px]">
          <RobotIcon size={18} />
          <span className="min-w-0 truncate font-semibold text-tyba-text">
            {orchestratorTitle ?? session.title}
          </span>
          <PendingBadge count={tree.mainPending} />
          {visual && (
            <span
              className={`ml-auto flex shrink-0 items-center gap-1.5 text-[12px] font-semibold ${
                IN_PROGRESS.has(visual.labelKey) ? "text-tyba-blue" : visual.textClass
              }`}
            >
              <span
                className={`size-[7px] rounded-full ${
                  IN_PROGRESS.has(visual.labelKey) ? "bg-tyba-blue" : visual.dotClass
                }`}
              />
              {t(visual.labelKey)}
            </span>
          )}
        </div>
        {(() => {
          const usage = usageSummary(session.agent_usage, i18n.language, t);
          if (usage) {
            return (
              <span
                className="mt-1.5 truncate pl-[26px] font-mono text-[11.5px] text-tyba-text-muted"
                title={usage}
              >
                {usage}
              </span>
            );
          }
          const created = Date.parse(session.created_at);
          const took = Number.isFinite(created)
            ? formatDuration(Math.max(0, now - created), i18n.language, {
                whole: true,
              })
            : null;
          return took ? (
            <span className="mt-1.5 pl-[26px] font-mono text-[11.5px] text-tyba-text-muted">
              {t("agentsSessionElapsed", { time: took })}
            </span>
          ) : null;
        })()}
      </div>

      <div className="max-h-[45vh] min-h-0 overflow-y-auto overflow-x-hidden px-3 pb-2.5 pt-1">
        {rounds.length === 0 ? (
          <p className="pt-2 text-[12px] leading-relaxed text-tyba-text-faint">
            {t("agentsEmpty")}
          </p>
        ) : (
          <div className="flex flex-col gap-3">
            {rounds.map((round) => {
              const expanded = roundOpen[round.round] ?? round.latest;
              const visible = expanded ? round.rows : collapsedRows(round);
              return (
                <section key={round.round} className="min-w-0">
                  {rounds.length > 1 && (
                    <RoundHeader
                      round={round}
                      now={now}
                      expanded={expanded}
                      onToggle={() =>
                        setRoundOpen((prev) => ({
                          ...prev,
                          [round.round]: !expanded,
                        }))
                      }
                    />
                  )}
                  <ul className="flex min-w-0 flex-col">
                    {visible.map((row) => (
                      <SubagentRow
                        key={row.key}
                        row={row}
                        approval={
                          row.run.agent_id
                            ? (inline.get(row.run.agent_id) ?? null)
                            : null
                        }
                        selected={row.run.agent_id != null && row.run.agent_id === selectedId}
                        onSelect={onSelect}
                      />
                    ))}
                  </ul>
                </section>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function RoundHeader({
  round,
  now,
  expanded,
  onToggle,
}: {
  round: AgentRound;
  now: number;
  expanded: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const age = roundAge(round.startedAtMs, now);
  const ageText =
    age.unit === "now"
      ? t("agentsRoundNow")
      : t(age.unit === "min" ? "agentsRoundMinutes" : "agentsRoundHours", {
          count: age.count,
        });
  const running = round.counts.running + round.counts.starting;
  const parts = [
    running > 0 && t("agentsCountRunning", { count: running }),
    round.counts.awaiting > 0 &&
      t("agentsCountAwaiting", { count: round.counts.awaiting }),
    round.counts.done > 0 && t("agentsCountDone", { count: round.counts.done }),
    round.counts.failed > 0 &&
      t("agentsCountFailed", { count: round.counts.failed }),
    round.counts.blocked > 0 &&
      t("agentsCountBlocked", { count: round.counts.blocked }),
  ].filter(Boolean);
  const Caret = expanded ? CaretDown : CaretRight;
  return (
    <button
      type="button"
      aria-expanded={expanded}
      onClick={onToggle}
      // Mockup Proposta: "Rodada 2, agora", 11px apagado, sem caixa alta.
      className="flex w-full min-w-0 items-center gap-1.5 py-1 text-left text-[11px] text-tyba-text-faint hover:text-tyba-text-muted"
    >
      <Caret size={9} className="shrink-0" />
      <span className="shrink-0">
        {t("agentsRound", { n: round.round })}, {ageText}
      </span>
      <span className="min-w-0 flex-1 truncate text-right">
        {parts.join(" · ")}
      </span>
    </button>
  );
}

function SubagentRow({
  row,
  approval,
  selected,
  onSelect,
}: {
  row: AgentTreeRow;
  /** O pedido pendente mais antigo deste subagente, se houver. */
  approval: ApprovalRequest | null;
  selected: boolean;
  onSelect: (agentId: string) => void;
}) {
  const { t } = useTranslation();
  const id = row.run.agent_id;
  const line = subagentLine(row.run);
  const awaiting = row.light === "awaiting" && approval !== null;
  return (
    <li
      // Quadro Agentes (g5-sub): linha solta; marcada = a placa do item ativo
      // da lateral; pedindo você = a mesma placa com o contorno amarelo.
      className={`min-w-0 rounded-[12px] pt-3 pb-0.5 ${
        awaiting
          ? "tyba-pill-on -mx-2 mt-1 px-2 pb-2 shadow-[var(--tyba-pill-on),inset_0_0_0_1.5px_var(--tyba-amber-fill)]!"
          : selected
            ? "tyba-pill-on -mx-2 mt-1 px-2 pb-2"
            : ""
      }`}
    >
      <button
        type="button"
        disabled={id == null}
        aria-pressed={selected}
        onClick={() => id && onSelect(id)}
        className={`group flex w-full min-w-0 flex-col gap-0.5 rounded-[5px] text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-tyba-green ${
          id ? "" : "cursor-default opacity-70"
        }`}
      >
        <span className="flex w-full min-w-0 items-center gap-2.5 text-[13.5px] font-semibold">
          <span
            className={`size-[7px] shrink-0 rounded-full ${LIGHT_CLASS[row.light]}`}
            role="img"
            aria-label={t(LIGHT_LABEL[row.light])}
          />
          <span
            className={`min-w-0 flex-1 truncate ${
              row.light === "done" ||
              row.light === "failed" ||
              row.light === "blocked"
                ? "text-tyba-text-muted"
                : "text-tyba-text"
            } ${id ? "group-hover:text-tyba-text" : ""}`}
          >
            {row.run.description || row.run.agent_type || t("agentsUnknownType")}
          </span>
          <PendingBadge count={awaiting ? row.pending - 1 : row.pending} />
          <span className="shrink-0 font-mono text-[11.5px] font-medium tabular-nums text-tyba-text-faint">
            {formatElapsed(row.elapsedMs)}
          </span>
          {id &&
            (selected ? (
              <ArrowsOutSimple size={11} className="shrink-0 text-tyba-text-faint" />
            ) : (
              <CaretRight size={11} className="shrink-0 text-tyba-text-faint" />
            ))}
        </span>
        {!awaiting && line && line.kind !== "run" && line.kind !== "tool" && (
          <span className="pl-[17px]">
            <SubagentLineText line={line} />
          </span>
        )}
      </button>
      {/* O que está executando AGORA, inteiro (mockup Subagentes): a linha
          cortada não deixava ver o comando. */}
      {line && (line.kind === "run" || line.kind === "tool") && (
        <NowBox
          line={line}
          compact={!selected}
          recent={selected ? (row.run.recent_activity ?? []) : []}
        />
      )}
      {awaiting && approval && <InlineApproval approval={approval} />}
    </li>
  );
}

/** A ação em curso, inteira: ferramenta + comando ou caminho, em mono. */
export function NowBox({
  line,
  compact = false,
  recent = [],
}: {
  line: NonNullable<ReturnType<typeof subagentLine>>;
  compact?: boolean;
  /** As ações anteriores, mais nova primeiro. */
  recent?: { tool: string; detail: string | null; at_ms: number }[];
}) {
  const { t } = useTranslation();
  if (line.kind !== "run" && line.kind !== "tool") return null;
  const tool = line.kind === "run" ? "Bash" : line.tool;
  const detail = line.detail;
  return (
    <>
    <div
      title={compact ? `${tool} ${detail ?? ""}` : undefined}
      // Quadro Agentes (g5-now): caixa clara com fio, AGORA em verde, a
      // ferramenta em azul e o comando numa linha (inteiro no hover e no
      // detalhe aberto).
      className={`mt-2 mb-1 ml-[17px] flex min-w-0 gap-2.5 rounded-[10px] bg-tyba-paper-solid px-3 py-2 font-mono text-[12px] shadow-[inset_0_0_0_1px_var(--tyba-line),0_1px_2px_color-mix(in_srgb,var(--tyba-text)_4%,transparent)] ${
        compact ? "items-center whitespace-nowrap" : "items-start"
      }`}
    >
      <span className="shrink-0 pt-px text-[10px] font-semibold tracking-[0.08em] text-tyba-green">
        {t("agentsNow")}
      </span>
      <span className="shrink-0 text-tyba-blue">{tool}</span>
      {detail && (
        <span
          className={`min-w-0 flex-1 leading-[17px] text-tyba-text ${
            compact ? "truncate" : "break-all"
          }`}
        >
          {detail}
        </span>
      )}
    </div>
      {recent.length > 0 && (
        <ul className="mb-1.5 ml-[17px] flex min-w-0 flex-col gap-0.5">
          {recent.map((r) => (
            <li key={`${r.at_ms}-${r.tool}`} className="flex min-w-0 gap-2 text-[11px] text-tyba-text-faint">
              <span className="w-[48px] shrink-0 tabular-nums">{t("agentsAgo", { time: formatElapsed(Date.now() - r.at_ms) })}</span>
              <span className="shrink-0 font-mono">{r.tool}</span>
              {r.detail && (
                <span className="min-w-0 truncate font-mono text-tyba-text-muted" title={r.detail}>
                  {r.detail}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** O que o subagente faz agora — ou, terminado, o resultado em uma linha. */
function SubagentLineText({ line }: { line: NonNullable<ReturnType<typeof subagentLine>> }) {
  const { t } = useTranslation();
  const base = "block min-w-0 truncate text-[12px]";
  if (line.kind === "run") {
    return (
      <span className={`${base} text-tyba-text-muted`} title={line.detail}>
        {t("agentsDoingRun")}{" "}
        <span className="font-mono text-tyba-text">{line.detail}</span>
      </span>
    );
  }
  if (line.kind === "tool") {
    return (
      <span className={`${base} text-tyba-text-muted`} title={line.detail ?? undefined}>
        {line.tool}
        {line.detail && (
          <>
            {" "}
            <span className="font-mono text-tyba-text">{line.detail}</span>
          </>
        )}
      </span>
    );
  }
  if (line.kind === "interrupted") {
    return (
      <span className={`${base} text-tyba-red`}>{t("agentsInterrupted")}</span>
    );
  }
  return (
    <span className={`${base} text-tyba-text-faint`} title={line.text}>
      {line.text}
    </span>
  );
}

/**
 * O pedido do subagente, com Aprovar/Recusar ali mesmo.
 *
 * Mesmo caminho de decisão do toast e da fila (`decideApproval` →
 * `resolveApproval`). Vermelho não ganha botão aqui: continua com o painel
 * dentro da sessão, que é onde a ação vermelha se decide (princípio 4) — o
 * `App` também não o tira do toast.
 */
function InlineApproval({ approval }: { approval: ApprovalRequest }) {
  const { t } = useTranslation();
  const red = approval.risk === "red";
  const decide = (decision: ApprovalDecision) => {
    const effect = decideApproval({
      request: approval,
      decision,
      confirmingId: null,
    });
    // Não-vermelho resolve direto; o braço de confirmação nunca vem daqui.
    if (effect.type !== "resolve") return;
    resolveApproval(effect.requestId, effect.decision, effect.feedback).catch(
      () => {},
    );
  };
  return (
    <div className="flex min-w-0 flex-col gap-1.5 pb-1 pl-[17px]">
      <p className="truncate text-[12px] text-tyba-amber" title={approval.command}>
        {t("agentsWantsToRun")}{" "}
        <span className="font-mono text-tyba-text">{approval.command}</span>
      </p>
      {red ? (
        <p className="text-[11px] text-tyba-red">{t("agentsRedInSession")}</p>
      ) : (
        <div className="flex gap-2 pt-0.5">
          <button
            type="button"
            onClick={() => decide("approved")}
            className="h-7 rounded-[6px] bg-tyba-green px-3 text-[12px] text-tyba-text-inverse hover:bg-[var(--tyba-primary-hover)] focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)]"
          >
            {t("approve")}
          </button>
          <button
            type="button"
            onClick={() => decide("denied")}
            className="h-7 rounded-[6px] border border-tyba-border-strong px-3 text-[12px] text-tyba-text-muted hover:text-tyba-text focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)]"
          >
            {t("deny")}
          </button>
        </div>
      )}
    </div>
  );
}
