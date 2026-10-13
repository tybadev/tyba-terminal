import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { PANEL_SECTION, SidePanelHeader } from "./SidePanelBar";
import { useRelative } from "./GitHistoryView";
import { translateError } from "../lib/errors";
import { requestConfirm } from "../lib/confirm";
import {
  commitPlan,
  gitChangeRows,
  pruneChosen,
  toggleUntracked,
  upstreamLine,
  type GitChangeRow,
} from "../lib/gitPanel";
import {
  onRepoChanged,
  repoLog,
  sessionDiff,
  worktreeCommit,
  worktreeStage,
  type LogRow,
  type RepoSnapshot,
  type Session,
  type SessionDiff,
} from "../lib/ipc";

const TONE: Record<string, string> = {
  green: "text-tyba-green",
  amber: "text-tyba-amber",
  red: "text-tyba-red",
  muted: "text-tyba-text-faint",
};

function RowName({ row }: { row: GitChangeRow }) {
  const { t } = useTranslation();
  return (
    <span className="flex min-w-0 items-center gap-2">
      <span className={`shrink-0 ${TONE[row.tone]}`}>{row.letter}</span>
      <span className="truncate text-tyba-text" title={row.path}>
        {row.path}
      </span>
      {row.sensitive && (
        <span className="shrink-0 rounded-[4px] bg-tyba-red/[.12] px-1.5 font-sans text-[10px] text-tyba-red">
          {t("gitPanelLooksSecret")}
        </span>
      )}
    </span>
  );
}

function Counts({ row }: { row: GitChangeRow }) {
  return (
    <span className="shrink-0 text-[11px]">
      {row.insertions > 0 && <span className="text-tyba-green">+{row.insertions}</span>}
      {row.insertions > 0 && row.deletions > 0 && " "}
      {row.deletions > 0 && <span className="text-tyba-red">−{row.deletions}</span>}
    </span>
  );
}

/**
 * Painel Git (mockup PainelGit): o resumo para agir. Branch e à frente/atrás
 * vêm do snapshot do repo que o core já publica; a lista de alterações e o
 * commit usam os mesmos comandos do Diff. Revisar linha a linha é no Diff.
 */
