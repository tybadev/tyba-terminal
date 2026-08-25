import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ShieldSlash } from "@phosphor-icons/react";

import {
  DEFAULT_AGENT_ROWS,
  disambiguators,
  needsYou,
  tokenValues,
  visibleRows,
  type AgentToken,
  type AgentTokenValues,
} from "../lib/agentsSidebar";
import type { AgentRow } from "../lib/agentsBoard";
import type { SessionId } from "../lib/ipc";
import type { SessionPlace } from "../lib/agentsBoard";

interface Props {
  rows: AgentRow[];
  /** Barra lateral aberta. Fechada, sobra o trilho: só os pontos. */
  open: boolean;
  onSelect: (sessionId: SessionId, place: SessionPlace) => void;
}

function Token({
  token,
  values,
  row,
}: {
  token: AgentToken;
  values: AgentTokenValues;
  row: AgentRow;
}) {
  switch (token) {
    case "state_icon":
      return (
        <span
          className={`size-1.5 shrink-0 rounded-full ${row.visual.dotClass}`}
          aria-hidden
        />
      );
    case "no_gate":
      return values.no_gate ? (
        <ShieldSlash size={10} className="shrink-0 text-tyba-amber" />
      ) : null;
    case "workspace":
      return (
        <span className="min-w-0 truncate text-[12px] text-tyba-text">
          {values.workspace}
        </span>
      );
    case "state_text":
      return (
        <span className={`shrink-0 text-[10px] ${row.visual.textClass}`}>
          {values.state_text}
        </span>
      );
    case "agent":
      return values.agent ? (
        <span className="shrink-0 font-mono text-[10px] text-tyba-text-faint">
          {values.agent}
        </span>
      ) : null;
    case "detail":
      return values.detail ? (
        <span className="min-w-0 truncate text-[10px] text-tyba-text-muted">
          {values.detail}
        </span>
      ) : null;
  }
}

/**
 * A frota, sempre visível — o painel de agentes do herdr, na língua do TYBA.
 *
 * Mostra **todos** os agentes, não só quem espera: a visão de frota é o que
 * responde "o que está acontecendo agora", e esconder quem trabalha faz a
 * seção piscar de existência a cada turno. Quem precisa de você leva **marca**,
 * que é o que separa informação de pedido.
 *
 * O nome do workspace aparece aqui e na lista de espaços acima, e isso não é
 * repetição: lá ele vem com branch e git, aqui com o agente e o estado. É
 * coordenada, não conteúdo — mesma razão pela qual um caminho de arquivo
 * aparece na árvore e no resultado da busca.
 *
 * A seção inteira desaparece quando não há agente nenhum: espaço permanente
 * para lista vazia é o que faz barra lateral virar depósito.
 */
export function AgentsSidebar({ rows, open, onSelect }: Props) {
  const { t } = useTranslation();
  // Calculado sobre a lista inteira, e não por linha: saber se uma linha é
  // ambígua exige olhar as outras.
  const marcas = useMemo(() => disambiguators(rows), [rows]);
  if (rows.length === 0) return null;

  return (
    /* Sem cabeçalho próprio: o botão "Agentes" logo acima é o título desta
       seção, e ele já carrega o contador de quem espera. Repetir a palavra a
       oito pixels de distância seria ruído. */
    <section className="flex shrink-0 flex-col">
      {rows.map((row) => {
        const values = tokenValues(row, t, marcas.get(row.session.id) ?? null);
        const marca = needsYou(row);
        return (
          <button
            key={row.session.id}
            type="button"
            onClick={() => onSelect(row.session.id, row.place)}
            aria-label={`${values.workspace} — ${values.state_text}`}
            className={`group relative flex shrink-0 flex-col gap-0.5 rounded-[4px] py-1 text-left transition-colors hover:bg-tyba-text/[.05] ${
              open ? "px-2" : "items-center px-0"
            } ${marca ? "bg-tyba-amber/[.05]" : ""}`}
          >
            {/* A marca. Mesma barra âmbar do painel, de propósito: as duas
                superfícies falam do mesmo estado e não podem usar sinais
                diferentes para ele. */}
            {marca && (
              <span
                className="absolute inset-y-0.5 left-0 w-0.5 rounded-full bg-tyba-amber"
                aria-hidden
              />
            )}
            {open ? (
              visibleRows(DEFAULT_AGENT_ROWS, values).map((tokens, i) => (
                <span
                  key={i}
                  className="flex min-w-0 items-center gap-1.5 leading-none"
                >
                  {tokens.map((token) => (
                    <Token
                      key={token}
                      token={token}
                      values={values}
                      row={row}
                    />
                  ))}
                </span>
              ))
            ) : (
              /* No trilho fechado sobra o ponto: nome truncado em duas letras
                 não identifica ninguém, e a cor já diz o que importa. */
              <span
                className={`size-1.5 rounded-full ${row.visual.dotClass}`}
                aria-hidden
              />
            )}
          </button>
        );
      })}
    </section>
  );
}
