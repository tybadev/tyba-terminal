import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, GitBranch, ShieldSlash } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { basename } from "@/lib/utils";
import { formatCombo } from "@/lib/keys";
import {
  boardOrder,
  buildRows,
  groupByWorkspace,
  wantsAttention,
  type AgentRow,
  type SessionPlace,
} from "../lib/agentsBoard";
import type { LayoutState, Session, SessionId } from "../lib/ipc";
import { AgentIcon } from "./icons/AgentIcon";

interface Props {
  sessions: Session[];
  layout: LayoutState;
  activeSessionId: SessionId | null;
  /** Atalho do "ir para o próximo", só para exibir ao lado do botão. */
  nextAttentionCombo: string | null;
  onJump: (sessionId: SessionId, place: SessionPlace) => void;
  onNextAttention: () => void;
}

/** O texto que a linha mostra depois do estado: o que o agente está esperando. */
function detailOf(session: Session): string | null {
  switch (session.status.state) {
    case "awaiting_input":
      return session.status.hint;
    case "idle":
      return session.status.summary;
    case "failed":
      return session.status.reason;
    default:
      return null;
  }
}

function Row({
  row,
  active,
  onJump,
}: {
  row: AgentRow;
  active: boolean;
  onJump: Props["onJump"];
}) {
  const { t } = useTranslation();
  // Na linha observada o detalhe é o binário que a tela sugere: o título é o do
  // shell, e sem isso a linha não diz de que agente está falando.
  const detail = row.observed ? row.observed.agent : detailOf(row.session);
  const branch = row.session.worktree?.branch ?? null;
  const repo = row.session.repo_root ? basename(row.session.repo_root) : null;

  return (
    <button
      type="button"
      onClick={() => onJump(row.session.id, row.place)}
      className={`group flex w-full items-center gap-3 rounded-md px-3 py-2 text-left transition-colors hover:bg-tyba-text/[.05] ${
        active ? "bg-tyba-text/[.08]" : ""
      }`}
    >
      <span
        className={`size-2 shrink-0 rounded-full ${row.visual.dotClass}`}
        aria-hidden
      />
      <AgentIcon size={14} className="shrink-0 text-tyba-text-muted" />

      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="truncate text-sm text-tyba-text">
            {row.session.title}
          </span>
          <span className={`shrink-0 text-xs ${row.visual.textClass}`}>
            {t(row.visual.labelKey)}
          </span>
          {row.observed && (
            // Mesma linguagem do aviso no terminal e do painel de subagentes:
            // escudo cortado, âmbar. O selo repete no cabeçalho da seção porque
            // a rolagem tira o cabeçalho da tela e a linha continua valendo.
            <span
              className="flex shrink-0 items-center gap-1 rounded-[3px] border border-tyba-amber/40 px-1 text-[10px] text-tyba-amber"
              title={t("agentsNoGate")}
            >
              <ShieldSlash size={10} weight="fill" aria-hidden />
              {t("agentsBoardNoGate")}
            </span>
          )}
        </span>
        {detail && (
          <span className="mt-0.5 block truncate text-xs text-tyba-text-muted">
            {detail}
          </span>
        )}
      </span>

      {branch && (
        <span className="flex shrink-0 items-center gap-1 text-xs text-tyba-text-faint">
          <GitBranch size={12} />
          <span className="max-w-40 truncate">{branch}</span>
        </span>
      )}
      {!branch && repo && (
        <span className="shrink-0 truncate text-xs text-tyba-text-faint">
          {repo}
        </span>
      )}

      <ArrowRight
        size={14}
        className="shrink-0 text-tyba-text-faint opacity-0 transition-opacity group-hover:opacity-100"
        aria-hidden
      />
    </button>
  );
}

/**
 * Todos os agentes, de todos os workspaces, num lugar só.
 *
 * O componente é burro de propósito: ele não sabe o que é urgência nem o que é
 * rollup — isso mora em `lib/agentsBoard.ts`, que é testado. Aqui só entra
 * pintura e o clique que devolve a intenção de saltar.
 */
export function AgentsBoard({
  sessions,
  layout,
  activeSessionId,
  nextAttentionCombo,
  onJump,
  onNextAttention,
}: Props) {
  const { t } = useTranslation();
  const sections = useMemo(
    () => buildRows(sessions, layout),
    [sessions, layout],
  );
  const groups = useMemo(
    () => groupByWorkspace(sections.managed),
    [sections.managed],
  );
  const waiting = useMemo(
    () => boardOrder(sections).filter(wantsAttention).length,
    [sections],
  );
  const empty = groups.length === 0 && sections.observed.length === 0;

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <header className="flex items-center gap-3 border-b border-tyba-border px-4 py-3">
        <h2 className="text-sm font-medium text-tyba-text">
          {t("agentsBoard")}
        </h2>
        <span className="text-xs text-tyba-text-muted">
          {waiting > 0
            ? t("agentsBoardWaiting", { count: waiting })
            : t("agentsBoardAllQuiet")}
        </span>
        <span className="flex-1" />
        {waiting > 0 && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onNextAttention}
            className="gap-2 text-xs"
          >
            {t("agentsBoardGoToNext")}
            {nextAttentionCombo && (
              <kbd className="rounded border border-tyba-border px-1 text-tyba-text-faint">
                {formatCombo(nextAttentionCombo)}
              </kbd>
            )}
          </Button>
        )}
      </header>

      {empty ? (
        <p className="px-4 py-6 text-sm text-tyba-text-muted">
          {t("agentsBoardEmpty")}
        </p>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
          {groups.map((group) => (
            <section key={group.workspaceId} className="mb-3">
              <h3 className="flex items-center gap-2 px-3 py-1 text-xs text-tyba-text-muted">
                {group.workspaceColor && (
                  <span
                    className="size-2 rounded-full"
                    style={{ backgroundColor: group.workspaceColor }}
                    aria-hidden
                  />
                )}
                <span className="truncate">{group.workspaceName}</span>
                <span className="text-tyba-text-faint">
                  {group.rows.length}
                </span>
              </h3>
              {group.rows.map((row) => (
                <Row
                  key={row.session.id}
                  row={row}
                  active={row.session.id === activeSessionId}
                  onJump={onJump}
                />
              ))}
            </section>
          ))}

          {/* Mesmo quadro, seção própria e sempre embaixo: o que o TYBA lançou
              vem primeiro, e o que ele apenas deduziu da tela vem depois, com o
              que a seção não tem dito no cabeçalho. */}
          {sections.observed.length > 0 && (
            <section className="mb-3 border-t border-tyba-border pt-2">
              <h3 className="flex items-center gap-2 px-3 py-1 text-xs text-tyba-text-muted">
                <ShieldSlash
                  size={12}
                  weight="fill"
                  className="shrink-0 text-tyba-amber"
                  aria-hidden
                />
                <span className="truncate">{t("agentsBoardObserved")}</span>
                <span className="text-tyba-text-faint">
                  {sections.observed.length}
                </span>
              </h3>
              <p className="px-3 pb-1 text-[11px] leading-snug text-tyba-text-faint">
                {t("agentsNoGate")}
              </p>
              {sections.observed.map((row) => (
                <Row
                  key={row.session.id}
                  row={row}
                  active={row.session.id === activeSessionId}
                  onJump={onJump}
                />
              ))}
            </section>
          )}
        </div>
      )}
    </div>
  );
}