export function GitPanel({
  session,
  branch,
  snapshot,
  onOpenDiff,
  onOpenHistory,
  onClose,
}: {
  session: Session;
  branch: string | null;
  snapshot: RepoSnapshot | undefined;
  onOpenDiff: () => void;
  onOpenHistory: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [diff, setDiff] = useState<SessionDiff | null>(null);
  // O último commit é o do HEAD, não o da sessão: `diff.commits` só tem o que
  // veio depois da base e fica vazio num repo em dia.
  const [head, setHead] = useState<LogRow | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    sessionDiff(session.id)
      .then((d) => {
        setDiff(d);
        setLoadError(null);
      })
      .catch((e) => setLoadError(translateError(e, t)));
    repoLog(session.id, {}, null, 1)
      .then((page) => setHead(page.rows[0] ?? null))
      .catch(() => setHead(null));
  }, [session.id, t]);

  // Mesma cadência do Diff: evento do core para index/HEAD, poll leve para
  // edição de arquivo (que o watcher do git dir não enxerga).
  useEffect(() => {
    refresh();
    const timer = window.setInterval(refresh, 5000);
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onRepoChanged(() => refresh()).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      unlisten?.();
    };
  }, [refresh]);

  const { rows, untracked, insertions, deletions } = useMemo(
    () => gitChangeRows(diff),
    [diff],
  );
  // Untracked começa desmarcado: arquivo novo só entra no commit marcado.
  const [chosen, setChosen] = useState<Set<string>>(() => new Set());
  useEffect(() => {
    setChosen((prev) => pruneChosen(prev, diff));
  }, [diff]);
  const plan = useMemo(() => commitPlan(diff, chosen), [diff, chosen]);

  const toggle = async (path: string) => {
    const first = toggleUntracked(chosen, path, false);
    if (!first.needsConfirm) {
      setChosen(first.chosen);
      return;
    }
    const ok = await requestConfirm({
      title: t("gitPanelSecretConfirmTitle", { path }),
      detail: t("gitPanelSecretConfirmDetail"),
      confirmLabel: t("gitPanelSecretConfirmAction"),
      destructive: true,
    });
    if (ok) setChosen((prev) => toggleUntracked(prev, path, true).chosen);
  };
  const upstream = upstreamLine(snapshot?.ahead, snapshot?.behind);
  const shownBranch = snapshot?.branch ?? branch;
  const relative = useRelative();
  const upstreamName = snapshot?.upstream ?? "@{upstream}";

  const commit = async () => {
    if (busy || !message.trim() || plan.mode === "nothing") return;
    setBusy(true);
    setActionError(null);
    try {
      if (plan.stage.length > 0) await worktreeStage(session.id, plan.stage);
      await worktreeCommit(session.id, message);
      setMessage("");
      refresh();
    } catch (e) {
      setActionError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <SidePanelHeader kind="git" title={t("sidePanelGit")} onClose={onClose} />

      {loadError && !diff ? (
        <p className="px-4 py-3 text-[12px] text-tyba-text-faint">
          {t("gitPanelNoRepo")}
        </p>
      ) : (
        <>
          <div className={`${PANEL_SECTION} flex flex-col gap-1.5 px-4 py-3.5`}>
            <div className="flex items-center justify-between gap-3">
              <span className="min-w-0 truncate text-[13px] text-tyba-blue">
                {shownBranch ?? "HEAD"}
              </span>
              <span className="shrink-0 text-[11px] text-tyba-text-muted">
                {t(upstream.key, {
                  ahead: upstream.ahead,
                  behind: upstream.behind,
                  upstream: upstreamName,
                })}
              </span>
            </div>
            {head && (
              <div className="truncate text-[11px] text-tyba-text-faint" title={head.subject}>
                {t("gitPanelLastCommit", {
                  ago: relative(head.authored_at, Date.now()),
                  subject: head.subject,
                })}
              </div>
            )}
          </div>

          <div className="flex min-h-0 flex-1 flex-col gap-2.5 px-4 py-3">
            <div className="flex items-center justify-between text-[12px] text-tyba-text-muted">
              <span>
                {rows.length + untracked.length === 0
                  ? t("gitPanelClean")
                  : t("gitPanelChanges", { count: rows.length + untracked.length })}
              </span>
              {rows.length + untracked.length > 0 && (
                <span className="font-mono">
                  <span className="text-tyba-green">+{insertions}</span>{" "}
                  <span className="text-tyba-red">−{deletions}</span>
                </span>
              )}
            </div>

            <div className="-mx-2 min-h-0 flex-1 overflow-y-auto">
              <ul>
                {rows.map((row) => (
                  <li
                    key={row.path}
                    className="flex items-center justify-between gap-3 rounded-[6px] px-2 py-1.5 text-[13px] hover:bg-tyba-raised hover:shadow-[var(--tyba-row-raised)]"
                  >
                    <RowName row={row} />
                    <Counts row={row} />
                  </li>
                ))}
              </ul>
              {untracked.length > 0 && (
                <>
                  <div className="px-2 pb-1 pt-3 text-[11px] text-tyba-text-faint">
                    {t("gitPanelUntracked", { count: untracked.length })}
                  </div>
                  <ul>
                    {untracked.map((row) => (
                      <li key={row.path}>
                        <label className="flex cursor-pointer items-center justify-between gap-3 rounded-[6px] px-2 py-1.5 text-[13px] hover:bg-tyba-raised hover:shadow-[var(--tyba-row-raised)]">
                          <span className="flex min-w-0 items-center gap-2">
                            <input
                              type="checkbox"
                              checked={chosen.has(row.path)}
                              onChange={() => void toggle(row.path)}
                              aria-label={t("gitPanelIncludeUntracked", {
                                path: row.path,
                              })}
                              className="shrink-0 accent-tyba-green"
                            />
                            <RowName row={row} />
                          </span>
                          <Counts row={row} />
                        </label>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </div>

            {plan.mode !== "nothing" && (
              <label className="flex flex-col gap-1.5 text-[11px] text-tyba-text-faint">
                {t("gitPanelCommitMessage")}
                <input
                  type="text"
                  spellCheck={false}
                  autoCapitalize="off"
                  autoCorrect="off"
                  value={message}
                  onChange={(e) => setMessage(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void commit();
                  }}
                  className="rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-2 text-[13px] text-tyba-text outline-none focus:border-tyba-green/50"
                />
              </label>
            )}
            <div className="flex gap-2">
              {plan.mode !== "nothing" && (
                <Button
                  size="sm"
                  className="h-auto flex-1 rounded-[6px] bg-tyba-green px-3.5 py-[7px] text-[12px] font-normal text-tyba-text-inverse hover:bg-tyba-green/90"
                  disabled={busy || !message.trim()}
                  onClick={() => void commit()}
                >
                  {plan.mode === "staged"
                    ? t("gitPanelCommitStaged", { count: diff?.staged_files.length ?? 0 })
                    : t("gitPanelCommitTracked")}
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                className="h-auto rounded-[6px] border border-tyba-border-strong bg-transparent px-3.5 py-[7px] text-[12px] font-normal text-tyba-text-muted hover:bg-transparent hover:text-tyba-text"
                onClick={onOpenDiff}
              >
                {t("gitPanelViewDiff")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                className="h-auto rounded-[6px] border border-tyba-border-strong bg-transparent px-3.5 py-[7px] text-[12px] font-normal text-tyba-text-muted hover:bg-transparent hover:text-tyba-text"
                onClick={onOpenHistory}
              >
                {t("gitLogOpen")}
              </Button>
            </div>
            {actionError && (
              <p role="alert" className="text-[11px] text-tyba-red">
                {actionError}
              </p>
            )}
            <p className="text-[11px] text-tyba-text-faint">{t("gitPanelPushNote")}</p>
          </div>
        </>
      )}
    </div>
  );
}
