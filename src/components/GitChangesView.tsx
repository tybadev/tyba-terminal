import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Minus, Plus, Sparkle } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { GitLoading, useDiffStep } from "./GitLoading";
import { requestConfirm } from "../lib/confirm";
import { openEditorTab } from "../lib/contentTabs";
import { numberHunk } from "../lib/diff";
import { translateError } from "../lib/errors";
import { amendPlan, commitPlan, pruneChosen, toggleUntracked } from "../lib/gitPanel";
import {
  onRepoChanged,
  sessionDiff,
  sessionDiffHunks,
  suggestCommitMessage,
  worktreeCommit,
  worktreeCommitAmend,
  worktreeDiscard,
  worktreeHunkAction,
  worktreeStage,
  worktreeUnstage,
  type DiffScope,
  type FileDiff,
  type FileHunks,
  type HunkAction,
  type Session,
  type SessionDiff,
} from "../lib/ipc";
import { isSensitivePath } from "../lib/sensitivePaths";

const LETTER: Record<FileDiff["status"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  type_changed: "T",
  unmerged: "U",
  other: "?",
};

const LETTER_TONE: Record<FileDiff["status"], string> = {
  added: "text-tyba-green",
  modified: "text-tyba-amber",
  deleted: "text-tyba-red",
  renamed: "text-tyba-amber",
  copied: "text-tyba-green",
  type_changed: "text-tyba-amber",
  unmerged: "text-tyba-red",
  other: "text-tyba-text-faint",
};

const LINE: Record<string, string> = {
  add: "bg-tyba-diff-add-bg text-tyba-diff-add-text",
  del: "bg-tyba-diff-del-bg text-tyba-diff-del-text",
  context: "text-tyba-text-muted",
};

const SMALL_BTN =
  "shrink-0 whitespace-nowrap rounded-[5px] border border-tyba-border-strong px-2 py-0.5 text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50";

type Pick = { path: string; scope: DiffScope; oldPath: string | null; untracked: boolean };

function split(path: string): { name: string; dir: string } {
  const i = path.lastIndexOf("/");
  return i < 0 ? { name: path, dir: "" } : { name: path.slice(i + 1), dir: path.slice(0, i) };
}

/**
 * Seção Alterações da tela de Git: a árvore de trabalho. À esquerda o que está
 * preparado, alterado e novo (novo só entra marcado); à direita o diff do
 * arquivo escolhido, com ação por trecho; embaixo da lista, o commit.
 */
