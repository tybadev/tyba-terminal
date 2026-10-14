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
  Check,
  CircleNotch,
  GitBranch,
  GitDiff,
  GitCommit,
  Minus,
  Sparkle,
  Warning,
  Plus,
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
  sessionConflictChoose,
  sessionConflictMarkResolved,
  sessionConflicts,
  sessionDiff,
  sessionDiffHunks,
  suggestCommitMessage,
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
  fileSignature,
  isSectionOpen,
  keyPath,
  keyScope,
  langOfPath,
  reviewProgress,
  toggleSection,
  viewedNow,
  type DiffMode,
  type DiffScopeKey,
  type ReviewComment,
  type Row,
  type SectionOpenState,
} from "../lib/diff";
import { requestConfirm } from "../lib/confirm";
import { openEditorTab } from "../lib/contentTabs";
import { translateError } from "../lib/errors";
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
  unmerged: "U",
  other: "?",
};

const STATUS_COLOR: Record<FileDiff["status"], string> = {
  added: "text-tyba-green",
  modified: "text-tyba-amber",
  deleted: "text-tyba-red",
  renamed: "text-tyba-violet",
  copied: "text-tyba-violet",
  type_changed: "text-tyba-text-muted",
  unmerged: "text-tyba-red",
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

const CONFLICT_RESOLVED_KEY: Record<ConflictState["operation"], string> = {
  merge: "conflictMergeResolved",
  rebase: "conflictRebaseResolved",
  cherry_pick: "conflictCherryPickResolved",
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

// Mockup Diff: fundo e texto tingido vêm dos tokens `--tyba-diff-*`
// (literais do mockup no TYBA escuro e claro; fórmula nos outros temas).
const LINE_BG: Record<string, string> = {
  add: "bg-tyba-diff-add-bg text-tyba-diff-add-text",
  del: "bg-tyba-diff-del-bg text-tyba-diff-del-text",
  context: "text-tyba-text/90",
};

const GUTTER_SIGN: Record<string, { sign: string; cls: string }> = {
  add: { sign: "+", cls: "text-tyba-green" },
  del: { sign: "−", cls: "text-tyba-red" },
  context: { sign: " ", cls: "" },
};

interface Props {
  session: Session;
  editor: string;
  suggestAgent: string;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
  onSendToAgent: (prompt: string) => Promise<void>;
  onResolveConflicts: (state: ConflictState) => Promise<void>;
  /** Aberto como aba (mockup Diff): o rótulo da aba já diz sessão e branch,
   *  e fechar é na aba — some o título, expandir e fechar do cabeçalho. */
  asTab?: boolean;
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
  suggestAgent,
  expanded,
  onToggleExpand,
  onClose,
  onSendToAgent,
  onResolveConflicts,
  asTab = false,
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
  const [conflictBusy, setConflictBusy] = useState<string | null>(null);
  const [suggesting, setSuggesting] = useState(false);
  // "Visto" por arquivo (chave escopo+caminho → assinatura do arquivo quando
  // foi marcado), só desta montagem. Se o arquivo muda, deixa de contar.
  const [viewedMarks, setViewedMarks] = useState<Map<string, string>>(() => new Map());
  // Mockup Diff: na tela de Git (asTab) mostra um arquivo por vez.
  const single = asTab;
  const [focusKey, setFocusKey] = useState<string | null>(null);
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
        // Um arquivo por vez: só o arquivo em foco precisa dos trechos.
        if (single) {
          if (key === focusKey) fetchHunks(scope, file);
          continue;
        }
        const collapsed =
          collapsedByKey[key] ??
          defaultCollapsed(file, hunksByKey[key], manyFiles);
        if (!collapsed) fetchHunks(scope, file);
      }
    }
  }, [diff, collapsedByKey, hunksByKey, fetchHunks, single, focusKey]);

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
            collapsedByKey:
              single && focusKey ? { ...collapsedByKey, [focusKey]: false } : collapsedByKey,
            mode,
          })
        : [],
    [diff, hunksByKey, collapsedByKey, mode, single, focusKey],
  );

  const displayRows = useMemo<DisplayRow[]>(() => {
    const out: DisplayRow[] = [];
    // Mockup Diff: sem título acima do código; com uma seção só, o título da
    // coluna basta.
    const manySections = rows.filter((r) => r.kind === "section").length > 1;
    // Um arquivo por vez: só as linhas do arquivo em foco (até o próximo).
    let inFocus = !single;
    for (const row of rows) {
      if (row.kind === "section" && (single || !manySections)) continue;
      if (single && row.kind === "file") inFocus = row.key === focusKey;
      if (!inFocus) continue;
      out.push(row);
      if (row.kind !== "line") continue;
      const saved = comments[row.key];
      if (saved && draftAnchor !== row.key) {
        out.push({
          kind: "comment",
          key: `${row.key}:c`,
          comment: saved,
          anchor: row.key,
        });
      }
      if (draftAnchor === row.key) {
        out.push({ kind: "commentEditor", key: `${row.key}:e`, anchor: row.key });
      }
    }
    return out;
  }, [rows, comments, draftAnchor, single, focusKey]);

  // Foco some (arquivo commitado, descartado) ou ainda não existe → o primeiro.
  useEffect(() => {
    if (!single) return;
    const files = rows.filter((r) => r.kind === "file");
    if (!files.some((r) => r.kind === "file" && r.key === focusKey)) {
      const first = files[0];
      setFocusKey(first && first.kind === "file" ? first.key : null);
    }
  }, [single, rows, focusKey]);

  // Um arquivo por vez: trocar de arquivo começa do topo dele.
  useEffect(() => {
    if (single) scrollRef.current?.scrollTo({ top: 0 });
  }, [single, focusKey]);

  const virtualizer = useVirtualizer({
    count: displayRows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const kind = displayRows[index]?.kind;
      if (kind === "line" || kind === "split") return 21;
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

  const viewedScopes = useMemo(
    () =>
      diff
        ? { committed: diff.files, staged: diff.staged_files, unstaged: diff.unstaged_files }
        : {},
    [diff],
  );
  const viewed = useMemo(() => viewedNow(viewedMarks, viewedScopes), [viewedMarks, viewedScopes]);
  const progress = reviewProgress(viewedScopes, viewed);
  const totals = useMemo(() => {
    const all = diff ? [...diff.files, ...diff.staged_files, ...diff.unstaged_files] : [];
    return {
      files: new Set(all.map((f) => f.path)).size,
      insertions: all.reduce((n, f) => n + (f.insertions ?? 0), 0),
      deletions: all.reduce((n, f) => n + (f.deletions ?? 0), 0),
    };
  }, [diff]);
  const commentsByPath = useMemo(() => {
    const out = new Map<string, number>();
    for (const c of commentList) out.set(c.path, (out.get(c.path) ?? 0) + 1);
    return out;
  }, [commentList]);

  // Marcar visto recolhe o arquivo; desmarcar abre de novo.
  const toggleViewed = (key: string) => {
    const next = !viewed.has(key);
    const file = (viewedScopes[keyScope(key) as DiffScopeKey] ?? []).find(
      (f) => f.path === keyPath(key),
    );
    setViewedMarks((prev) => {
      const out = new Map(prev);
      if (next && file) out.set(key, fileSignature(file));
      else out.delete(key);
      return out;
    });
    if (!single) {
      toggleFile(key, next);
      return;
    }
    if (next) {
      const keys = rows.flatMap((r) => (r.kind === "file" ? [r.key] : []));
      const after = [...keys.slice(keys.indexOf(key) + 1), ...keys.slice(0, keys.indexOf(key))];
      const nextKey = after.find((k) => !viewed.has(k));
      if (nextKey) setFocusKey(nextKey);
    }
  };

  const openInEditor = (file: FileDiff) =>
    // Abre como aba de editor do TYBA; o caminho do diff é do repo
    // e o core o resolve contra a raiz da sessão.
    void openEditorTab(file.path, undefined, {
      session: session.id,
    }).catch((e) => setActionError(String(e)));

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
        {/* Mockup Diff: as ações em lote moram no título da seção na coluna. */}
        <div className="flex items-center justify-between gap-2 pb-1 pl-2 pr-2.5 pt-3">
          <button
            onClick={() => setOpenSections((prev) => toggleSection(prev, scope))}
            aria-expanded={open}
            className="tyba-label flex items-center gap-1 hover:text-tyba-text-muted"
          >
            {open ? (
              <CaretDown size={9} className="shrink-0" />
            ) : (
              <CaretRight size={9} className="shrink-0" />
            )}
            {t(SECTION_LABEL_KEY[scope])} · {files.length}
          </button>
          {sectionActions(scope)}
        </div>
        {open &&
          files.map((file) => {
            const key = fileKeyOf(scope, file.path);
            const active = single ? focusKey === key : stickyFile?.key === key;
            const seen = viewed.has(key);
            const notes = commentsByPath.get(file.path) ?? 0;
            return (
              <button
                key={key}
                onClick={() => (single ? setFocusKey(key) : jumpTo(key))}
                className={`flex items-center gap-2 rounded-[6px] px-2.5 py-1.5 text-left font-mono text-[12.5px] transition-colors ${
                  active
                    ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]"
                    : seen
                      ? "text-tyba-text-faint hover:bg-tyba-text/[.03]"
                      : "text-tyba-text-muted hover:bg-tyba-text/[.03] hover:text-tyba-text"
                }`}
              >
                <span className={`shrink-0 ${STATUS_COLOR[file.status]}`}>
                  {STATUS_LETTER[file.status]}
                </span>
                <span className="min-w-0 flex-1 truncate" title={file.path}>
                  {file.path}
                  {seen && " ✓"}
                </span>
                {notes > 0 ? (
                  <span className="shrink-0 text-[11px] text-tyba-violet">
                    {t("diffCommentCount", { count: notes })}
                  </span>
                ) : (
                  <Numstat file={file} />
                )}
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
          // Mockup Diff: cartão violeta (borda 0,35, fundo 0,06, cantos de 8px)
          // recuado até a coluna do código.
          <div className="py-1.5 pl-24 pr-4">
          <div className="flex flex-col gap-2 rounded-[8px] border border-tyba-violet/35 bg-tyba-violet/[.06] px-3 py-2.5 text-[12px]">
            <span className="text-[color-mix(in_srgb,var(--tyba-violet)_60%,var(--tyba-text))]">
              {t("diffCommentYours", {
                line: row.comment.newNo ?? row.comment.oldNo ?? "",
              })}
            </span>
            <span className="whitespace-pre-wrap leading-snug text-tyba-text">
              {row.comment.text}
            </span>
            <span className="flex flex-wrap items-center gap-3 text-[11px] text-tyba-text-muted">
              <button
                onClick={() => {
                  setDraftAnchor(row.anchor);
                  setDraftText(row.comment.text);
                }}
                className="hover:text-tyba-text"
              >
                {t("diffCommentEdit")}
              </button>
              <button
                aria-label={t("diffCommentRemove")}
                onClick={() =>
                  setComments((prev) => {
                    const next = { ...prev };
                    delete next[row.anchor];
                    return next;
                  })
                }
                className="hover:text-tyba-red"
              >
                {t("diffCommentDelete")}
              </button>
              <span className="text-tyba-text-faint">{t("diffCommentPending")}</span>
            </span>
          </div>
          </div>
        );
      case "commentEditor":
        return (
          <div className="py-1.5 pl-24 pr-4">
          <div className="flex flex-col gap-1.5 rounded-[8px] border border-tyba-violet/35 bg-tyba-violet/[.06] px-3 py-2.5">
            <textarea
              autoCapitalize="sentences"
              autoCorrect="on"
              spellCheck
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
              className="min-h-[52px] w-full max-w-xl resize-y rounded-[6px] border border-tyba-border-field bg-tyba-sunken p-2 font-sans text-[12px] text-tyba-text outline-none focus:border-tyba-violet/50"
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
          </div>
        );
      case "section":
        // Só existe com mais de uma seção (ver displayRows): rótulo fino que
        // separa as seções; as ações ficam na coluna.
        return (
          <div className="tyba-label px-4 pb-1.5 pt-4">
            {t(SECTION_LABEL_KEY[row.scope])} · {row.count}
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
            className="group/file flex w-full cursor-pointer items-center gap-2 tyba-divide-t bg-tyba-surface/60 px-4 py-2 text-left hover:bg-tyba-text/[.03]"
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
            <span className="min-w-0 truncate font-mono text-[12px] text-tyba-text">
              {file.old_path ? `${file.old_path} → ${file.path}` : file.path}
            </span>
            <Numstat file={file} />
            {row.generated && (
              <span className="shrink-0 rounded-full bg-tyba-text/[.06] px-1.5 text-[9px] text-tyba-text-faint">
                {t("diffGenerated")}
              </span>
            )}
            <span className="flex-1" />
            {fileActions(row.scope, file)}
            <span
              className="flex shrink-0 items-center gap-3.5 text-[11px] text-tyba-text-muted"
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => e.stopPropagation()}
            >
              {file.status !== "deleted" && (
                <button
                  onClick={() => openInEditor(file)}
                  className="hover:text-tyba-text"
                >
                  {t("diffOpenInEditor")}
                </button>
              )}
              <label className="flex cursor-pointer items-center gap-1.5 hover:text-tyba-text">
                <input
                  type="checkbox"
                  checked={viewed.has(row.key)}
                  onChange={() => toggleViewed(row.key)}
                  className="accent-tyba-green"
                />
                {t("diffViewed")}
              </label>
            </span>
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
            className={`group/line flex items-stretch font-mono text-[12.5px] leading-[21px] ${LINE_BG[row.line.kind]}`}
          >
            <span className="relative w-10 shrink-0 select-none pr-2.5 text-right text-[12px] leading-[21px] text-tyba-text-ghost">
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
            <span className="w-10 shrink-0 select-none pr-2.5 text-right text-[12px] leading-[21px] text-tyba-text-ghost">
              {row.newNo ?? ""}
            </span>
            <span
              className={`w-5 shrink-0 select-none text-center ${gutter.cls}`}
            >
              {gutter.sign}
            </span>
            <span className="min-w-0 flex-1 whitespace-pre pr-4">
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
              <span className="w-10 shrink-0 select-none pr-2.5 text-right text-[12px] leading-[21px] text-tyba-text-ghost">
                {entry.no}
              </span>
              <span
                className={`w-4 shrink-0 select-none text-center ${gutter.cls}`}
              >
                {gutter.sign}
              </span>
              <span className="min-w-0 flex-1 truncate whitespace-pre pr-3">
                <LineText text={entry.text} spans={spans} />
              </span>
              {signKind === "left" && (
                <span className="w-px shrink-0 bg-tyba-border" />
              )}
            </div>
          );
        };
        return (
          <div className="flex font-mono text-[12.5px] leading-[21px] text-tyba-text/90">
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
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-tyba-sunken"
    >
      {/* Mockup Diff: faixa de 40px na superfície (vidro no claro), 12px. */}
      <header className="flex h-10 shrink-0 items-center gap-3.5 tyba-divide-b bg-[var(--tyba-panel-bg)] px-4 text-[12px] text-tyba-text-muted">
        {!asTab && (
          <span className="flex min-w-0 shrink items-center gap-2">
            <GitDiff size={13} className="shrink-0 text-tyba-violet" />
            <span className="min-w-0 truncate text-tyba-text">{session.title}</span>
            <span className="truncate font-mono text-[11px]">{branch}</span>
          </span>
        )}
        <span className="flex min-w-0 shrink items-center gap-1.5 whitespace-nowrap">
          {/* Sem worktree não há base congelada: o diff é contra o HEAD. */}
          {!browseBranch && (
            <span className="shrink-0">
              {t(session.worktree ? "diffComparingBase" : "diffComparingHead")}
            </span>
          )}
          <BranchPicker
            sessionId={session.id}
            browsing={browseBranch}
            label={browseBranch ?? baseShort}
            onBrowse={browseTo}
            variant="base"
          />
          {!browseBranch && session.worktree && (
            <span className="min-w-0 truncate text-tyba-text-faint">
              {t("diffBaseFrozen")}
            </span>
          )}
        </span>
        <div className="flex-1" />
        {diff && totals.files > 0 && (
          <span className="shrink-0 whitespace-nowrap">
            {t("diffFilesSummary", { count: totals.files })} ·{" "}
            <span className="text-tyba-green">+{totals.insertions}</span>{" "}
            <span className="text-tyba-red">−{totals.deletions}</span>
          </span>
        )}
        <div className="flex items-center gap-0.5 rounded-[6px] bg-tyba-sunken p-0.5">
          <button
            aria-pressed={mode === "unified"}
            onClick={() => setMode("unified")}
            className={`flex h-5 items-center rounded-[4px] px-2 text-[12px] ${
              mode === "unified"
                ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]"
                : "text-tyba-text-muted hover:text-tyba-text"
            }`}
          >
            {t("diffUnified")}
          </button>
          <button
            aria-pressed={mode === "split"}
            onClick={() => setMode("split")}
            className={`flex h-5 items-center rounded-[4px] px-2 text-[12px] ${
              mode === "split"
                ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]"
                : "text-tyba-text-muted hover:text-tyba-text"
            }`}
          >
            {t("diffSplit")}
          </button>
        </div>
        {!asTab && (
          <>
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
          </>
        )}
      </header>

      {conflicts && !browseBranch && (
        <div
          className={`flex shrink-0 items-start gap-2 tyba-divide-b px-3 py-2 ${
            conflicts.files.length > 0 ? "bg-tyba-red/[.06]" : "bg-tyba-amber/[.06]"
          }`}
        >
          <Warning
            size={14}
            className={`mt-0.5 shrink-0 ${
              conflicts.files.length > 0 ? "text-tyba-red" : "text-tyba-amber"
            }`}
          />
          <div className="min-w-0 flex-1">
            <p className="text-[12px] text-tyba-text">
              {t(
                conflicts.files.length > 0
                  ? CONFLICT_OPERATION_KEY[conflicts.operation]
                  : CONFLICT_RESOLVED_KEY[conflicts.operation],
              )}
              {conflicts.ours && conflicts.theirs && (
                <span className="font-mono text-[11px] text-tyba-text-muted">
                  {" "}
                  · {conflicts.ours} ← {conflicts.theirs}
                </span>
              )}
            </p>
            {conflicts.files.length > 0 ? (
              conflicts.files.map((f) => {
                const busyRow = conflictBusy === f.path;
                const act = (run: () => Promise<void>) => {
                  if (conflictBusy) return;
                  setConflictBusy(f.path);
                  setActionError(null);
                  run()
                    .then(() => refresh())
                    .catch((e) => setActionError(translateError(e, t)))
                    .finally(() => setConflictBusy(null));
                };
                const chip =
                  "shrink-0 rounded-[3px] border border-tyba-border px-1.5 py-0.5 text-[10px] text-tyba-text-muted hover:bg-tyba-text/[.06] hover:text-tyba-text disabled:opacity-50";
                return (
                  <div key={f.path} className="flex items-center gap-1.5 pt-1">
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-tyba-red/90">
                      {f.path}
                      <span className="pl-1.5 text-[9px] text-tyba-text-faint">
                        {f.kind}
                      </span>
                    </span>
                    {busyRow ? (
                      <CircleNotch
                        size={12}
                        className="shrink-0 animate-spin text-tyba-text-faint"
                      />
                    ) : (
                      <>
                        <button
                          disabled={conflictBusy !== null}
                          onClick={() =>
                            act(() =>
                              sessionConflictChoose(session.id, f.path, "ours"),
                            )
                          }
                          className={chip}
                        >
                          {t("conflictKeepOurs", {
                            branch: conflicts.ours ?? "ours",
                          })}
                        </button>
                        <button
                          disabled={conflictBusy !== null}
                          onClick={() =>
                            act(() =>
                              sessionConflictChoose(
                                session.id,
                                f.path,
                                "theirs",
                              ),
                            )
                          }
                          className={chip}
                        >
                          {t("conflictTakeTheirs", {
                            branch: conflicts.theirs ?? "theirs",
                          })}
                        </button>
                        <button
                          title={t("conflictOpenEditor")}
                          disabled={conflictBusy !== null}
                          onClick={() =>
                            void openWorktreeFile(
                              session.id,
                              f.path,
                              editor || undefined,
                            ).catch((e) => setActionError(String(e)))
                          }
                          className={chip}
                        >
                          <ArrowSquareOut size={11} />
                        </button>
                        <button
                          title={t("conflictMarkResolved")}
                          disabled={conflictBusy !== null}
                          onClick={() =>
                            act(() =>
                              sessionConflictMarkResolved(session.id, f.path),
                            )
                          }
                          className={chip}
                        >
                          <Check size={11} />
                        </button>
                      </>
                    )}
                  </div>
                );
              })
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
          className={`w-[260px] shrink-0 flex-col overflow-y-auto tyba-divide-r p-2.5 ${
            expanded ? "flex" : "hidden xl:flex"
          }`}
        >
          {progress.total > 0 && (
            <span className="px-2.5 pb-2 pt-0.5 text-[11px] text-tyba-text-faint">
              {t("diffViewedCount", progress)}
            </span>
          )}
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
              {diff.commits_truncated && (
                <span className="px-2 py-1 text-[11px] text-tyba-text-faint">
                  {t("diffCommitsMore")}
                </span>
              )}
            </div>
          )}
          {diff && sidebarSection("committed", diff.files)}
          {diff && sidebarSection("staged", diff.staged_files)}
          {diff && sidebarSection("unstaged", diff.unstaged_files)}
        </aside>

        <main className="relative min-w-0 flex-1">
          {stickyFile && !single && (
            <button
              onClick={() => jumpTo(stickyFile.key)}
              className="absolute inset-x-0 top-0 z-10 flex items-center gap-2 tyba-divide-b bg-tyba-bg/95 px-4 py-1.5 text-left backdrop-blur"
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
        <div className="flex h-8 shrink-0 items-center gap-2 tyba-divide-t px-4">
          <GitCommit size={13} className="shrink-0 text-tyba-text-faint" />
          <input
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
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
            className="h-7 min-w-0 flex-1 rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 font-mono text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
          />
          <Button
            variant="ghost"
            size="icon"
            aria-label={t("commitSuggest")}
            title={t("commitSuggest")}
            disabled={stagedCount === 0 || suggesting || busy !== null}
            onClick={() => {
              setSuggesting(true);
              setActionError(null);
              suggestCommitMessage(session.id, suggestAgent)
                .then(setCommitMsg)
                .catch((e) => setActionError(translateError(e, t)))
                .finally(() => setSuggesting(false));
            }}
            className="size-6 shrink-0"
          >
            {suggesting ? (
              <CircleNotch size={13} className="animate-spin" />
            ) : (
              <Sparkle size={13} />
            )}
          </Button>
          <Button
            size="sm"
            className="h-6 gap-1.5 px-2.5 text-[11px]"
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

      {(() => {
        const ahead = diff?.commits.length ?? 0;
        const dirty = stagedCount + unstagedCount > 0;
        // Esquerda da barra: com comentário, o envio ao agente (mockup);
        // sem, o estado do worktree. Mensagens de ação ficam junto.
        const leading = (
          <>
            {commentList.length > 0 ? (
              <>
                <Button
                  size="sm"
                  className="h-auto shrink-0 gap-1.5 rounded-[6px] bg-tyba-violet px-3.5 py-1.5 text-[12px] font-normal text-white hover:bg-tyba-violet/90"
                  disabled={sendState === "sending"}
                  onClick={() => void sendToAgent()}
                >
                  {sendState === "sending"
                    ? t("diffCommentsSending")
                    : t("diffCommentsSend", { count: commentList.length })}
                </Button>
                <span className="truncate text-[11px] text-tyba-text-faint">
                  {t("diffSendHint", { agent: suggestAgent })}
                </span>
              </>
            ) : (
              <span className="flex min-w-0 items-center gap-2 truncate font-mono text-[11px] text-tyba-text-muted">
                <GitBranch size={12} className="shrink-0" />
                {browseBranch
                  ? t("branchExploring", { branch: browseBranch })
                  : t("diffAhead", { count: ahead })}
                <span className="text-tyba-text-faint">·</span>
                <span className={dirty ? "text-tyba-amber" : ""}>
                  {browseBranch
                    ? t("diffAhead", { count: ahead })
                    : dirty
                      ? t("diffDirty")
                      : t("diffClean")}
                </span>
              </span>
            )}
            {actionError && (
              <span className="min-w-0 truncate text-[11px] text-tyba-red" title={actionError}>
                {actionError}
              </span>
            )}
            {pushedInfo !== null && !actionError && (
              <span className="truncate text-[11px] text-tyba-green">{t("diffPushed")}</span>
            )}
            {sendState === "sent" && commentList.length === 0 && (
              <span className="text-[11px] text-tyba-green">{t("diffCommentsSent")}</span>
            )}
            {sendError && (
              <span className="min-w-0 truncate text-[11px] text-tyba-red">{sendError}</span>
            )}
          </>
        );
        const push = browseBranch === null && ahead > 0 && (
          <Button
            variant="ghost"
            size="sm"
            className="h-auto shrink-0 gap-1.5 rounded-[6px] border border-tyba-border-strong bg-transparent px-3 py-[5px] text-[12px] font-normal text-tyba-text-muted hover:bg-transparent hover:text-tyba-text"
            disabled={busy !== null}
            onClick={async () => {
              // Push é vermelho: confirmação toda vez, como na tela de Git.
              const ok = await requestConfirm({
                title: t("gitScreenPushConfirm", { branch: branch || "HEAD" }),
                detail: t("gitScreenPushConfirmDetail"),
                confirmLabel: t("gitScreenPush"),
              });
              if (!ok) return;
              void gitAction(async () => {
                const info = await worktreePush(session.id);
                setPushedInfo(info);
                window.setTimeout(
                  () => setPushedInfo((cur) => (cur === info ? null : cur)),
                  5000,
                );
              }, "push");
            }}
          >
            <UploadSimple size={12} />
            {busy === "push" ? t("diffPushing") : t("diffPush")}
          </Button>
        );
        return browseBranch === null ? (
          <div className="max-h-[45%] shrink-0 overflow-y-auto">
            <DeliverySection
              session={session}
              hasUncommittedWork={dirty}
              aheadCount={ahead}
              isWorktree={Boolean(session.worktree)}
              onSendToAgent={onSendToAgent}
              onResolveConflicts={onResolveConflicts}
              onClose={onClose}
              leading={leading}
              trailing={push}
            />
          </div>
        ) : (
          <div className="flex shrink-0 items-center gap-2.5 tyba-divide-t bg-[var(--tyba-panel-bg)] px-4 py-3">
            {leading}
          </div>
        );
      })()}
    </div>
  );
}
