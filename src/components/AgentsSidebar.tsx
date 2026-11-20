import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import {
  disambiguators,
  needsYou,
  tokenValues,
} from "../lib/agentsSidebar";
import { agentLabelOf, type AgentRow } from "../lib/agentsBoard";
import { formatElapsed } from "../lib/agentTree";
import { turnElapsedMs } from "../lib/sessionStatus";
import { useNow } from "./ui/loading";
import type { SessionId, SubagentSnapshot } from "../lib/ipc";
import type { SessionPlace } from "../lib/agentsBoard";

interface Props {
  rows: AgentRow[];
  /** Barra lateral aberta. Fechada, sobra o trilho: só os pontos. */
  open: boolean;
  onSelect: (sessionId: SessionId, place: SessionPlace) => void;
  /** Subagentes de cada sessão, para a linha "↳ tipo: o que faz". */
  subagents?: Map<SessionId, SubagentSnapshot>;
}

/**
 * O robô de agente (decisão do dono, 2026-11-15: o do `bemmedical-rpa-platform`,
 * "esse mesmo, não tem problema que foge do Phosphor"), com o piscar de olho.
 * As cores são do desenho, não do tema: é ilustração, como a logo do Claude.
 */
export function RobotIcon({ size = 16 }: { size?: number }) {
  const still =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  const eye = (x: number) => (
    <rect x={x} y="35" width="12" height="16" rx="6" fill="#2A2275">
      {!still && (
        <>
          <animate
            attributeName="height"
            values="16;16;2;16"
            keyTimes="0;0.92;0.95;1"
            dur="3.4s"
            repeatCount="indefinite"
          />
          <animate
            attributeName="y"
            values="35;35;42;35"
            keyTimes="0;0.92;0.95;1"
            dur="3.4s"
            repeatCount="indefinite"
          />
        </>
      )}
    </rect>
  );
  return (
    <svg
      viewBox="4 -8 76 92"
      width={size}
      height={size}
      aria-hidden
      className="shrink-0"
    >
      <rect x="39" y="0" width="6" height="14" rx="3" fill="#A08AF5" />
      <circle cx="42" cy="2" r="8" fill="#A08AF5" />
      <rect x="6" y="14" width="72" height="58" rx="20" fill="#E6E1F7" />
      {eye(23)}
      {eye(49)}
      <rect x="22" y="74" width="40" height="8" rx="4" fill="#574898" />
    </svg>
  );
}

/** Nome do agente como o quadro escreve: "Claude Code", "Codex". */
function agentName(row: AgentRow): string | null {
  const raw = (() => {
    if (row.observed) return agentLabelOf(row.observed.agent);
    const kind = row.session.kind;
    if (kind.type !== "agent") return null;
    if (kind.runner === "claude_code") return "claude";
    if (kind.runner === "codex") return "codex";
    return kind.runner.custom;
  })();
  if (raw === "claude") return "Claude Code";
  if (raw === "codex") return "Codex";
  return raw;
}

/** "há 40s", "há 12 min": só de um instante que o core mandou. */
function ago(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  return `${Math.round(m / 60)} h`;
}

const capitalize = (text: string) =>
  text ? text[0].toUpperCase() + text.slice(1) : text;

/**
 * A frota, sempre visível — o painel de agentes do herdr, na língua do TYBA,
 * no cartão do quadro aprovado (Sidebar, 2026-11-15): estado e tempo em cima,
 * robô + agente · sessão, o que ele faz e os subagentes. Quem precisa de você
 * é o cartão branco com contorno amarelo.
 *
 * Mostra **todos** os agentes, não só quem espera: a visão de frota é o que
 * responde "o que está acontecendo agora". A seção inteira desaparece quando
 * não há agente nenhum.
 */
export function AgentsSidebar({ rows, open, onSelect, subagents }: Props) {
  const { t } = useTranslation();
  // Calculado sobre a lista inteira, e não por linha: saber se uma linha é
  // ambígua exige olhar as outras.
  const marcas = useMemo(() => disambiguators(rows), [rows]);
  // Com algum turno rodando, redesenha a cada segundo para o tempo andar.
  useNow(rows.some((row) => turnElapsedMs(row.session, 0) !== null));
  if (rows.length === 0) return null;
  const now = Date.now();

  return (
    <section className={`flex shrink-0 flex-col ${open ? "gap-1.5 px-0.5" : "gap-0.5"}`}>
      {rows.map((row) => {
        const values = tokenValues(row, t, marcas.get(row.session.id) ?? null);
        const marca = needsYou(row);
        if (!open) {
          /* No trilho fechado sobra o ponto: nome truncado em duas letras
             não identifica ninguém, e a cor já diz o que importa. */
          return (
            <button
              key={row.session.id}
              type="button"
              onClick={() => onSelect(row.session.id, row.place)}
              aria-label={`${values.workspace} — ${values.state_text}`}
              className="flex h-6 shrink-0 items-center justify-center rounded-[6px] hover:bg-tyba-text/[.05]"
            >
              <span
                className={`size-1.5 rounded-full ${row.visual.dotClass}`}
                aria-hidden
              />
            </button>
          );
        }
        const status = row.session.status;
        const turn = row.session.last_turn;
        // Tempo só do que o core mediu: rodando, desde o começo do turno
        // aberto; parado, desde o fim do último turno.
        const running = turnElapsedMs(row.session, now);
        const time =
          running !== null
            ? formatElapsed(running)
            : status.state !== "running" && turn
              ? t("agentAgo", { time: ago(turn.ended_at_ms, now) })
              : null;
        const name = agentName(row);
        const detail = values.detail ?? row.session.status_note ?? null;
        const subs = (subagents?.get(row.session.id)?.subagents ?? []).filter(
          (s) => s.status !== "done",
        );
        return (
          <button
            key={row.session.id}
            type="button"
            onClick={() => onSelect(row.session.id, row.place)}
            aria-label={`${values.workspace} — ${values.state_text}`}
            className={`flex shrink-0 flex-col gap-1 rounded-[10px] px-3 py-2.5 text-left transition-colors ${
              marca
                ? "tyba-pill-on outline outline-[1.5px] -outline-offset-[1.5px] outline-tyba-amber-fill"
                : "bg-tyba-raised/60 shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-tyba-raised/80"
            }`}
          >
            <span
              className={`flex min-w-0 items-center gap-1.5 text-[11.5px] font-semibold ${
                marca ? "text-tyba-amber" : row.visual.textClass
              }`}
            >
              <span
                aria-hidden
                className={`size-[7px] shrink-0 rounded-full ${row.visual.dotClass}`}
              />
              <span className="min-w-0 truncate">
                {marca ? t("agentNeedsYou") : capitalize(values.state_text)}
                {time && ` · ${time}`}
              </span>
            </span>
            <span className="flex min-w-0 items-center gap-1.5 text-[13px] font-semibold text-tyba-text">
              <RobotIcon />
              <span className="min-w-0 truncate">
                {name ? `${name} · ${values.workspace}` : values.workspace}
              </span>
            </span>
            {detail && (
              <span className="min-w-0 truncate font-mono text-[10.5px] leading-[15px] text-tyba-text-muted">
                {detail}
              </span>
            )}
            {subs.slice(0, 2).map((s, i) => (
              <span
                key={s.agent_id ?? i}
                className="min-w-0 truncate border-l border-[var(--tyba-line)] pl-2 font-mono text-[10.5px] leading-[15px] text-tyba-text-muted"
              >
                ↳ {s.agent_type}: {s.description}
              </span>
            ))}
          </button>
        );
      })}
    </section>
  );
}