export function GitChangesView({
  session,
  suggestAgent,
}: {
  session: Session;
  suggestAgent: string;
}) {
  const { t } = useTranslation();
  const [diff, setDiff] = useState<SessionDiff | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loadAt] = useState(() => Date.now());
  const diffStep = useDiffStep(session);
  const [pick, setPick] = useState<Pick | null>(null);
  const [hunks, setHunks] = useState<FileHunks | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(() => new Set());
  const [message, setMessage] = useState("");
  const [amend, setAmend] = useState(false);
  const [busy, setBusy] = useState(false);
  const [suggesting, setSuggesting] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    sessionDiff(session.id)
      .then((d) => {
        setDiff(d);
        setLoadError(null);
      })
      .catch((e) => setLoadError(translateError(e, t)));
  }, [session.id, t]);

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

  const untrackedSet = useMemo(() => new Set(diff?.untracked ?? []), [diff]);
  const staged = diff?.staged_files ?? [];
  const unstaged = (diff?.unstaged_files ?? []).filter((f) => !untrackedSet.has(f.path));
  const untracked = (diff?.unstaged_files ?? []).filter((f) => untrackedSet.has(f.path));
  const plan = useMemo(() => commitPlan(diff, chosen), [diff, chosen]);
  // Amend não prepara rastreados sozinho: sem nada preparado, só a mensagem.
  const amending = useMemo(() => amendPlan(diff, chosen), [diff, chosen]);

  useEffect(() => {
    setChosen((prev) => pruneChosen(prev, diff));
  }, [diff]);

  // Arquivo escolhido some (commitado, descartado) → cai no primeiro da lista.
  useEffect(() => {
    if (!diff) return;
    const all: Pick[] = [
      ...staged.map((f) => ({ path: f.path, scope: "staged" as const, oldPath: f.old_path ?? null, untracked: false })),
      ...unstaged.map((f) => ({ path: f.path, scope: "unstaged" as const, oldPath: f.old_path ?? null, untracked: false })),
      ...untracked.map((f) => ({ path: f.path, scope: "unstaged" as const, oldPath: null, untracked: true })),
    ];
    if (!pick || !all.some((p) => p.path === pick.path && p.scope === pick.scope)) {
      setPick(all[0] ?? null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [diff]);

  useEffect(() => {
    if (!pick) {
      setHunks(null);
      return;
    }
    let alive = true;
    sessionDiffHunks(session.id, pick.path, pick.scope, pick.oldPath)
      .then((h) => alive && setHunks(h))
      .catch((e) => alive && setActionError(translateError(e, t)));
    return () => {
      alive = false;
    };
  }, [session.id, pick, diff, t]);

  const act = async (run: () => Promise<unknown>) => {
    setActionError(null);
    try {
      await run();
      refresh();
    } catch (e) {
      setActionError(translateError(e, t));
    }
  };

  const discardFile = async (path: string) => {
    const ok = await requestConfirm({
      title: t("gitChangesDiscardConfirm", { path }),
      detail: t("gitChangesDiscardDetail"),
      confirmLabel: t("gitChangesDiscard"),
      destructive: true,
    });
    if (ok) await act(() => worktreeDiscard(session.id, [path]));
  };

  const hunkAction = async (index: number, header: string, action: HunkAction) => {
    if (!pick) return;
    if (action === "discard") {
      const ok = await requestConfirm({
        title: t("gitChangesDiscardHunkConfirm"),
        detail: t("gitChangesDiscardDetail"),
        confirmLabel: t("gitChangesDiscard"),
        destructive: true,
      });
      if (!ok) return;
    }
    await act(() => worktreeHunkAction(session.id, pick.path, pick.scope, index, header, action));
  };

  const toggleNew = async (path: string) => {
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

  const commit = async () => {
    if (busy || !message.trim()) return;
    if (!amend && plan.mode === "nothing") return;
    setBusy(true);
    setActionError(null);
    try {
      const stage = amend ? amending.stage : plan.stage;
      if (stage.length > 0) await worktreeStage(session.id, stage);
      if (amend) await worktreeCommitAmend(session.id, message);
      else await worktreeCommit(session.id, message);
      setMessage("");
      setAmend(false);
      refresh();
    } catch (e) {
      setActionError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const row = (file: FileDiff, scope: DiffScope, isNew: boolean) => {
    const { name, dir } = split(file.path);
    const on = pick?.path === file.path && pick.scope === scope;
    const select = () =>
      setPick({ path: file.path, scope, oldPath: file.old_path ?? null, untracked: isNew });
    return (
      <div
        key={`${scope}:${file.path}`}
        role="button"
        tabIndex={0}
        onClick={select}
        onKeyDown={(e) => {
          if (e.key === "Enter") select();
        }}
        className={`group/row flex cursor-pointer items-center gap-2 rounded-[6px] px-2 py-[5px] text-[12.5px] outline-none ${
          on
            ? "bg-tyba-raised shadow-[var(--tyba-raised-ring)]"
            : "hover:bg-tyba-raised/60 focus-visible:bg-tyba-raised/60"
        }`}
      >
        {isNew && (
          <input
            type="checkbox"
            checked={chosen.has(file.path)}
            onClick={(e) => e.stopPropagation()}
            onChange={() => void toggleNew(file.path)}
            aria-label={t("gitPanelIncludeUntracked", { path: file.path })}
            className="shrink-0 accent-tyba-green"
          />
        )}
        <span className={`shrink-0 font-mono ${LETTER_TONE[file.status]}`}>{LETTER[file.status]}</span>
        <span className="min-w-0 flex-1 truncate" title={file.path}>
          <span className="text-tyba-text">{name}</span>
          {dir && <span className="pl-1.5 text-[11px] text-tyba-text-faint">{dir}</span>}
        </span>
        {isSensitivePath(file.path) && (
          <span className="shrink-0 rounded-[4px] bg-tyba-red/[.12] px-1 text-[10px] text-tyba-red">
            {t("gitPanelLooksSecret")}
          </span>
        )}
        <span className="shrink-0 font-mono text-[11px]">
          {(file.insertions ?? 0) > 0 && <span className="text-tyba-green">+{file.insertions}</span>}{" "}
          {(file.deletions ?? 0) > 0 && <span className="text-tyba-red">−{file.deletions}</span>}
        </span>
        {!isNew && (
          <button
            type="button"
            aria-label={t(scope === "staged" ? "diffUnstageFile" : "diffStageFile")}
            title={t(scope === "staged" ? "diffUnstageFile" : "diffStageFile")}
            onClick={(e) => {
              e.stopPropagation();
              void act(() =>
                scope === "staged"
                  ? worktreeUnstage(session.id, [file.path])
                  : worktreeStage(session.id, [file.path]),
              );
            }}
            className="flex size-5 shrink-0 items-center justify-center rounded-[4px] border border-tyba-border-strong text-tyba-text-muted opacity-0 hover:text-tyba-text group-hover/row:opacity-100 group-focus-visible/row:opacity-100"
          >
            {scope === "staged" ? <Minus size={11} /> : <Plus size={11} />}
          </button>
        )}
      </div>
    );
  };

  const groupHeader = (label: string, action?: { label: string; run: () => void }) => (
    <div className="flex items-center justify-between px-2 pb-1.5 pt-3.5 first:pt-1.5">
      <span className="tyba-label">{label}</span>
      {action && (
        <button type="button" className={SMALL_BTN} onClick={action.run}>
          {action.label}
        </button>
      )}
    </div>
  );

  const total = staged.length + unstaged.length + untracked.length;
  const commitLabel = amend
    ? t(amending.messageOnly ? "gitChangesAmendMessageOnly" : "gitChangesAmend")
    : plan.mode === "staged"
      ? t("gitPanelCommitStaged", { count: staged.length })
      : plan.mode === "tracked"
        ? t("gitPanelCommitTracked")
        : t("gitPanelCommit");

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <div className="flex w-[360px] shrink-0 flex-col tyba-divide-r">
        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {loadError && !diff && <p className="p-3 text-[12px] text-tyba-text-faint">{t("gitPanelNoRepo")}</p>}
          {!loadError && !diff && (
            <GitLoading {...diffStep} startedAt={loadAt} shape="list" rows={5} className="-m-2" />
          )}
          {/* A atualização falhou mas a lista anterior segue na tela: avisa. */}
          {loadError && diff && (
            <p role="status" className="px-2 pb-1 text-[11px] text-tyba-amber" title={loadError}>
              {t("gitRefreshFailed")} ·{" "}
              <button type="button" onClick={refresh} className="hover:underline">
                {t("gitRefreshRetry")}
              </button>
            </p>
          )}
          {diff && total === 0 && (
            <p className="flex items-center gap-2 p-3 text-[12px] text-tyba-text-muted">
              <span className="text-tyba-green">✓</span>
              {t("gitPanelClean")}
            </p>
          )}
          {staged.length > 0 && (
            <>
              {groupHeader(t("gitChangesStaged", { count: staged.length }), {
                label: t("gitChangesUnstageAll"),
                run: () => void act(() => worktreeUnstage(session.id, [])),
              })}
              {staged.map((f) => row(f, "staged", false))}
            </>
          )}
          {unstaged.length > 0 && (
            <>
              {groupHeader(t("gitChangesUnstaged", { count: unstaged.length }), {
                label: t("gitChangesStageAll"),
                run: () => void act(() => worktreeStage(session.id, unstaged.map((f) => f.path))),
              })}
              {unstaged.map((f) => row(f, "unstaged", false))}
            </>
          )}
          {untracked.length > 0 && (
            <>
              {groupHeader(t("gitPanelUntracked", { count: untracked.length }))}
              {untracked.map((f) => row(f, "unstaged", true))}
            </>
          )}
        </div>

        <form
          aria-label={t("gitPanelCommitMessage")}
          className="flex shrink-0 flex-col gap-2 tyba-divide-t bg-tyba-raised px-3.5 py-3"
          onSubmit={(e) => {
            e.preventDefault();
            void commit();
          }}
        >
          <div className="flex gap-1.5">
            <textarea
              rows={3}
              aria-label={t("gitPanelCommitMessage")}
              placeholder={t("gitPanelCommitMessage")}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  void commit();
                }
              }}
              className="min-w-0 flex-1 resize-none rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-2 text-[12.5px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
            />
            <button
              type="button"
              aria-label={t("commitSuggest")}
              title={t("commitSuggest")}
              disabled={suggesting || staged.length === 0}
              onClick={() => {
                setSuggesting(true);
                suggestCommitMessage(session.id, suggestAgent)
                  .then(setMessage)
                  .catch((e) => setActionError(translateError(e, t)))
                  .finally(() => setSuggesting(false));
              }}
              className="flex w-9 shrink-0 items-center justify-center rounded-[6px] border border-tyba-border-field text-tyba-text-muted hover:text-tyba-text disabled:opacity-40"
            >
              <Sparkle size={14} className={suggesting ? "animate-pulse" : ""} />
            </button>
          </div>
          <div className="flex items-center gap-2">
            <Button
              type="submit"
              size="sm"
              disabled={busy || !message.trim() || (!amend && plan.mode === "nothing")}
              className="h-auto flex-1 rounded-[6px] bg-tyba-green px-3 py-[7px] text-[12px] font-normal text-tyba-text-inverse hover:bg-tyba-green/90"
            >
              {commitLabel}
            </Button>
            <label className="flex shrink-0 items-center gap-1.5 text-[11px] text-tyba-text-muted">
              <input
                type="checkbox"
                checked={amend}
                onChange={(e) => setAmend(e.target.checked)}
                className="accent-tyba-green"
              />
              amend
            </label>
          </div>
          {actionError && (
            <p role="alert" className="text-[11px] text-tyba-red">
              {actionError}
            </p>
          )}
          <p className="text-[11px] text-tyba-text-faint">{t("gitChangesNote")}</p>
        </form>
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        {pick ? (
          <>
            <div className="flex shrink-0 items-center gap-2 tyba-divide-b px-[18px] py-2 text-[12px]">
              <span className="min-w-0 flex-1 truncate" title={pick.path}>
                <span className="text-tyba-text">{split(pick.path).name}</span>
                <span className="pl-1.5 text-[11px] text-tyba-text-faint">{split(pick.path).dir}</span>
              </span>
              <button
                type="button"
                className={SMALL_BTN}
                onClick={() =>
                  void openEditorTab(pick.path, undefined, { session: session.id }).catch((e) =>
                    setActionError(String(e)),
                  )
                }
              >
                {t("gitChangesEditor")}
              </button>
              {pick.scope === "unstaged" && !pick.untracked && (
                <button type="button" className={SMALL_BTN} onClick={() => void discardFile(pick.path)}>
                  {t("gitChangesDiscard")}
                </button>
              )}
              {!pick.untracked && (
                <button
                  type="button"
                  className={`${SMALL_BTN} text-tyba-text`}
                  onClick={() =>
                    void act(() =>
                      pick.scope === "staged"
                        ? worktreeUnstage(session.id, [pick.path])
                        : worktreeStage(session.id, [pick.path]),
                    )
                  }
                >
                  {t(pick.scope === "staged" ? "gitChangesUnstageFile" : "gitChangesStageFile")}
                </button>
              )}
            </div>
            <div className="min-h-0 flex-1 overflow-auto py-1.5 font-mono text-[12.5px] leading-[21px]">
              {hunks?.binary && <p className="px-[18px] py-3 text-tyba-text-faint">{t("diffBinary")}</p>}
              {hunks?.hunks.map((hunk, index) => (
                <div key={`${index}:${hunk.header}`} className="pb-3">
                  <div className="flex items-center gap-2 px-[18px] py-0.5 text-[11px] text-tyba-violet">
                    <span className="min-w-0 flex-1 truncate">{hunk.header}</span>
                    {!pick.untracked && pick.scope === "unstaged" && (
                      <>
                        <button
                          type="button"
                          className={SMALL_BTN}
                          onClick={() => void hunkAction(index, hunk.header, "stage")}
                        >
                          {t("gitChangesStageHunk")}
                        </button>
                        <button
                          type="button"
                          className={SMALL_BTN}
                          onClick={() => void hunkAction(index, hunk.header, "discard")}
                        >
                          {t("gitChangesDiscardHunk")}
                        </button>
                      </>
                    )}
                    {pick.scope === "staged" && (
                      <button
                        type="button"
                        className={SMALL_BTN}
                        onClick={() => void hunkAction(index, hunk.header, "unstage")}
                      >
                        {t("gitChangesUnstageHunk")}
                      </button>
                    )}
                  </div>
                  {numberHunk(hunk).map((l) => (
                    <div key={l.idx} className={`flex whitespace-pre ${LINE[l.line.kind]}`}>
                      <span className="w-11 shrink-0 select-none pr-2.5 text-right text-tyba-text-ghost">
                        {l.oldNo ?? ""}
                      </span>
                      <span className="w-11 shrink-0 select-none pr-2.5 text-right text-tyba-text-ghost">
                        {l.newNo ?? ""}
                      </span>
                      <span className="w-4 shrink-0 select-none text-center">
                        {l.line.kind === "add" ? "+" : l.line.kind === "del" ? "−" : " "}
                      </span>
                      <span className="pr-4">{l.line.text}</span>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </>
        ) : (
          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
            {diff && t("gitPanelClean")}
          </div>
        )}
      </div>
    </div>
  );
}
