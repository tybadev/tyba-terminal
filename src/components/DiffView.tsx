import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ArrowSquareOut,
  ArrowsInSimple,
  ArrowsOutSimple,
  CaretDown,
  CaretRight,
  ChatCircleText,
  GitBranch,
  GitCommit,
  Minus,
  Warning,
  PaperPlaneTilt,
  Plus,
  Rows,
  SquareSplitHorizontal,
  Trash,
  UploadSimple,
  X,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  onRepoChanged,
  openWorktreeFile,
  sessionBranchDiff,
  sessionBranchHunks,
  sessionConflicts,
  sessionDiff,
  sessionDiffHunks,
  worktreeCommit,
  worktreeDiscard,
  worktreePush,
  worktreeStage,
  worktreeUnstage,
  type ConflictState,
  type FileDiff,
  type FileHunks,
  type Session,
  type SessionDiff,
} from "../lib/ipc";
import {
  buildRows,
  buildReviewPrompt,
  defaultCollapsed,
  MANY_FILES_AUTO_COLLAPSE,
  fileKeyOf,
  isSectionOpen,
  keyPath,
  keyScope,
  langOfPath,
  toggleSection,
  type DiffMode,
  type DiffScopeKey,
  type ReviewComment,
  type Row,
  type SectionOpenState,
} from "../lib/diff";
import { highlightBlock, type TokenSpan } from "../lib/highlight";
import { BranchPicker } from "./BranchPicker";
import { DeliverySection } from "./DeliverySection";

const STATUS_LETTER: Record<FileDiff["status"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  type_changed: "T",
  other: "?",
};

const STATUS_COLOR: Record<FileDiff["status"], string> = {
  added: "text-tyba-green",
  modified: "text-tyba-amber",
  deleted: "text-tyba-red",
  renamed: "text-tyba-violet",
  copied: "text-tyba-violet",
  type_changed: "text-tyba-text-muted",
  other: "text-tyba-text-muted",
};

const SECTION_LABEL_KEY: Record<DiffScopeKey, string> = {
  committed: "diffCommitted",
  staged: "diffStaged",
  unstaged: "diffChanges",
};

const CONFLICT_OPERATION_KEY: Record<ConflictState["operation"], string> = {
  merge: "conflictMerge",
  rebase: "conflictRebase",
  cherry_pick: "conflictCherryPick",
};

function Numstat({ file }: { file: FileDiff }) {
  return (
    <span className="flex shrink-0 items-center gap-1.5 font-mono text-[10px] tabular-nums">
      {file.insertions === null ? (
        <span className="text-tyba-text-faint">bin</span>
      ) : (
        <>
          <span className="text-tyba-green">+{file.insertions}</span>
          <span className="text-tyba-red">−{file.deletions ?? 0}</span>
        </>
      )}
    </span>
  );
}

function LineText({
  text,
  spans,
}: {
  text: string;
  spans: TokenSpan[] | undefined;
}) {
  if (!spans) return <>{text}</>;
  return (
    <>
      {spans.map((span, i) => (
        <span key={i} style={span.color ? { color: span.color } : undefined}>
          {span.text}
        </span>
      ))}
    </>
  );
}

const LINE_BG: Record<string, string> = {
  add: "bg-tyba-green/[.07]",
  del: "bg-tyba-red/[.07]",
  context: "",
};

const GUTTER_SIGN: Record<string, { sign: string; cls: string }> = {
  add: { sign: "+", cls: "text-tyba-green" },
  del: { sign: "−", cls: "text-tyba-red" },
  context: { sign: " ", cls: "" },
};

interface Props {
  session: Session;
  editor: string;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
  onSendToAgent: (prompt: string) => Promise<void>;
  onResolveConflicts: (state: ConflictState) => Promise<void>;
}

type DisplayRow =
  | Row
  | { kind: "comment"; key: string; comment: ReviewComment; anchor: string }
  | { kind: "commentEditor"; key: string; anchor: string };

const DISCARD_ARM_MS = 3000;

// Stale-while-revalidate: o último diff de cada sessão sobrevive ao desmonte do
// painel. Reabrir pinta na hora enquanto revalida em background, em vez de
// mostrar "carregando" e recomputar do zero. O componente é keyed por session.id,
// então cada montagem semeia o estado inicial daqui. Teto com evicção FIFO pra
// não crescer sem limite ao longo da vida do app.
const DIFF_CACHE_CAP = 64;
const diffCache = new Map<string, SessionDiff>();
function cacheDiff(id: string, d: SessionDiff) {
  diffCache.delete(id);
  diffCache.set(id, d);
  while (diffCache.size > DIFF_CACHE_CAP) {
    const oldest = diffCache.keys().next().value;
    if (oldest === undefined) break;
    diffCache.delete(oldest);
  }
}

export function DiffView({
  session,
  editor,
  expanded,
  onToggleExpand,
  onClose,
  onSendToAgent,
  onResolveConflicts,
}: Props) {
  const { t } = useTranslation();
  const [diff, setDiff] = useState<SessionDiff | null>(
    () => diffCache.get(session.id) ?? null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [hunksByKey, setHunksByKey] = useState<
    Record<string, FileHunks | undefined>
  >({});
  const [hunkErrors, setHunkErrors] = useState<Record<string, string>>({});
  const [collapsedByKey, setCollapsedByKey] = useState<
    Record<string, boolean | undefined>
  >({});
  const [openSections, setOpenSections] = useState<SectionOpenState>({});
  const [mode, setMode] = useState<DiffMode>("unified");
  const [tokens, setTokens] = useState<Record<string, TokenSpan[][]>>({});
  const [comments, setComments] = useState<Record<string, ReviewComment>>({});
  const [draftAnchor, setDraftAnchor] = useState<string | null>(null);
  const [draftText, setDraftText] = useState("");
  const [sendState, setSendState] = useState<
    "idle" | "sending" | "sent" | "error"
  >("idle");
  const [sendError, setSendError] = useState<string | null>(null);
  const [commitMsg, setCommitMsg] = useState("");
  const [busy, setBusy] = useState<null | "commit" | "push" | "git">(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [pushedInfo, setPushedInfo] = useState<string | null>(null);
  const [discardArm, setDiscardArm] = useState<string | null>(null);
  const [browseBranch, setBrowseBranch] = useState<string | null>(null);
  const [conflicts, setConflicts] = useState<ConflictState | null>(null);
  const [resolvingConflicts, setResolvingConflicts] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const requestedRef = useRef(new Set<string>());
  const highlightedRef = useRef(new Set<string>());

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // Esc só fecha se o foco está no diff (ou em lugar nenhum) — um Esc
      // digitado no terminal ao lado não pode derrubar o painel.
      const active = document.activeElement;
      if (
        active &&
        active !== document.body &&
        !containerRef.current?.contains(active)
      ) {
        return;
      }
      onClose();
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const clearCaches = useCallback((keep: (fileKey: string) => boolean) => {
    const hunkKeyKeep = (hunkKey: string) =>
      keep(hunkKey.slice(0, hunkKey.lastIndexOf(":")));
    for (const key of [...requestedRef.current]) {
      if (!keep(key)) requestedRef.current.delete(key);
    }
    for (const key of [...highlightedRef.current]) {
      if (!hunkKeyKeep(key)) highlightedRef.current.delete(key);
    }
    setHunksByKey((prev) =>
      Object.fromEntries(Object.entries(prev).filter(([k]) => keep(k))),
    );
    setHunkErrors((prev) =>
      Object.fromEntries(Object.entries(prev).filter(([k]) => keep(k))),
    );
    setTokens((prev) =>
      Object.fromEntries(Object.entries(prev).filter(([k]) => hunkKeyKeep(k))),
    );
  }, []);

  /// Única porta de atualização: compara HEAD e as listas de trabalho com
  /// o snapshot anterior e invalida só o que mudou de verdade — HEAD novo
  /// derruba tudo; staging/worktree novos derrubam o que não é committed.
  const lastSnapRef = useRef<{
    root: string;
    head: string;
    work: string;
  } | null>(null);
  const refresh = useCallback(() => {
    if (!browseBranch) {
      sessionConflicts(session.id)
        .then(setConflicts)
        .catch(() => setConflicts(null));
    }
    const load = browseBranch
      ? sessionBranchDiff(session.id, browseBranch)
      : sessionDiff(session.id);
    load
      .then((d) => {
        const head = d.commits[0]?.sha ?? "";
        const work = JSON.stringify([d.staged_files, d.unstaged_files]);
        const prev = lastSnapRef.current;
        // `cd` troca o repo por baixo da mesma sessão: tudo que o painel
        // acumulou (hunks, colapsos, seções) é de OUTRO repo — zera.
        if (prev && prev.root !== d.root) {
          clearCaches(() => false);
          setCollapsedByKey({});
          setOpenSections({});
        } else if (prev && prev.head !== head) {
          clearCaches(() => false);
        } else if (prev && prev.work !== work) {
          clearCaches((key) => keyScope(key) === "committed");
        }
        lastSnapRef.current = { root: d.root, head, work };
        if (!browseBranch) cacheDiff(session.id, d);
        setDiff(d);
        setLoadError(null);
      })
      .catch((e) => setLoadError(String(e)));
  }, [session.id, browseBranch, clearCaches]);

  // Entrar/sair do modo explorar troca a fonte do diff inteira — nada do
  // estado acumulado sobrevive à troca.
  const browseTo = useCallback(
    (branch: string | null) => {
      setBrowseBranch(branch);
      clearCaches(() => false);
      setCollapsedByKey({});
      setOpenSections({});
      lastSnapRef.current = null;
      setDiff(branch === null ? (diffCache.get(session.id) ?? null) : null);
      setLoadError(null);
    },
    [clearCaches, session.id],
  );

  useEffect(() => {
    refresh();
  }, [refresh]);

  // O watcher do core só enxerga o git dir (index/HEAD/refs) — edição de
  // arquivo no worktree não gera evento. Enquanto o painel está aberto,
  // um poll leve (numstat) cobre esse buraco; o refresh já é no-op de
  // renderização quando nada mudou.
  useEffect(() => {
    const id = window.setInterval(() => refresh(), 5000);
    return () => window.clearInterval(id);
  }, [refresh]);

  useEffect(() => {
    let timer: number | null = null;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onRepoChanged(() => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => refresh(), 600);
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [refresh]);

  const gitAction = useCallback(
    async (
      action: () => Promise<unknown>,
      kind: "commit" | "push" | "git" = "git",
    ): Promise<boolean> => {
      if (busy) return false;
      setBusy(kind);
      setActionError(null);
      setPushedInfo(null);
      try {
        await action();
        refresh();
        return true;
      } catch (e) {
        setActionError(String(e));
        return false;
      } finally {
        setBusy(null);
      }
    },
    [busy, refresh],
  );

  const discardTimer = useRef<number | null>(null);
  const armDiscard = useCallback((key: string) => {
    if (discardTimer.current !== null) {
      window.clearTimeout(discardTimer.current);
    }
    setDiscardArm(key);
    discardTimer.current = window.setTimeout(() => {
      discardTimer.current = null;
      setDiscardArm(null);
    }, DISCARD_ARM_MS);
  }, []);

  // No erro, a key CONTINUA em requestedRef — soltá-la faria o effect
  // redisparar o request a cada render, um loop infinito de processos git
  // contra um repo que está falhando. Retry é explícito (botão) ou vem
  // de graça quando um refresh invalida os caches.
  const fetchHunks = useCallback(
    (scope: DiffScopeKey, file: FileDiff) => {
      const key = fileKeyOf(scope, file.path);
      if (requestedRef.current.has(key)) return;
      requestedRef.current.add(key);
      const load = browseBranch
        ? sessionBranchHunks(session.id, browseBranch, file.path, file.old_path)
        : sessionDiffHunks(session.id, file.path, scope, file.old_path);
      void load
        .then((hunks) => {
          setHunksByKey((prev) => ({ ...prev, [key]: hunks }));
          setHunkErrors((prev) => {
            if (!(key in prev)) return prev;
            const { [key]: _, ...rest } = prev;
            return rest;
          });
        })
        .catch((e) => {
          setHunkErrors((prev) => ({ ...prev, [key]: String(e) }));
        });
    },
    [session.id, browseBranch],
  );

  const retryHunks = useCallback(
    (fileKey: string) => {
      requestedRef.current.delete(fileKey);
      setHunkErrors((prev) => {
        const { [fileKey]: _, ...rest } = prev;
        return rest;
      });
    },
    [],
  );

  useEffect(() => {
    if (!diff) return;
    for (const [scope, files] of [
      ["committed", diff.files],
      ["staged", diff.staged_files],
      ["unstaged", diff.unstaged_files],
    ] as const) {
      const manyFiles = files.length > MANY_FILES_AUTO_COLLAPSE;
      for (const file of files) {
        const key = fileKeyOf(scope, file.path);
        const collapsed =
          collapsedByKey[key] ??
          defaultCollapsed(file, hunksByKey[key], manyFiles);
        if (!collapsed) fetchHunks(scope, file);
      }
    }
  }, [diff, collapsedByKey, hunksByKey, fetchHunks]);

  useEffect(() => {
    for (const [key, hunks] of Object.entries(hunksByKey)) {
      if (!hunks || hunks.binary) continue;
      const lang = langOfPath(keyPath(key));
      if (!lang) continue;
      for (const [hi, hunk] of hunks.hunks.entries()) {
        const hunkKey = `${key}:h${hi}`;
        if (highlightedRef.current.has(hunkKey)) continue;
        if (hunk.lines.length > 2000) continue;
        highlightedRef.current.add(hunkKey);
        void highlightBlock(
          hunk.lines.map((l) => l.text),
          lang,
        ).then((spans) => {
          if (spans) setTokens((prev) => ({ ...prev, [hunkKey]: spans }));
        });
      }
    }
  }, [hunksByKey]);

  const rows = useMemo<Row[]>(
    () =>
      diff
        ? buildRows({
            committed: diff.files,
            staged: diff.staged_files,
            unstaged: diff.unstaged_files,
            hunksByKey,
            collapsedByKey,
            mode,
          })
        : [],
    [diff, hunksByKey, collapsedByKey, mode],
  );

  const displayRows = useMemo<DisplayRow[]>(() => {
    const out: DisplayRow[] = [];
    for (const row of rows) {
      out.push(row);
      if (row.kind !== "line") continue;
      const saved = comments[row.key];
      if (saved) {
        out.push({
          kind: "comment",
          key: `${row.key}:c`,
          comment: saved,
          anchor: row.key,
        });
      }
      if (draftAnchor === row.key && !saved) {
        out.push({ kind: "commentEditor", key: `${row.key}:e`, anchor: row.key });
      }
    }
    return out;
  }, [rows, comments, draftAnchor]);

  const virtualizer = useVirtualizer({
    count: displayRows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const kind = displayRows[index]?.kind;
      if (kind === "line" || kind === "split") return 20;
      if (kind === "hunk") return 24;
      if (kind === "file") return 40;
      if (kind === "section") return 40;
      if (kind === "commentEditor") return 96;
      if (kind === "comment") return 56;
      return 36;
    },
    getItemKey: (index) => {
      const row = displayRows[index];
      return row.kind === "section" ? `section:${row.scope}` : row.key;
    },
    overscan: 30,
  });

  const virtualItems = virtualizer.getVirtualItems();

  const stickyFile = useMemo(() => {
    const first = virtualItems[0];
    if (!first) return null;
    for (let i = first.index; i >= 0; i -= 1) {
      const row = displayRows[i];
      if (row?.kind === "file") return row;
      if (row?.kind === "section") return null;
    }
    return null;
  }, [virtualItems, displayRows]);

  const toggleFile = useCallback((key: string, next: boolean) => {
    setCollapsedByKey((prev) => ({ ...prev, [key]: next }));
  }, []);

  const jumpTo = useCallback(
    (key: string) => {
      const index = displayRows.findIndex(
        (r) => r.kind === "file" && r.key === key,
      );
      if (index >= 0) virtualizer.scrollToIndex(index, { align: "start" });
    },
    [displayRows, virtualizer],
  );

  const saveDraft = useCallback(
    (anchor: string) => {
      const text = draftText.trim();
      const row = rows.find((r) => r.kind === "line" && r.key === anchor);
      if (!text || !row || row.kind !== "line") return;
      setComments((prev) => ({
        ...prev,
        [anchor]: {
          path: row.path,
          oldNo: row.oldNo,
          newNo: row.newNo,
          excerpt: row.line.text.trim().slice(0, 120),
          text,
        },
      }));
      setDraftAnchor(null);
      setDraftText("");
    },
    [draftText, rows],
  );

  const commentList = useMemo(() => Object.values(comments), [comments]);

  const sendToAgent = useCallback(async () => {
    if (commentList.length === 0 || sendState === "sending") return;
    setSendState("sending");
    setSendError(null);
    try {
      await onSendToAgent(
        buildReviewPrompt(diff?.base_ref.slice(0, 7) ?? "", commentList),
      );
      setSendState("sent");
      setComments({});
    } catch (e) {
      setSendState("error");
      setSendError(String(e));
    }
  }, [commentList, sendState, onSendToAgent, diff]);

  const branch = session.worktree?.branch ?? "";
  const baseShort = diff?.base_ref.slice(0, 7) ?? "";
  const stagedCount = diff?.staged_files.length ?? 0;
  const unstagedCount = diff?.unstaged_files.length ?? 0;

  const fileActions = (scope: DiffScopeKey, file: FileDiff) => {
    const discardKey = `file:${file.path}`;
    return (
      <span
        className="hidden shrink-0 items-center gap-1 group-hover/file:flex"
        onClick={(e) => e.stopPropagation()}
      >
        {scope === "unstaged" && (
          <button
            title={t("diffStageFile")}
            aria-label={t("diffStageFile")}
            onClick={() =>
              void gitAction(() => worktreeStage(session.id, [file.path]))
            }
            className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-green"
          >
            <Plus size={12} weight="bold" />
          </button>
        )}
        {scope === "staged" && (
          <button
            title={t("diffUnstageFile")}
            aria-label={t("diffUnstageFile")}
            onClick={() =>
              void gitAction(() => worktreeUnstage(session.id, [file.path]))
            }
            className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-amber"
          >
            <Minus size={12} weight="bold" />
          </button>
        )}
        {scope === "unstaged" &&
          (discardArm === discardKey ? (
            <button
              onClick={() => {
                if (discardTimer.current !== null) {
                  window.clearTimeout(discardTimer.current);
                  discardTimer.current = null;
                }
                setDiscardArm(null);
                void gitAction(() =>
                  worktreeDiscard(session.id, [file.path]),
                );
              }}
              className="flex h-5 items-center gap-1 rounded-[3px] bg-tyba-red/20 px-1.5 text-[10px] text-tyba-red"
            >
              {t("diffDiscardConfirm")}
            </button>
          ) : (
            <button
              title={t("diffDiscardFile")}
              aria-label={t("diffDiscardFile")}
              onClick={() => armDiscard(discardKey)}
              className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-red"
            >
              <Trash size={12} />
            </button>
          ))}
        {file.status !== "deleted" && (
          <button
            title={t("diffOpenFile")}
            aria-label={t("diffOpenFile")}
            onClick={() =>
              void openWorktreeFile(session.id, file.path, editor).catch(
                (e) => setActionError(String(e)),
              )
            }
            className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-text"
          >
            <ArrowSquareOut size={12} />
          </button>
        )}
      </span>
    );
  };

  const scopeFiles = useCallback(
    (scope: DiffScopeKey): FileDiff[] => {
      if (!diff) return [];
      if (scope === "committed") return diff.files;
      if (scope === "staged") return diff.staged_files;
      return diff.unstaged_files;
    },
    [diff],
  );

  const allFilesCollapsed = (scope: DiffScopeKey) => {
    const files = scopeFiles(scope);
    const manyFiles = files.length > MANY_FILES_AUTO_COLLAPSE;
    return files.every((file) => {
      const key = fileKeyOf(scope, file.path);
      return (
        collapsedByKey[key] ??
        defaultCollapsed(file, hunksByKey[key], manyFiles)
      );
    });
  };

  const setAllFilesCollapsed = (scope: DiffScopeKey, collapsed: boolean) => {
    setCollapsedByKey((prev) => {
      const next = { ...prev };
      for (const file of scopeFiles(scope)) {
        next[fileKeyOf(scope, file.path)] = collapsed;
      }
      return next;
    });
  };

  const BULK_BTN =
    "flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.06]";

  const sectionActions = (scope: DiffScopeKey) => {
    const collapsed = allFilesCollapsed(scope);
    const collapseToggle = scopeFiles(scope).length > 0 && (
      <button
        title={collapsed ? t("diffExpandAll") : t("diffCollapseAll")}
        aria-label={collapsed ? t("diffExpandAll") : t("diffCollapseAll")}
        onClick={() => setAllFilesCollapsed(scope, !collapsed)}
        className={`${BULK_BTN} hover:text-tyba-text`}
      >
        {collapsed ? (
          <ArrowsOutSimple size={12} />
        ) : (
          <ArrowsInSimple size={12} />
        )}
      </button>
    );
    if (scope === "staged" && stagedCount > 0) {
      return (
        <span className="flex items-center gap-1">
          {collapseToggle}
          <button
            title={t("diffUnstageAll")}
            aria-label={t("diffUnstageAll")}
            onClick={() =>
              void gitAction(() => worktreeUnstage(session.id, []))
            }
            className={`${BULK_BTN} hover:text-tyba-text`}
          >
            <Minus size={12} />
          </button>
        </span>
      );
    }
    if (scope === "unstaged" && unstagedCount > 0) {
      return (
        <span className="flex items-center gap-1">
          {collapseToggle}
          <button
            title={t("diffStageAll")}
            aria-label={t("diffStageAll")}
            onClick={() =>
              void gitAction(() => worktreeStage(session.id, []))
            }
            className={`${BULK_BTN} hover:text-tyba-text`}
          >
            <Plus size={12} />
          </button>
          {discardArm === "all" ? (
            <button
              onClick={() => {
                if (discardTimer.current !== null) {
                  window.clearTimeout(discardTimer.current);
                  discardTimer.current = null;
                }
                setDiscardArm(null);
                void gitAction(() => worktreeDiscard(session.id, []));
              }}
              className="rounded-[3px] bg-tyba-red/20 px-1.5 py-0.5 text-[10px] text-tyba-red"
            >
              {t("diffDiscardAllConfirm")}
            </button>
          ) : (
            <button
              title={t("diffDiscardAll")}
              aria-label={t("diffDiscardAll")}
              onClick={() => armDiscard("all")}
              className={`${BULK_BTN} hover:text-tyba-red`}
            >
              <Trash size={12} />
            </button>
          )}
        </span>
      );
    }
    return collapseToggle || null;
  };

  const sidebarSection = (
    scope: DiffScopeKey,
    files: FileDiff[],
  ) => {
    if (files.length === 0) return null;
    const open = isSectionOpen(openSections, scope);
    return (
      <div className="flex flex-col gap-px">
        <button
          onClick={() => setOpenSections((prev) => toggleSection(prev, scope))}
          aria-expanded={open}
          className="tyba-label flex items-center gap-1 px-2 pb-1 pt-3 hover:text-tyba-text-muted"
        >
          {open ? (
            <CaretDown size={9} className="shrink-0" />
          ) : (
            <CaretRight size={9} className="shrink-0" />
          )}
          {t(SECTION_LABEL_KEY[scope])} · {files.length}
        </button>
        {open &&
          files.map((file) => {
            const key = fileKeyOf(scope, file.path);
            const active = stickyFile?.key === key;
            return (
              <button
                key={key}
                onClick={() => jumpTo(key)}
                className={`flex items-center gap-2 rounded-[4px] px-2 py-1 text-left transition-colors ${
                  active
                    ? "bg-tyba-text/[.06] text-tyba-text"
                    : "text-tyba-text-muted hover:bg-tyba-text/[.03] hover:text-tyba-text"
                }`}
              >
                <span
                  className={`w-3 shrink-0 text-center font-mono text-[10px] ${STATUS_COLOR[file.status]}`}
                >
                  {STATUS_LETTER[file.status]}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11px]">
                  {file.path}
                </span>
                <Numstat file={file} />
              </button>
            );
          })}
      </div>
    );
  };

  const renderRow = (row: DisplayRow) => {
    switch (row.kind) {
      case "comment":
        return (
          <div className="flex items-start gap-2 border-l-2 border-tyba-green/50 bg-tyba-green/[.04] py-1.5 pl-[104px] pr-4">
            <ChatCircleText
              size={13}
              className="mt-0.5 shrink-0 text-tyba-green"
            />
            <span className="min-w-0 flex-1 whitespace-pre-wrap text-[12px] leading-snug text-tyba-text">
              {row.comment.text}
            </span>
            <button
              aria-label={t("diffCommentRemove")}
              onClick={() =>
                setComments((prev) => {
                  const next = { ...prev };
                  delete next[row.anchor];
                  return next;
                })
              }
              className="shrink-0 text-tyba-text-faint hover:text-tyba-red"
            >
              <Trash size={12} />
            </button>
          </div>
        );
      case "commentEditor":
        return (
          <div className="flex flex-col gap-1.5 border-l-2 border-tyba-green/50 bg-tyba-green/[.04] py-2 pl-[104px] pr-4">
            <textarea
              autoFocus
              value={draftText}
              onChange={(e) => setDraftText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  saveDraft(row.anchor);
                }
                if (e.key === "Escape") {
                  e.stopPropagation();
                  setDraftAnchor(null);
                  setDraftText("");
                }
              }}
              placeholder={t("diffCommentPlaceholder")}
              className="min-h-[52px] w-full max-w-xl resize-y rounded-[4px] border border-tyba-border bg-black/30 p-2 font-sans text-[12px] text-tyba-text outline-none focus:border-tyba-green/50"
            />
            <div className="flex gap-2">
              <Button size="sm" className="h-6 px-2 text-[11px]" onClick={() => saveDraft(row.anchor)}>
                {t("diffCommentSave")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-[11px]"
                onClick={() => {
                  setDraftAnchor(null);
                  setDraftText("");
                }}
              >
                {t("cancel")}
              </Button>
            </div>
          </div>
        );
      case "section":
        return (
          <div className="flex items-baseline gap-2 px-4 pb-2 pt-5">
            <span className="text-[12px] font-medium uppercase tracking-wide text-tyba-text-muted">
              {t(SECTION_LABEL_KEY[row.scope])}
            </span>
            <span className="font-mono text-[10px] text-tyba-text-faint">
              {row.count}
            </span>
            <span className="flex-1" />
            {sectionActions(row.scope)}
          </div>
        );
      case "file": {
        const { file } = row;
        return (
          <div
            role="button"
            tabIndex={0}
            onClick={() => toggleFile(row.key, !row.collapsed)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                toggleFile(row.key, !row.collapsed);
              }
            }}
            className="group/file flex w-full cursor-pointer items-center gap-2 border-t border-tyba-border bg-tyba-surface/60 px-4 py-2 text-left hover:bg-tyba-text/[.03]"
          >
            {row.collapsed ? (
              <CaretRight size={11} className="shrink-0 text-tyba-text-faint" />
            ) : (
              <CaretDown size={11} className="shrink-0 text-tyba-text-faint" />
            )}
            <span
              className={`w-3 shrink-0 text-center font-mono text-[11px] ${STATUS_COLOR[file.status]}`}
            >
              {STATUS_LETTER[file.status]}
            </span>
            <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text">
              {file.old_path ? `${file.old_path} → ${file.path}` : file.path}
            </span>
            {row.generated && (
              <span className="shrink-0 rounded-full bg-tyba-text/[.06] px-1.5 text-[9px] text-tyba-text-faint">
                {t("diffGenerated")}
              </span>
            )}
            {fileActions(row.scope, file)}
            <Numstat file={file} />
          </div>
        );
      }
      case "hunk":
        return (
          <div className="truncate px-4 py-1 font-mono text-[11px] text-tyba-violet/80">
            {row.header}
          </div>
        );
      case "line": {
        const gutter = GUTTER_SIGN[row.line.kind];
        const spans = tokens[row.hunkKey]?.[row.idx];
        return (
          <div
            className={`group/line flex items-stretch font-mono text-[12px] leading-[20px] ${LINE_BG[row.line.kind]}`}
          >
            <span className="relative w-12 shrink-0 select-none pr-2 text-right text-[10px] leading-[20px] text-tyba-text-faint">
              <button
                aria-label={t("diffCommentAdd")}
                onClick={() => {
                  setDraftAnchor(row.key);
                  setDraftText("");
                }}
                className="absolute left-0.5 top-0.5 hidden size-4 items-center justify-center rounded-[3px] bg-tyba-green text-black group-hover/line:flex"
              >
                <Plus size={11} weight="bold" />
              </button>
              {row.oldNo ?? ""}
            </span>
            <span className="w-12 shrink-0 select-none pr-2 text-right text-[10px] leading-[20px] text-tyba-text-faint">
              {row.newNo ?? ""}
            </span>
            <span
              className={`w-5 shrink-0 select-none text-center ${gutter.cls}`}
            >
              {gutter.sign}
            </span>
            <span className="min-w-0 flex-1 whitespace-pre pr-4 text-tyba-text/90">
              <LineText text={row.line.text} spans={spans} />
            </span>
          </div>
        );
      }
      case "split": {
        const side = (
          entry: typeof row.left,
          signKind: "left" | "right",
        ) => {
          if (!entry)
            return <div className="min-w-0 flex-1 bg-tyba-text/[.015]" />;
          const gutter = GUTTER_SIGN[entry.kind];
          const spans = tokens[row.hunkKey]?.[entry.idx];
          return (
            <div
              className={`flex min-w-0 flex-1 items-stretch ${LINE_BG[entry.kind]}`}
            >
              <span className="w-10 shrink-0 select-none pr-2 text-right text-[10px] leading-[20px] text-tyba-text-faint">
                {entry.no}
              </span>
              <span
                className={`w-4 shrink-0 select-none text-center ${gutter.cls}`}
              >
                {gutter.sign}
              </span>
              <span className="min-w-0 flex-1 truncate whitespace-pre pr-3 text-tyba-text/90">
                <LineText text={entry.text} spans={spans} />
              </span>
              {signKind === "left" && (
                <span className="w-px shrink-0 bg-tyba-border" />
              )}
            </div>
          );
        };
        return (
          <div className="flex font-mono text-[12px] leading-[20px]">
            {side(row.left, "left")}
            {side(row.right, "right")}
          </div>
        );
      }
      case "empty": {
        const fileKey = row.key.endsWith(":loading")
          ? row.key.slice(0, -":loading".length)
          : null;
        const error = fileKey ? hunkErrors[fileKey] : undefined;
        if (row.label === "loading" && fileKey && error) {
          return (
            <div className="flex items-center gap-2 px-16 py-2 font-mono text-[11px]">
              <span className="max-w-md truncate text-tyba-red" title={error}>
                {t("diffHunksError")}
              </span>
              <button
                onClick={() => retryHunks(fileKey)}
                className="rounded-[3px] px-1.5 py-0.5 text-tyba-text-faint hover:bg-tyba-text/[.06] hover:text-tyba-text"
              >
                {t("diffHunksRetry")}
              </button>
            </div>
          );
        }
        return (
          <div className="px-16 py-2 font-mono text-[11px] text-tyba-text-faint">
            {row.label === "binary"
              ? t("diffBinary")
              : row.label === "loading"
                ? t("diffLoading")
                : t("diffNoChanges")}
          </div>
        );
      }
    }
  };

  return (
    <div
      ref={containerRef}
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-tyba-bg"
    >
      <header className="flex h-8 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
        <GitBranch size={13} className="shrink-0 text-tyba-green" />
        <span className="min-w-0 truncate text-[12px] text-tyba-text">
          {session.title}
        </span>
        <span className="truncate font-mono text-[11px] text-tyba-text-muted">
          {branch}
        </span>
        <BranchPicker
          sessionId={session.id}
          browsing={browseBranch}
          label={
            browseBranch ?? `${t("diffBase")} ${baseShort} ‥ HEAD`
          }
          onBrowse={browseTo}
        />
        <div className="flex-1" />
        <div className="flex items-center gap-0.5 rounded-[5px] border border-tyba-border p-0.5">
          <button
            aria-pressed={mode === "unified"}
            onClick={() => setMode("unified")}
            className={`flex h-6 items-center gap-1.5 rounded-[3px] px-2 text-[11px] ${
              mode === "unified"
                ? "bg-tyba-text/[.06] text-tyba-text"
                : "text-tyba-text-faint hover:text-tyba-text"
            }`}
          >
            <Rows size={12} />
            {t("diffUnified")}
          </button>
          <button
            aria-pressed={mode === "split"}
            onClick={() => setMode("split")}
            className={`flex h-6 items-center gap-1.5 rounded-[3px] px-2 text-[11px] ${
              mode === "split"
                ? "bg-tyba-text/[.06] text-tyba-text"
                : "text-tyba-text-faint hover:text-tyba-text"
            }`}
          >
            <SquareSplitHorizontal size={12} />
            {t("diffSplit")}
          </button>
        </div>
        <Button
          variant="ghost"
          size="icon"
          aria-label={expanded ? t("diffCollapse") : t("diffExpand")}
          onClick={onToggleExpand}
          className="size-6"
        >
          {expanded ? <ArrowsInSimple size={13} /> : <ArrowsOutSimple size={13} />}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("diffClose")}
          onClick={onClose}
          className="size-6"
        >
          <X size={13} />
        </Button>
      </header>

      {conflicts && !browseBranch && (
        <div className="flex shrink-0 items-start gap-2 border-b border-tyba-border bg-tyba-red/[.06] px-3 py-2">
          <Warning size={14} className="mt-0.5 shrink-0 text-tyba-red" />
          <div className="min-w-0 flex-1">
            <p className="text-[12px] text-tyba-text">
              {t(CONFLICT_OPERATION_KEY[conflicts.operation])}
              {conflicts.ours && conflicts.theirs && (
                <span className="font-mono text-[11px] text-tyba-text-muted">
                  {" "}
                  · {conflicts.ours} ← {conflicts.theirs}
                </span>
              )}
            </p>
            {conflicts.files.length > 0 ? (
              <p className="break-words pt-0.5 font-mono text-[11px] text-tyba-red/90">
                {conflicts.files.map((f) => f.path).join("  ")}
              </p>
            ) : (
              <p className="pt-0.5 text-[11px] text-tyba-text-faint">
                {t("conflictAllResolved")}
              </p>
            )}
          </div>
          {conflicts.files.length > 0 && (
            <Button
              size="sm"
              variant="outline"
              disabled={resolvingConflicts}
              onClick={() => {
                const state = conflicts;
                setResolvingConflicts(true);
                setActionError(null);
                onResolveConflicts(state)
                  .catch((e) => setActionError(String(e)))
                  .finally(() => setResolvingConflicts(false));
              }}
              className="h-6 shrink-0 gap-1.5 px-2 text-[11px]"
            >
              <ChatCircleText size={12} />
              {resolvingConflicts
                ? t("conflictResolveSending")
                : t("conflictResolve")}
            </Button>
          )}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <aside
          className={`w-64 shrink-0 flex-col overflow-y-auto border-r border-tyba-border px-2 py-2 ${
            expanded ? "flex" : "hidden xl:flex"
          }`}
        >
          {diff && diff.commits.length > 0 && (
            <div className="flex flex-col gap-px">
              <span className="tyba-label px-2 pb-1 pt-1">
                {t("diffCommits")} · {diff.commits.length}
              </span>
              {diff.commits.map((c) => (
                <div key={c.sha} className="flex items-center gap-2 px-2 py-1">
                  <GitCommit
                    size={12}
                    className="shrink-0 text-tyba-text-faint"
                  />
                  <span className="shrink-0 font-mono text-[10px] text-tyba-text-faint">
                    {c.sha.slice(0, 7)}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-muted">
                    {c.subject}
                  </span>
                </div>
              ))}
            </div>
          )}
          {diff && sidebarSection("committed", diff.files)}
          {diff && sidebarSection("staged", diff.staged_files)}
          {diff && sidebarSection("unstaged", diff.unstaged_files)}
        </aside>

        <main className="relative min-w-0 flex-1">
          {stickyFile && (
            <button
              onClick={() => jumpTo(stickyFile.key)}
              className="absolute inset-x-0 top-0 z-10 flex items-center gap-2 border-b border-tyba-border bg-tyba-bg/95 px-4 py-1.5 text-left backdrop-blur"
            >
              <span
                className={`w-3 shrink-0 text-center font-mono text-[10px] ${STATUS_COLOR[stickyFile.file.status]}`}
              >
                {STATUS_LETTER[stickyFile.file.status]}
              </span>
              <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-tyba-text-muted">
                {stickyFile.file.path}
              </span>
            </button>
          )}
          <div ref={scrollRef} className="h-full overflow-y-auto">
            {loadError && (
              <div className="p-6 text-[12px] text-tyba-red">{loadError}</div>
            )}
            {!diff && !loadError && (
              <div className="p-6 font-mono text-[12px] text-tyba-text-faint">
                {t("diffLoading")}
              </div>
            )}
            {diff &&
              diff.files.length === 0 &&
              stagedCount === 0 &&
              unstagedCount === 0 && (
                <div className="flex h-full items-center justify-center text-[12px] text-tyba-text-faint">
                  {t("diffEmpty")}
                </div>
              )}
            <div
              style={{
                height: virtualizer.getTotalSize(),
                position: "relative",
              }}
            >
              {virtualItems.map((item) => (
                <div
                  key={item.key}
                  data-index={item.index}
                  ref={virtualizer.measureElement}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${item.start}px)`,
                  }}
                >
                  {renderRow(displayRows[item.index])}
                </div>
              ))}
            </div>
          </div>
        </main>
      </div>

      {stagedCount + unstagedCount > 0 && (
        <div className="flex h-10 shrink-0 items-center gap-2 border-t border-tyba-border px-4">
          <GitCommit size={13} className="shrink-0 text-tyba-text-faint" />
          <input
            value={commitMsg}
            onChange={(e) => setCommitMsg(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                (e.target as HTMLInputElement).blur();
                return;
              }
              if (
                e.key === "Enter" &&
                commitMsg.trim() &&
                stagedCount > 0 &&
                busy === null
              ) {
                void gitAction(
                  () => worktreeCommit(session.id, commitMsg),
                  "commit",
                ).then((ok) => {
                  if (ok) setCommitMsg("");
                });
              }
            }}
            placeholder={
              stagedCount > 0
                ? t("diffCommitMsgPlaceholder")
                : t("diffCommitStageFirst")
            }
            className="h-7 min-w-0 flex-1 rounded-[4px] border border-tyba-border bg-black/20 px-2 font-mono text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
          />
          <Button
            size="sm"
            className="h-7 gap-1.5 px-2.5 text-[11px]"
            disabled={!commitMsg.trim() || stagedCount === 0 || busy !== null}
            title={stagedCount === 0 ? t("diffCommitStageFirst") : undefined}
            onClick={() =>
              void gitAction(
                () => worktreeCommit(session.id, commitMsg),
                "commit",
              ).then((ok) => {
                if (ok) setCommitMsg("");
              })
            }
          >
            {busy === "commit"
              ? t("diffCommitting")
              : t("diffCommitAction", { count: stagedCount })}
          </Button>
        </div>
      )}

      {browseBranch === null && (
      <div className="max-h-[45%] shrink-0 overflow-y-auto">
        <DeliverySection
          session={session}
          hasUncommittedWork={
            (diff?.staged_files.length ?? 0) > 0 ||
            (diff?.unstaged_files.length ?? 0) > 0
          }
          aheadCount={diff?.commits.length ?? 0}
          isWorktree={Boolean(session.worktree)}
          onSendToAgent={onSendToAgent}
          onClose={onClose}
        />
      </div>
      )}

      <footer className="flex h-9 shrink-0 items-center gap-3 border-t border-tyba-border px-4 font-mono text-[11px] text-tyba-text-muted">
        <GitBranch size={12} className="shrink-0" />
        <span>
          {browseBranch
            ? t("branchExploring", { branch: browseBranch })
            : t("diffAhead", { count: diff?.commits.length ?? 0 })}
        </span>
        <span className="text-tyba-text-faint">·</span>
        <span className={stagedCount + unstagedCount > 0 ? "text-tyba-amber" : ""}>
          {browseBranch
            ? t("diffAhead", { count: diff?.commits.length ?? 0 })
            : stagedCount + unstagedCount > 0
              ? t("diffDirty")
              : t("diffClean")}
        </span>
        <div className="flex-1" />
        {actionError && (
          <span className="max-w-md truncate text-tyba-red" title={actionError}>
            {actionError}
          </span>
        )}
        {pushedInfo !== null && !actionError && (
          <span className="max-w-md truncate text-tyba-green">
            {t("diffPushed")}
          </span>
        )}
        {sendState === "sent" && commentList.length === 0 && (
          <span className="text-tyba-green">{t("diffCommentsSent")}</span>
        )}
        {sendError && <span className="max-w-md truncate text-tyba-red">{sendError}</span>}
        {commentList.length > 0 && (
          <Button
            size="sm"
            className="h-6 gap-1.5 px-2.5 text-[11px]"
            disabled={sendState === "sending"}
            onClick={() => void sendToAgent()}
          >
            <PaperPlaneTilt size={12} />
            {sendState === "sending"
              ? t("diffCommentsSending")
              : t("diffCommentsSend", { count: commentList.length })}
          </Button>
        )}
        {browseBranch === null && (diff?.commits.length ?? 0) > 0 && (
          <Button
            variant="outline"
            size="sm"
            className="h-6 gap-1.5 px-2.5 text-[11px]"
            disabled={busy !== null}
            onClick={() =>
              void gitAction(async () => {
                const info = await worktreePush(session.id);
                setPushedInfo(info);
                window.setTimeout(
                  () => setPushedInfo((cur) => (cur === info ? null : cur)),
                  5000,
                );
              }, "push")
            }
          >
            <UploadSimple size={12} />
            {busy === "push" ? t("diffPushing") : t("diffPush")}
          </Button>
        )}
      </footer>
    </div>
  );
}
