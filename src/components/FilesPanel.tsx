import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ArrowClockwise,
  ArrowsInSimple,
  ArrowsOutSimple,
  CaretDown,
  CaretRight,
  Copy,
  Crosshair,
  DownloadSimple,
  FilePlus,
  Folder,
  FolderOpen,
  FolderSimplePlus,
  HardDrives,
  PencilSimple,
  PlusMinus,
  ArrowSquareOut,
  SealCheck,
  Terminal,
  TrashSimple,
  TreeView,
  Warning,
  WarningCircle,
  X,
} from "@phosphor-icons/react";

import type {
  EditContent,
  FileContent,
  FileDecoStatus,
  FileEntry,
  FilesPanelInfo,
  GutterKind,
  GutterMarker,
  LspDiagnostic,
  LspLocation,
  LspManagedProgress,
  LspStatus,
  Session,
} from "@/lib/ipc";
import {
  filesCreate,
  filesDecorations,
  filesDelete,
  filesEditBegin,
  filesEditEnd,
  filesFocus,
  filesGutter,
  filesListDir,
  filesOpenExternal,
  filesPanelInfo,
  filesRead,
  filesReanchor,
  filesRefresh,
  filesRename,
  filesUnwatchDir,
  filesWatchDir,
  filesWrite,
  lspChange,
  lspCloseDoc,
  lspCompletion,
  lspDefinition,
  lspDidSave,
  lspHover,
  lspManagedConsent,
  lspManagedDownload,
  lspManagedUseMine,
  lspOpen,
  lspOpenExternal,
  lspRetry,
  lspSignature,
  lspStatus,
  onFilesConflict,
  onFilesDecorations,
  onFilesGutter,
  onFilesTree,
  onSessionCwd,
  onLspDiagnostics,
  onLspManagedError,
  onLspManagedProgress,
} from "@/lib/ipc";
import type { LspBridge } from "@/lib/cmLsp";
import { fileIcon } from "@/lib/fileIcon";
import {
  highlightBlock,
  langForFence,
  langForFile,
  type TokenSpan,
} from "@/lib/highlight";
import { isRemoteUrl, safeMarkdownUrl } from "@/lib/markdownUrl";
import { requestConfirm } from "@/lib/confirm";
import { IS_MAC } from "@/lib/platform";
import { lineDiff } from "@/lib/lineDiff";
import { toastError } from "@/lib/toast";
import { getEffectiveBase, onEffectiveBaseChange } from "@/theme";
import { CodeEditor, type CodeEditorHandle } from "./CodeEditor";

interface Props {
  session: Session;
  editor: string;
  expanded: boolean;
  openRequest: { path: string; nonce: number } | null;
  onToggleExpand: () => void;
  onClose: () => void;
  onJumpToDiff: () => void;
  onRunInTerminal: (command: string) => void;
}

type TreeEdit =
  | { kind: "create"; parentDir: string; isDir: boolean }
  | { kind: "rename"; rel: string; name: string };

const DECO_LABEL: Record<FileDecoStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  untracked: "?",
};

const DECO_CLASS: Record<FileDecoStatus, string> = {
  added: "text-tyba-green",
  modified: "text-tyba-amber",
  deleted: "text-tyba-red",
  renamed: "text-tyba-amber",
  untracked: "text-tyba-text-faint",
};

const GUTTER_CLASS: Record<GutterKind, string> = {
  added: "bg-tyba-green",
  modified: "bg-tyba-amber",
  deleted: "bg-tyba-red",
};

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function pct(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

function managedPhaseKey(phase: LspManagedProgress["phase"]): string {
  switch (phase) {
    case "verifying":
      return "lspManagedVerifying";
    case "extracting":
      return "lspManagedExtracting";
    case "error":
      return "lspManagedError";
    default:
      return "lspManagedPreparing";
  }
}

function parentOf(rel: string): string {
  const i = rel.lastIndexOf("/");
  return i < 0 ? "" : rel.slice(0, i);
}

function MarkdownCode({
  code,
  lang,
  dark,
}: {
  code: string;
  lang: string | null;
  dark: boolean;
}) {
  const [tokens, setTokens] = useState<TokenSpan[][] | null>(null);
  const body = code.replace(/\n+$/, "");
  useEffect(() => {
    setTokens(null);
    if (!lang) return;
    let alive = true;
    void highlightBlock(body.split("\n"), lang, dark ? "mono-dark" : "vitesse-dark")
      .then((r) => {
        if (alive) setTokens(r);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [body, lang, dark]);
  return (
    <pre className="files-md-pre">
      <code>
        {tokens
          ? tokens.map((line, i) => (
              <div key={i}>
                {line.length === 0
                  ? "\n"
                  : line.map((tk, j) => (
                      <span key={j} style={{ color: tk.color }}>
                        {tk.text}
                      </span>
                    ))}
              </div>
            ))
          : body}
      </code>
    </pre>
  );
}

export function FilesPanel({
  session,
  editor,
  expanded,
  openRequest,
  onToggleExpand,
  onClose,
  onJumpToDiff,
  onRunInTerminal,
}: Props) {
  const { t } = useTranslation();
  const [info, setInfo] = useState<FilesPanelInfo | null>(null);
  const [entriesByDir, setEntriesByDir] = useState<Record<string, FileEntry[]>>(
    {},
  );
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [decorations, setDecorations] = useState<Map<string, FileDecoStatus>>(
    new Map(),
  );
  const [selected, setSelected] = useState<string | null>(null);
  const [contextDir, setContextDir] = useState<string>("");
  const [content, setContent] = useState<FileContent | null>(null);
  const [contentError, setContentError] = useState<string | null>(null);
  const [markdownSource, setMarkdownSource] = useState(false);
  const [tokens, setTokens] = useState<TokenSpan[][] | null>(null);
  const [dirMeta, setDirMeta] = useState<
    Record<string, { total: number; truncated: boolean }>
  >({});
  const [isDark, setIsDark] = useState(() => getEffectiveBase() === "dark");
  const [gutter, setGutter] = useState<GutterMarker[]>([]);
  const [editing, setEditing] = useState(false);
  const [editBaseline, setEditBaseline] = useState<EditContent | null>(null);
  const [savedHash, setSavedHash] = useState<string>("");
  const [docVersion, setDocVersion] = useState(0);
  const [dirty, setDirty] = useState(false);
  const [conflict, setConflict] = useState<{
    path: string;
    diskHash: string | null;
  } | null>(null);
  const [conflictDiff, setConflictDiff] = useState<{
    disk: string;
    edited: string;
  } | null>(null);
  const [treeEdit, setTreeEdit] = useState<TreeEdit | null>(null);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [lspStat, setLspStat] = useState<LspStatus | null>(null);
  const [diagsByPath, setDiagsByPath] = useState<Map<string, LspDiagnostic[]>>(
    new Map(),
  );
  const [showInstall, setShowInstall] = useState(false);
  const [showManaged, setShowManaged] = useState(true);
  const [showAlts, setShowAlts] = useState(false);
  const [copied, setCopied] = useState(false);
  const [diagsTruncated, setDiagsTruncated] = useState(false);
  const lspTimer = useRef<number | undefined>(undefined);
  const editorRef = useRef<CodeEditorHandle>(null);

  // LSP é local-only (fatia 3): painel remoto (SSH) não sobe LSP — o LSP remoto
  // é a fatia 4b. `remote` guarda toda ativação e UI de LSP daqui pra baixo.
  const remote = info?.remote ?? false;
  const connectionDown = session.connection && session.connection !== "live";

  const currentDiagnostics = useMemo(
    () => (remote || !selected ? [] : diagsByPath.get(selected) ?? []),
    [remote, selected, diagsByPath],
  );

  useEffect(
    () => onEffectiveBaseChange(() => setIsDark(getEffectiveBase() === "dark")),
    [],
  );

  const entriesRef = useRef(entriesByDir);
  entriesRef.current = entriesByDir;
  const reqId = useRef(0);
  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selected;
  const editingRef = useRef(false);
  editingRef.current = editing;
  const dirtyRef = useRef(false);
  dirtyRef.current = dirty;

  const confirmDiscard = useCallback(async (): Promise<boolean> => {
    if (!editingRef.current || !dirtyRef.current) return true;
    return requestConfirm({
      title: t("filesDiscardTitle"),
      detail: t("filesDiscardDetail"),
      confirmLabel: t("filesDiscardConfirm"),
      destructive: true,
    });
  }, [t]);

  const relist = useCallback(
    (dir: string) => {
      void filesListDir(session.id, dir)
        .then((listing) => {
          setEntriesByDir((prev) => ({ ...prev, [dir]: listing.entries }));
          setDirMeta((prev) => ({
            ...prev,
            [dir]: { total: listing.total, truncated: listing.truncated },
          }));
        })
        .catch(() => {});
    },
    [session.id],
  );

  const refreshLspStatus = useCallback(
    (rel: string) => {
      // Painel remoto não tem LSP (local-only, fatia 3): sem status, o que já
      // esconde toda a UI de LSP (que depende de `lspStat`).
      if (remote) return;
      void lspStatus(session.id, rel)
        .then((s) => {
          if (selectedRef.current !== rel) return;
          setLspStat(s);
          if (
            s.state === "installing" &&
            s.progress.phase === "pending"
          ) {
            void lspManagedDownload(session.id, s.server_id, rel)
              .then((next) => {
                if (selectedRef.current === rel) setLspStat(next);
              })
              .catch(() => {});
          }
          if (
            s.state === "starting" ||
            s.state === "available" ||
            s.state === "installing"
          ) {
            window.clearTimeout(lspTimer.current);
            lspTimer.current = window.setTimeout(() => {
              if (selectedRef.current === rel) refreshLspStatus(rel);
            }, 1500);
          }
        })
        .catch(() => {});
    },
    [session.id, remote],
  );

  const handleRetry = useCallback(() => {
    const path = selectedRef.current;
    if (!path) return;
    void lspRetry(session.id, path)
      .then((s) => {
        if (selectedRef.current !== path) return;
        setLspStat(s);
        if (s.state === "starting" || s.state === "available") {
          refreshLspStatus(path);
        }
      })
      .catch(() => {});
  }, [session.id, refreshLspStatus]);

  const openFile = useCallback(
    (rel: string) => {
      const my = ++reqId.current;
      setSelected(rel);
      setContextDir(parentOf(rel));
      setContent(null);
      setContentError(null);
      setMarkdownSource(false);
      setTokens(null);
      setEditing(false);
      setEditBaseline(null);
      setDirty(false);
      setConflict(null);
      setConflictDiff(null);
      setGutter([]);
      setLspStat(null);
      setShowInstall(false);
      setShowManaged(true);
      window.clearTimeout(lspTimer.current);
      refreshLspStatus(rel);
      void filesFocus(session.id, rel)
        .then((markers) => {
          if (reqId.current === my) setGutter(markers);
        })
        .catch(() => {});
      void filesRead(session.id, rel, 0)
        .then((c) => {
          if (reqId.current === my) setContent(c);
        })
        .catch((e) => {
          if (reqId.current === my) setContentError(String(e));
        });
    },
    [session.id, refreshLspStatus],
  );

  const openFileGuarded = useCallback(
    async (rel: string) => {
      if (await confirmDiscard()) openFile(rel);
    },
    [confirmDiscard, openFile],
  );

  useEffect(() => {
    let alive = true;
    setEntriesByDir({});
    setDirMeta({});
    setExpandedDirs(new Set());
    setSelected(null);
    setContent(null);
    void filesPanelInfo(session.id)
      .then((i) => {
        if (alive) setInfo(i);
      })
      .catch(() => {});
    setBusy(true);
    void filesListDir(session.id, "")
      .then((listing) => {
        if (!alive) return;
        setEntriesByDir({ "": listing.entries });
        setDirMeta({ "": { total: listing.total, truncated: listing.truncated } });
      })
      .catch(() => {})
      .finally(() => {
        if (alive) setBusy(false);
      });
    void filesWatchDir(session.id, "").catch(() => {});
    void filesDecorations(session.id)
      .then((decos) => {
        if (alive) setDecorations(new Map(decos.map((d) => [d.path, d.status])));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [session.id]);

  useEffect(() => {
    const unTree = onFilesTree(session.id, (dirs) => {
      for (const dir of dirs) {
        if (dir === "" || dir in entriesRef.current) relist(dir);
      }
    });
    const unDeco = onFilesDecorations(session.id, (decos) => {
      setDecorations(new Map(decos.map((d) => [d.path, d.status])));
    });
    const unGutter = onFilesGutter(session.id, (path, markers) => {
      if (path === selectedRef.current) setGutter(markers);
    });
    const unConflict = onFilesConflict(session.id, (path, diskHash) => {
      if (editingRef.current && path === selectedRef.current) {
        setConflict({ path, diskHash });
      }
    });
    const unLsp = onLspDiagnostics(session.id, (files, filesTruncated) => {
      if (filesTruncated) setDiagsTruncated(true);
      setDiagsByPath((prev) => {
        const next = new Map(prev);
        for (const file of files) {
          if (file.diagnostics.length === 0) next.delete(file.path);
          else next.set(file.path, file.diagnostics);
        }
        return next;
      });
    });
    // A raiz não segue o `cd` — isso é decisão, não descuido. O que se faz aqui
    // é reperguntar ao core se ela ainda bate com o cwd vivo, porque é essa
    // medida que faz o botão de re-ancorar ganhar rótulo em vez de continuar um
    // ícone mudo que ninguém encontra.
    const unCwd = onSessionCwd(session.id, () => {
      void filesPanelInfo(session.id)
        .then((i) => setInfo(i))
        .catch(() => {});
    });
    return () => {
      void unTree.then((f) => f());
      void unDeco.then((f) => f());
      void unGutter.then((f) => f());
      void unConflict.then((f) => f());
      void unLsp.then((f) => f());
      void unCwd.then((f) => f());
    };
  }, [session.id, relist]);

  const reveal = useCallback(
    async (path: string) => {
      if (!(await confirmDiscard())) return;
      const parts = path.split("/");
      const dirs: string[] = [];
      let acc = "";
      for (let i = 0; i < parts.length - 1; i++) {
        acc = acc ? `${acc}/${parts[i]}` : parts[i];
        dirs.push(acc);
      }
      setExpandedDirs((prev) => {
        const next = new Set(prev);
        for (const dir of dirs) next.add(dir);
        return next;
      });
      for (const dir of dirs) {
        if (!(dir in entriesRef.current)) relist(dir);
        void filesWatchDir(session.id, dir).catch(() => {});
      }
      openFile(path);
    },
    [session.id, relist, openFile, confirmDiscard],
  );

  useEffect(() => {
    if (openRequest) void reveal(openRequest.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openRequest?.nonce]);

  const handleGotoDef = useCallback(
    async (loc: LspLocation) => {
      if (loc.in_root) {
        await reveal(loc.path);
        return;
      }
      const ok = await requestConfirm({
        title: t("lspGotoExternalTitle"),
        detail: loc.path,
        confirmLabel: t("lspGotoExternalOpen"),
      });
      if (ok) {
        void lspOpenExternal(loc.path, editor).catch((e) =>
          toastError(t("filesOpenExternal"), e),
        );
      }
    },
    [reveal, t, editor],
  );

  const lspBridge = useMemo<LspBridge | null>(() => {
    // Sem ponte de LSP no remoto: o editor não pede completion/hover/def nem
    // manda didChange (LSP local-only, fatia 3).
    if (!selected || remote) return null;
    const path = selected;
    return {
      completion: (line, character) =>
        lspCompletion(session.id, path, line, character),
      hover: (line, character) => lspHover(session.id, path, line, character),
      signature: (line, character) =>
        lspSignature(session.id, path, line, character),
      definition: (line, character) =>
        lspDefinition(session.id, path, line, character),
      change: (changes) => {
        void lspChange(session.id, path, changes).catch(() => {});
      },
      gotoDefinition: (loc) => void handleGotoDef(loc),
    };
  }, [session.id, selected, remote, handleGotoDef]);

  const chosenInstall =
    lspStat?.state === "absent" ? lspStat.install : null;

  const runInstall = useCallback(
    (command: string) => {
      if (command) onRunInTerminal(command);
    },
    [onRunInTerminal],
  );

  const copyInstall = useCallback((command: string) => {
    if (!command) return;
    void navigator.clipboard
      ?.writeText(command)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  }, []);

  const acceptManaged = useCallback(
    (serverId: string) => {
      const rel = selectedRef.current;
      if (!rel) return;
      void lspManagedConsent(session.id, serverId, "accept", rel)
        .then((s) => {
          if (selectedRef.current === rel) setLspStat(s);
        })
        .catch((e) => toastError(String(e)));
    },
    [session.id],
  );

  const useMineManaged = useCallback(
    (label: string, serverId: string) => {
      void lspManagedUseMine(serverId)
        .then((hints) => {
          setLspStat({
            state: "absent",
            server: label,
            install: hints.install,
            alternatives: hints.alternatives,
          });
          setShowInstall(true);
        })
        .catch((e) => toastError(String(e)));
    },
    [],
  );

  const refuseManaged = useCallback(
    (serverId: string) => {
      const rel = selectedRef.current;
      if (!rel) return;
      setShowManaged(false);
      void lspManagedConsent(session.id, serverId, "refuse", rel).catch(
        () => {},
      );
    },
    [session.id],
  );

  const installingId =
    lspStat?.state === "installing" ? lspStat.server_id : null;

  useEffect(() => {
    if (!installingId) return;
    let alive = true;
    const applyProgress = (progress: LspManagedProgress) => {
      if (!alive) return;
      setLspStat((prev) =>
        prev &&
        prev.state === "installing" &&
        prev.server_id === installingId
          ? { ...prev, progress }
          : prev,
      );
      const rel = selectedRef.current;
      if (!rel) return;
      if (progress.phase === "ready") {
        void lspRetry(session.id, rel)
          .then((s) => {
            if (selectedRef.current === rel) setLspStat(s);
            if (selectedRef.current === rel) refreshLspStatus(rel);
          })
          .catch(() => {});
      } else if (progress.phase === "error") {
        refreshLspStatus(rel);
      }
    };
    const unP = onLspManagedProgress(installingId, applyProgress);
    const unE = onLspManagedError(installingId, applyProgress);
    return () => {
      alive = false;
      void unP.then((f) => f());
      void unE.then((f) => f());
    };
  }, [installingId, session.id, refreshLspStatus]);

  const toggleDir = useCallback(
    (dir: string) => {
      setContextDir(dir);
      setExpandedDirs((prev) => {
        const next = new Set(prev);
        if (next.has(dir)) {
          next.delete(dir);
          void filesUnwatchDir(session.id, dir).catch(() => {});
        } else {
          next.add(dir);
          if (!(dir in entriesRef.current)) relist(dir);
          void filesWatchDir(session.id, dir).catch(() => {});
        }
        return next;
      });
    },
    [session.id, relist],
  );

  const loadAll = useCallback(async () => {
    const my = reqId.current;
    let cur = content;
    while (cur && cur.kind === "text" && cur.truncated && selected) {
      if (reqId.current !== my) return;
      const next = await filesRead(session.id, selected, cur.next_offset).catch(
        () => null,
      );
      if (reqId.current !== my) return;
      if (!next || next.kind !== "text") break;
      cur = { ...next, text: cur.text + next.text, offset: 0 };
      setContent(cur);
    }
  }, [content, selected, session.id]);

  const readFullDisk = useCallback(
    async (rel: string): Promise<string> => {
      let text = "";
      let offset = 0;
      for (;;) {
        const page = await filesRead(session.id, rel, offset).catch(() => null);
        if (!page || page.kind !== "text") break;
        text += page.text;
        if (!page.truncated) break;
        offset = page.next_offset;
      }
      return text;
    },
    [session.id],
  );

  const doRefresh = useCallback(async () => {
    setBusy(true);
    try {
      await filesRefresh(session.id).catch(() => {});
      const dirs = ["", ...Array.from(expandedDirs)];
      const results = await Promise.all(
        dirs.map((dir) =>
          filesListDir(session.id, dir)
            .then((listing) => ({ dir, listing }))
            .catch(() => null),
        ),
      );
      setEntriesByDir((prev) => {
        const next = { ...prev };
        for (const r of results) if (r) next[r.dir] = r.listing.entries;
        return next;
      });
      setDirMeta((prev) => {
        const next = { ...prev };
        for (const r of results)
          if (r)
            next[r.dir] = {
              total: r.listing.total,
              truncated: r.listing.truncated,
            };
        return next;
      });
      await filesDecorations(session.id)
        .then((decos) =>
          setDecorations(new Map(decos.map((d) => [d.path, d.status]))),
        )
        .catch(() => {});
    } finally {
      setBusy(false);
    }
  }, [session.id, expandedDirs]);

  const reanchor = useCallback(async () => {
    if (!(await confirmDiscard())) return;
    reqId.current += 1;
    await filesReanchor(session.id).catch(() => {});
    setExpandedDirs(new Set());
    setSelected(null);
    setContent(null);
    setEditing(false);
    setEditBaseline(null);
    setConflict(null);
    const listing = await filesListDir(session.id, "").catch(() => null);
    if (listing) {
      setEntriesByDir({ "": listing.entries });
      setDirMeta({ "": { total: listing.total, truncated: listing.truncated } });
    }
    const next = await filesPanelInfo(session.id).catch(() => null);
    if (next) setInfo(next);
  }, [session.id, confirmDiscard]);

  const isMarkdown = selected ? langForFile(selected) === "markdown" : false;

  useEffect(() => {
    setTokens(null);
    if (!content || content.kind !== "text" || !selected) return;
    if (isMarkdown && !markdownSource) return;
    const lang = langForFile(selected);
    if (!lang) return;
    let alive = true;
    void highlightBlock(
      content.text.split("\n"),
      lang,
      isDark ? "mono-dark" : "vitesse-dark",
    )
      .then((result) => {
        if (alive) setTokens(result);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [content, selected, isMarkdown, markdownSource, isDark]);

  const startEdit = useCallback(async () => {
    if (!selected) return;
    try {
      const base = await filesEditBegin(session.id, selected);
      setEditBaseline(base);
      setSavedHash(base.hash);
      setEditing(true);
      setDirty(false);
      setConflict(null);
      setConflictDiff(null);
      setDocVersion((v) => v + 1);
      const markers = await filesGutter(session.id, selected).catch(() => []);
      setGutter(markers);
      const path = selected;
      if (!remote)
        void lspOpen(session.id, path, base.text, true)
          .then((s) => {
            if (selectedRef.current !== path) return;
            setLspStat(s);
            if (s.state === "starting" || s.state === "available") {
              refreshLspStatus(path);
            }
          })
          .catch(() => {});
    } catch (e) {
      toastError(t("filesEditError"), e);
    }
  }, [selected, session.id, t, refreshLspStatus, remote]);

  const finishEdit = useCallback(() => {
    void filesEditEnd(session.id).catch(() => {});
    if (selected && !remote) void lspCloseDoc(session.id, selected).catch(() => {});
    setEditing(false);
    setEditBaseline(null);
    setDirty(false);
    setConflict(null);
    setConflictDiff(null);
    if (selected) openFile(selected);
  }, [session.id, selected, openFile, remote]);

  const save = useCallback(async () => {
    if (!selected || !editorRef.current) return;
    const value = editorRef.current.getValue();
    try {
      const result = await filesWrite(session.id, selected, value, savedHash);
      if (result.status === "written") {
        editorRef.current.markSaved();
        setSavedHash(result.hash);
        setDirty(false);
        setConflict(null);
        if (!remote) void lspDidSave(session.id, selected).catch(() => {});
      } else {
        setConflict({ path: selected, diskHash: result.disk_hash });
      }
    } catch (e) {
      toastError(t("filesSaveError"), e);
    }
  }, [selected, savedHash, session.id, t, remote]);

  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      const chord = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
      if (!chord || e.altKey || e.shiftKey || e.key.toLowerCase() !== "s") return;
      if (e.defaultPrevented) return;
      e.preventDefault();
      e.stopPropagation();
      void save();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, save]);

  const reloadFromDisk = useCallback(async () => {
    if (!selected) return;
    if (!(await confirmDiscard())) return;
    try {
      const base = await filesEditBegin(session.id, selected);
      setEditBaseline(base);
      setSavedHash(base.hash);
      setDirty(false);
      setConflict(null);
      setConflictDiff(null);
      setDocVersion((v) => v + 1);
      if (!remote)
        void lspOpen(session.id, selected, base.text, true).catch(() => {});
    } catch (e) {
      toastError(t("filesEditError"), e);
    }
  }, [selected, session.id, t, confirmDiscard, remote]);

  const overwrite = useCallback(async () => {
    if (!selected || !conflict || !editorRef.current) return;
    const value = editorRef.current.getValue();
    try {
      const result = await filesWrite(
        session.id,
        selected,
        value,
        conflict.diskHash ?? "",
      );
      if (result.status === "written") {
        editorRef.current.markSaved();
        setSavedHash(result.hash);
        setDirty(false);
        setConflict(null);
        setConflictDiff(null);
        if (!remote) void lspDidSave(session.id, selected).catch(() => {});
      } else {
        setConflict({ path: selected, diskHash: result.disk_hash });
      }
    } catch (e) {
      toastError(t("filesSaveError"), e);
    }
  }, [selected, conflict, session.id, t, remote]);

  const viewConflictDiff = useCallback(async () => {
    if (!selected || !editorRef.current) return;
    const edited = editorRef.current.getValue();
    const disk = await readFullDisk(selected);
    setConflictDiff({ disk, edited });
  }, [selected, readFullDisk]);

  const beginCreate = useCallback(
    (isDir: boolean) => {
      setTreeError(null);
      setTreeEdit({ kind: "create", parentDir: contextDir, isDir });
    },
    [contextDir],
  );

  const beginRename = useCallback((entry: FileEntry) => {
    setTreeError(null);
    setTreeEdit({ kind: "rename", rel: entry.rel_path, name: entry.name });
  }, []);

  const commitTreeEdit = useCallback(
    async (name: string) => {
      const trimmed = name.trim();
      if (!treeEdit || !trimmed) {
        setTreeEdit(null);
        return;
      }
      try {
        if (treeEdit.kind === "create") {
          const target = treeEdit.parentDir
            ? `${treeEdit.parentDir}/${trimmed}`
            : trimmed;
          await filesCreate(session.id, target, treeEdit.isDir);
          if (treeEdit.parentDir) {
            setExpandedDirs((prev) => new Set(prev).add(treeEdit.parentDir));
            relist(treeEdit.parentDir);
            void filesWatchDir(session.id, treeEdit.parentDir).catch(() => {});
          } else {
            relist("");
          }
        } else {
          const base = parentOf(treeEdit.rel);
          const target = base ? `${base}/${trimmed}` : trimmed;
          await filesRename(session.id, treeEdit.rel, target);
          // Remoto não tem watcher: reflete o rename relistando o pai.
          if (remote) relist(base);
          if (selected === treeEdit.rel) openFile(target);
        }
        setTreeEdit(null);
        setTreeError(null);
      } catch (e) {
        setTreeError(String(e));
      }
    },
    [treeEdit, session.id, relist, selected, openFile, remote],
  );

  const deleteEntry = useCallback(
    async (entry: FileEntry) => {
      if (entry.rel_path === selected && !(await confirmDiscard())) return;
      if (remote) {
        // Sem Lixeira remota: confirmação SEMPRE (arquivo ou pasta).
        const ok = await requestConfirm({
          title: t("filesDeleteRemoteTitle", { name: entry.name }),
          detail: t("filesDeleteRemoteDetail"),
          confirmLabel: t("filesDeleteConfirm"),
          destructive: true,
        });
        if (!ok) return;
      } else if (entry.is_dir) {
        const listing = await filesListDir(session.id, entry.rel_path).catch(
          () => null,
        );
        if (listing && listing.entries.length > 0) {
          const ok = await requestConfirm({
            title: t("filesDeleteDirTitle", { name: entry.name }),
            detail: t("filesDeleteDirDetail"),
            confirmLabel: t("filesDeleteConfirm"),
            destructive: true,
          });
          if (!ok) return;
        }
      }
      try {
        await filesDelete(session.id, entry.rel_path);
        if (selected === entry.rel_path) {
          setSelected(null);
          setContent(null);
          setEditing(false);
        }
        // Remoto não tem watcher: reflete a remoção relistando o pai.
        if (remote) relist(parentOf(entry.rel_path));
      } catch (e) {
        toastError(t("filesDeleteError"), e);
      }
    },
    [session.id, selected, t, confirmDiscard, remote, relist],
  );

  const selectedDecorated = selected ? decorations.has(selected) : false;
  const selectedEntry = useMemo(() => {
    if (!selected) return null;
    const siblings = entriesByDir[parentOf(selected)] ?? [];
    return siblings.find((e) => e.rel_path === selected) ?? null;
  }, [selected, entriesByDir]);
  const canEdit = content?.kind === "text";
  const rootLabel = info
    ? info.root.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || info.root
    : t("filesRootLabel");
  const driftLabel = info?.drifted_to
    ? info.drifted_to.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ||
      info.drifted_to
    : null;
  const lspErrors = currentDiagnostics.filter((d) => d.severity === 1).length;
  const lspWarnings = currentDiagnostics.filter((d) => d.severity === 2).length;
  const lspServer =
    lspStat && lspStat.state !== "unsupported" ? lspStat.server : "";

  const gutterMap = useMemo(() => {
    const map = new Map<number, GutterKind>();
    for (const marker of gutter) map.set(marker.line, marker.kind);
    return map;
  }, [gutter]);

  const mdComponents = useMemo<Components>(
    () => ({
      img(props) {
        const { src, alt } = props;
        return typeof src === "string" && src.length > 0 && !isRemoteUrl(src) ? (
          <img src={src} alt={alt ?? ""} className="max-w-full" />
        ) : (
          <span className="text-tyba-text-faint">{alt ?? ""}</span>
        );
      },
      a(props) {
        const { href, children } = props;
        return (
          <a href={href || undefined} onClick={(e) => e.preventDefault()}>
            {children}
          </a>
        );
      },
      pre(props) {
        return <>{props.children}</>;
      },
      code(props) {
        const { className, children } = props;
        const text = String(children ?? "");
        const match = /language-([\w-]+)/.exec(className || "");
        const block = !!match || text.includes("\n");
        if (!block) {
          return <code className="files-md-inline">{children}</code>;
        }
        return (
          <MarkdownCode
            code={text}
            lang={match ? langForFence(match[1]) : null}
            dark={isDark}
          />
        );
      },
    }),
    [isDark],
  );

  const rows = useMemo(() => {
    type Row =
      | { kind: "entry"; entry: FileEntry; depth: number }
      | { kind: "more"; dir: string; hidden: number; depth: number };
    const out: Row[] = [];
    const walk = (dir: string, depth: number) => {
      const entries = entriesByDir[dir] ?? [];
      for (const entry of entries) {
        out.push({ kind: "entry", entry, depth });
        if (entry.is_dir && expandedDirs.has(entry.rel_path)) {
          walk(entry.rel_path, depth + 1);
        }
      }
      const meta = dirMeta[dir];
      if (meta?.truncated) {
        out.push({
          kind: "more",
          dir,
          hidden: Math.max(0, meta.total - entries.length),
          depth,
        });
      }
    };
    walk("", 0);
    return out;
  }, [entriesByDir, expandedDirs, dirMeta]);

  const sourceLines = useMemo(
    () => (content?.kind === "text" ? content.text.split("\n") : []),
    [content],
  );

  const renderSourceLine = (index: number) => {
    if (tokens) {
      const line = tokens[index];
      if (!line || line.length === 0) return " ";
      return line.map((tk, j) => (
        <span key={j} style={{ color: tk.color }}>
          {tk.text}
        </span>
      ));
    }
    const text = sourceLines[index];
    return text === "" ? " " : text;
  };

  const inlineInput = (
    initial: string,
    placeholder: string,
    onCommit: (value: string) => void,
  ) => (
    <input
      autoFocus
      defaultValue={initial}
      placeholder={placeholder}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") onCommit((e.target as HTMLInputElement).value);
        else if (e.key === "Escape") {
          setTreeEdit(null);
          setTreeError(null);
        }
      }}
      onBlur={(e) => onCommit(e.target.value)}
      className="min-w-0 flex-1 rounded-[3px] border border-tyba-border-strong bg-tyba-surface px-1 py-0.5 text-[12px] text-tyba-text outline-none focus:border-tyba-green"
    />
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-tyba-bg">
      <header className="flex h-8 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
        <TreeView size={14} className="shrink-0 text-tyba-text-faint" />
        <span className="min-w-0 truncate text-[12px] text-tyba-text">
          {t("filesTitle")}
        </span>
        {info && (
          <span className="min-w-0 truncate font-mono text-[11px] text-tyba-text-muted">
            {info.root}
          </span>
        )}
        {remote && info?.host && (
          <span
            title={t("filesRemoteHost", { host: info.host })}
            className="flex shrink-0 items-center gap-1 rounded-[3px] bg-tyba-text/[.06] px-1.5 py-0.5 text-[10px] text-tyba-text-muted"
          >
            <HardDrives size={11} className="text-tyba-green" />
            <span className="max-w-[120px] truncate font-mono">{info.host}</span>
          </span>
        )}
        <div className="flex-1" />
        {remote && (
          <button
            onClick={() => void doRefresh()}
            aria-label={t("filesRefresh")}
            title={t("filesRefresh")}
            className="flex items-center gap-1 rounded-[3px] bg-tyba-green/10 px-1.5 py-0.5 text-[10px] text-tyba-green hover:bg-tyba-green/20"
          >
            <ArrowClockwise size={13} className={busy ? "animate-spin" : ""} />
          </button>
        )}
        {/* Discreto enquanto a raiz bate com o cwd vivo; com rótulo quando
            divergem. A raiz ser fixa é decisão, e o re-ancorar é a saída
            prevista — o que faltava era saber que a saída existe. Ícone mudo de
            14px num header de 28px é indistinguível de decoração. */}
        {driftLabel ? (
          <button
            onClick={() => void reanchor()}
            aria-label={t("filesReanchorTo", { dir: driftLabel })}
            title={t("filesReanchorTo", { dir: driftLabel })}
            className="flex items-center gap-1 rounded-[3px] bg-tyba-amber/10 px-1.5 py-0.5 text-[10px] text-tyba-amber hover:bg-tyba-amber/20"
          >
            <Crosshair size={13} />
            <span className="max-w-[120px] truncate font-mono">
              {driftLabel}
            </span>
          </button>
        ) : (
          <button
            onClick={() => void reanchor()}
            aria-label={t("filesReanchor")}
            title={t("filesReanchor")}
            className="text-tyba-text-faint hover:text-tyba-text"
          >
            <Crosshair size={14} />
          </button>
        )}
        <button
          onClick={onToggleExpand}
          aria-label={t(expanded ? "tunnelsCollapse" : "tunnelsExpand")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          {expanded ? <ArrowsInSimple size={14} /> : <ArrowsOutSimple size={14} />}
        </button>
        <button
          onClick={onClose}
          aria-label={t("tunnelsClose")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={14} />
        </button>
      </header>

      {remote && connectionDown && (
        <div className="flex items-center gap-2 border-b border-tyba-amber/40 bg-tyba-amber/10 px-3 py-1.5 text-[11px] text-tyba-text">
          <span className="min-w-0 flex-1 truncate">
            {t("filesReconnecting")}
          </span>
          <button
            onClick={() => void doRefresh()}
            className="shrink-0 rounded-[3px] px-1.5 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
          >
            {t("filesRefresh")}
          </button>
        </div>
      )}

      {/* A árvore pinta à direita por `flex-row-reverse`, e continua sendo o
          primeiro filho no DOM de propósito: a ordem de leitura e de Tab é
          árvore → conteúdo, que é a ordem lógica de navegar arquivo, não a
          ordem em que eles aparecem na tela. */}
      <div className="flex min-h-0 flex-1 flex-row-reverse">
        <div className="flex w-[240px] shrink-0 flex-col border-l border-tyba-border">
          <div className="flex h-7 shrink-0 items-center gap-1 border-b border-tyba-border px-2">
            <span
              className="min-w-0 flex-1 truncate text-[10px] uppercase tracking-wide text-tyba-text-faint"
              title={info?.root}
            >
              {rootLabel}
            </span>
            <button
              onClick={() => beginCreate(false)}
              aria-label={t("filesNewFile")}
              title={t("filesNewFile")}
              className="text-tyba-text-faint hover:text-tyba-text"
            >
              <FilePlus size={14} />
            </button>
            <button
              onClick={() => beginCreate(true)}
              aria-label={t("filesNewFolder")}
              title={t("filesNewFolder")}
              className="text-tyba-text-faint hover:text-tyba-text"
            >
              <FolderSimplePlus size={14} />
            </button>
          </div>
          <div
            className="min-h-0 flex-1 overflow-auto py-1"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "F2" && selectedEntry) {
                e.preventDefault();
                beginRename(selectedEntry);
              }
            }}
          >
            {treeEdit?.kind === "create" && (
              <div
                className="flex h-6 items-center gap-1 pr-2"
                style={{
                  paddingLeft: `${
                    8 +
                    (treeEdit.parentDir
                      ? treeEdit.parentDir.split("/").length
                      : 0) *
                      12
                  }px`,
                }}
              >
                <span className="flex size-4 shrink-0 items-center justify-center text-tyba-text-faint">
                  {treeEdit.isDir ? <Folder size={13} /> : <FilePlus size={13} />}
                </span>
                {inlineInput(
                  "",
                  treeEdit.isDir ? t("filesNewFolder") : t("filesNewFile"),
                  commitTreeEdit,
                )}
              </div>
            )}
            {treeError && (
              <div className="px-3 py-1 text-[10px] text-tyba-red">
                {treeError}
              </div>
            )}
            {busy && rows.length === 0 ? (
              <div className="flex items-center gap-2 px-3 py-2 text-[11px] text-tyba-text-faint">
                <ArrowClockwise size={12} className="animate-spin" />
                {t("filesLoading")}
              </div>
            ) : rows.length === 0 && !treeEdit ? (
              <div className="px-3 py-2 text-[11px] text-tyba-text-faint">
                {t("filesEmptyDir")}
              </div>
            ) : (
              rows.map((row) => {
                if (row.kind === "more") {
                  return (
                    <div
                      key={`more:${row.dir}`}
                      className="flex h-6 items-center text-[11px] italic text-tyba-text-faint"
                      style={{ paddingLeft: `${8 + (row.depth + 1) * 12}px` }}
                    >
                      {t("filesMore", { count: row.hidden })}
                    </div>
                  );
                }
                const { entry, depth } = row;
                const deco = decorations.get(entry.rel_path);
                const isSelected = selected === entry.rel_path;
                const EntryIcon = fileIcon(entry.name);
                const renaming =
                  treeEdit?.kind === "rename" && treeEdit.rel === entry.rel_path;
                return (
                  <div
                    key={entry.rel_path}
                    onClick={() =>
                      entry.is_dir
                        ? toggleDir(entry.rel_path)
                        : void openFileGuarded(entry.rel_path)
                    }
                    className={`group flex h-6 cursor-pointer items-center gap-1 pr-2 text-[12px] ${
                      isSelected
                        ? "bg-tyba-text/[.08] text-tyba-text"
                        : "text-tyba-text-muted hover:bg-tyba-text/[.04]"
                    } ${entry.gitignored ? "opacity-50" : ""}`}
                    style={{ paddingLeft: `${8 + depth * 12}px` }}
                  >
                    <span className="flex size-4 shrink-0 items-center justify-center text-tyba-text-faint">
                      {entry.is_dir ? (
                        expandedDirs.has(entry.rel_path) ? (
                          <CaretDown size={10} weight="bold" />
                        ) : (
                          <CaretRight size={10} weight="bold" />
                        )
                      ) : null}
                    </span>
                    <span className="flex size-4 shrink-0 items-center justify-center text-tyba-text-faint">
                      {entry.is_dir ? (
                        expandedDirs.has(entry.rel_path) ? (
                          <FolderOpen size={13} />
                        ) : (
                          <Folder size={13} />
                        )
                      ) : (
                        <EntryIcon size={13} />
                      )}
                    </span>
                    {renaming ? (
                      inlineInput(treeEdit.name, entry.name, commitTreeEdit)
                    ) : (
                      <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                    )}
                    {!renaming && deco && (
                      <span
                        className={`shrink-0 font-mono text-[10px] ${DECO_CLASS[deco]} group-hover:hidden`}
                      >
                        {DECO_LABEL[deco]}
                      </span>
                    )}
                    {!renaming && (
                      <span className="hidden shrink-0 items-center gap-0.5 group-hover:flex">
                        {deco && (
                          <button
                            onClick={(ev) => {
                              ev.stopPropagation();
                              onJumpToDiff();
                            }}
                            aria-label={t("filesJumpToDiff")}
                            title={t("filesJumpToDiff")}
                            className="flex size-4 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-green"
                          >
                            <PlusMinus size={11} />
                          </button>
                        )}
                        <button
                          onClick={(ev) => {
                            ev.stopPropagation();
                            beginRename(entry);
                          }}
                          aria-label={t("filesRenameAction")}
                          title={t("filesRenameAction")}
                          className="flex size-4 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-text"
                        >
                          <PencilSimple size={11} />
                        </button>
                        <button
                          onClick={(ev) => {
                            ev.stopPropagation();
                            void deleteEntry(entry);
                          }}
                          aria-label={t("filesDeleteAction")}
                          title={t("filesDeleteAction")}
                          className="flex size-4 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-red"
                        >
                          <TrashSimple size={11} />
                        </button>
                      </span>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </div>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {selected ? (
            <>
              <div className="flex h-7 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
                <span className="min-w-0 truncate font-mono text-[11px] text-tyba-text-muted">
                  {selected}
                </span>
                {editing && dirty && (
                  <span
                    aria-label={t("filesDirty")}
                    title={t("filesDirty")}
                    className="size-1.5 shrink-0 rounded-full bg-tyba-amber"
                  />
                )}
                {lspStat?.state === "experimental" && (
                  <span
                    title={t("lspExperimentalHint", { server: lspStat.server })}
                    className="shrink-0 text-[10px] uppercase tracking-wide text-tyba-text-faint"
                  >
                    {t("lspExperimental")}
                  </span>
                )}
                {lspStat?.state === "absent" && (
                  <button
                    onClick={() => setShowInstall((v) => !v)}
                    title={t("lspAbsent", { server: lspStat.server })}
                    className="flex shrink-0 items-center gap-1 text-[10px] text-tyba-amber hover:text-tyba-text"
                  >
                    <Warning size={12} />
                    <span>{t("lspInstall")}</span>
                  </button>
                )}
                {lspStat?.state === "managed_offer" && (
                  <button
                    onClick={() => setShowManaged((v) => !v)}
                    title={t("lspManagedOffer", { server: lspStat.server })}
                    className="flex shrink-0 items-center gap-1 text-[10px] text-tyba-blue hover:text-tyba-text"
                  >
                    <DownloadSimple size={12} />
                    <span>{t("lspManagedInstall")}</span>
                  </button>
                )}
                {lspStat?.state === "installing" && (
                  <span
                    title={lspServer}
                    className="flex shrink-0 items-center gap-1.5 text-[10px] text-tyba-text-faint"
                  >
                    <span className="size-1.5 animate-pulse rounded-full bg-tyba-blue" />
                    <span>
                      {lspStat.progress.phase === "downloading"
                        ? t("lspManagedDownloading", {
                            percent: pct(
                              lspStat.progress.downloaded,
                              lspStat.progress.total,
                            ),
                          })
                        : t(managedPhaseKey(lspStat.progress.phase))}
                    </span>
                  </span>
                )}
                {lspStat?.state === "starting" && (
                  <span
                    title={lspServer}
                    className="flex shrink-0 items-center gap-1.5 text-[10px] text-tyba-text-faint"
                  >
                    <span className="size-1.5 animate-pulse rounded-full bg-tyba-amber" />
                    <span>{t("lspStarting")}</span>
                  </span>
                )}
                {lspStat?.state === "crashed" && (
                  <button
                    onClick={handleRetry}
                    title={lspStat.reason ?? t("lspCrashed")}
                    className="flex shrink-0 items-center gap-1 text-[10px] text-tyba-red hover:text-tyba-text"
                  >
                    <span>{t("lspCrashed")}</span>
                    <ArrowClockwise size={11} />
                  </button>
                )}
                {lspStat?.state === "ready" && (
                  <span
                    title={
                      lspErrors > 0 || lspWarnings > 0
                        ? t("lspDiagnosticsCount", {
                            errors: lspErrors,
                            warnings: lspWarnings,
                          })
                        : lspServer
                    }
                    className="flex shrink-0 items-center gap-1.5 text-[10px]"
                  >
                    {lspErrors === 0 && lspWarnings === 0 ? (
                      <span className="size-1.5 rounded-full bg-tyba-green" />
                    ) : (
                      <>
                        {lspErrors > 0 && (
                          <span className="flex items-center gap-0.5 text-tyba-red">
                            <WarningCircle size={13} weight="fill" />
                            <span className="font-mono">{lspErrors}</span>
                          </span>
                        )}
                        {lspWarnings > 0 && (
                          <span className="flex items-center gap-0.5 text-tyba-amber">
                            <Warning size={13} weight="fill" />
                            <span className="font-mono">{lspWarnings}</span>
                          </span>
                        )}
                      </>
                    )}
                    {(diagsTruncated || lspStat.truncated) && (
                      <span
                        title={t("lspTruncated")}
                        className="font-mono text-tyba-text-faint"
                      >
                        +
                      </span>
                    )}
                  </span>
                )}
                <div className="flex-1" />
                {editing ? (
                  <>
                    <button
                      onClick={() => void save()}
                      disabled={!dirty}
                      className={`text-[11px] ${
                        dirty
                          ? "text-tyba-text hover:text-tyba-green"
                          : "text-tyba-text-faint"
                      }`}
                    >
                      {t("filesSave")}
                    </button>
                    <button
                      onClick={finishEdit}
                      className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
                    >
                      {t("filesDone")}
                    </button>
                  </>
                ) : (
                  <>
                    {isMarkdown && (
                      <button
                        onClick={() => setMarkdownSource((v) => !v)}
                        className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
                      >
                        {markdownSource ? t("filesShowPreview") : t("filesShowSource")}
                      </button>
                    )}
                    {canEdit && (
                      <button
                        onClick={() => void startEdit()}
                        className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
                      >
                        {t("filesEdit")}
                      </button>
                    )}
                    {selectedDecorated && (
                      <button
                        onClick={onJumpToDiff}
                        aria-label={t("filesJumpToDiff")}
                        title={t("filesJumpToDiff")}
                        className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-green"
                      >
                        <PlusMinus size={13} />
                      </button>
                    )}
                    {!remote && (
                      <button
                        onClick={() =>
                          void filesOpenExternal(
                            session.id,
                            selected,
                            editor,
                          ).catch((e) => setContentError(String(e)))
                        }
                        aria-label={t("filesOpenExternal")}
                        title={t("filesOpenExternal")}
                        className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-text"
                      >
                        <ArrowSquareOut size={13} />
                      </button>
                    )}
                  </>
                )}
              </div>

              {lspStat?.state === "managed_offer" && showManaged && (
                <div className="flex flex-col gap-2 border-b border-tyba-blue/40 bg-tyba-blue/10 px-3 py-2.5 text-[11px] text-tyba-text">
                  <div className="flex items-start gap-2">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="truncate font-medium">
                          {lspStat.card.label}
                        </span>
                        <span className="shrink-0 font-mono text-tyba-text-faint">
                          v{lspStat.card.version}
                        </span>
                        <span
                          title={t("lspManagedVerifiedHint")}
                          className="flex shrink-0 items-center gap-0.5 text-tyba-green"
                        >
                          <SealCheck size={12} weight="fill" />
                          {t("lspManagedVerified")}
                        </span>
                      </div>
                      <div className="mt-0.5 text-[10px] text-tyba-text-muted">
                        {t("lspManagedMeta", {
                          size: formatBytes(lspStat.card.size),
                          source: lspStat.card.source,
                        })}
                      </div>
                    </div>
                    <button
                      onClick={() => refuseManaged(lspStat.card.server_id)}
                      aria-label={t("tunnelsClose")}
                      className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                    >
                      <X size={12} />
                    </button>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => acceptManaged(lspStat.card.server_id)}
                      className="flex items-center gap-1 rounded-[3px] bg-tyba-blue/20 px-2 py-0.5 text-tyba-text hover:bg-tyba-blue/30"
                    >
                      <DownloadSimple size={12} />
                      {t("lspManagedAccept")}
                    </button>
                    <button
                      onClick={() =>
                        useMineManaged(lspStat.card.label, lspStat.card.server_id)
                      }
                      className="rounded-[3px] px-2 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
                    >
                      {t("lspManagedUseMine")}
                    </button>
                    <button
                      onClick={() => refuseManaged(lspStat.card.server_id)}
                      className="ml-auto text-tyba-text-faint hover:text-tyba-text"
                    >
                      {t("lspManagedRefuse")}
                    </button>
                  </div>
                </div>
              )}

              {lspStat?.state === "installing" && (
                <div className="flex flex-col gap-1.5 border-b border-tyba-blue/40 bg-tyba-blue/10 px-3 py-2 text-[11px] text-tyba-text">
                  <div className="flex items-center gap-2">
                    <DownloadSimple size={13} className="shrink-0 text-tyba-blue" />
                    <span className="min-w-0 flex-1 truncate">
                      {lspStat.progress.phase === "downloading"
                        ? t("lspManagedDownloadingOf", {
                            server: lspStat.server,
                            done: formatBytes(lspStat.progress.downloaded),
                            total: formatBytes(lspStat.progress.total),
                          })
                        : lspStat.progress.phase === "error"
                          ? lspStat.progress.message
                          : t(managedPhaseKey(lspStat.progress.phase))}
                    </span>
                    {lspStat.progress.phase === "error" && (
                      <button
                        onClick={() => acceptManaged(lspStat.server_id)}
                        title={t("lspManagedRetry")}
                        aria-label={t("lspManagedRetry")}
                        className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                      >
                        <ArrowClockwise size={12} />
                      </button>
                    )}
                  </div>
                  {lspStat.progress.phase === "downloading" && (
                    <div className="h-1 w-full overflow-hidden rounded-full bg-tyba-text/[.1]">
                      <div
                        className="h-full rounded-full bg-tyba-blue transition-all"
                        style={{
                          width: `${pct(
                            lspStat.progress.downloaded,
                            lspStat.progress.total,
                          )}%`,
                        }}
                      />
                    </div>
                  )}
                </div>
              )}

              {lspStat?.state === "absent" && showInstall && (
                <div className="flex flex-col gap-1.5 border-b border-tyba-amber/40 bg-tyba-amber/10 px-3 py-2 text-[11px] text-tyba-text">
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate">
                      {t("lspInstallHint", { server: lspStat.server })}
                    </span>
                    <button
                      onClick={() => setShowInstall(false)}
                      aria-label={t("tunnelsClose")}
                      className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                    >
                      <X size={12} />
                    </button>
                  </div>
                  {chosenInstall ? (
                    <>
                      <div className="flex items-center gap-2">
                        <code className="min-w-0 flex-1 truncate rounded-[3px] bg-tyba-surface px-2 py-1 font-mono text-[11px] text-tyba-text">
                          {chosenInstall.command}
                        </code>
                        <button
                          onClick={() => copyInstall(chosenInstall.command)}
                          title={t("lspCopy")}
                          aria-label={t("lspCopy")}
                          className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                        >
                          <Copy size={13} />
                        </button>
                      </div>
                      <div className="flex items-center gap-2">
                        <button
                          onClick={() => runInstall(chosenInstall.command)}
                          className="flex items-center gap-1 rounded-[3px] bg-tyba-text/[.08] px-2 py-0.5 text-tyba-text hover:bg-tyba-text/[.14]"
                        >
                          <Terminal size={12} />
                          {t("lspInstallInTerminal")}
                        </button>
                        {copied && (
                          <span className="text-tyba-green">{t("lspCopied")}</span>
                        )}
                        {lspStat.alternatives.length > 0 && (
                          <button
                            onClick={() => setShowAlts((v) => !v)}
                            className="ml-auto text-tyba-text-faint hover:text-tyba-text"
                          >
                            {showAlts
                              ? t("lspHideAlternatives")
                              : t("lspAlternatives", {
                                  count: lspStat.alternatives.length,
                                })}
                          </button>
                        )}
                      </div>
                      {showAlts &&
                        lspStat.alternatives.map((alt) => (
                          <div
                            key={alt.manager + alt.command}
                            className="flex items-center gap-2"
                          >
                            <span className="w-14 shrink-0 text-[10px] uppercase tracking-wide text-tyba-text-faint">
                              {alt.manager}
                            </span>
                            <code className="min-w-0 flex-1 truncate rounded-[3px] bg-tyba-surface px-2 py-1 font-mono text-[11px] text-tyba-text-muted">
                              {alt.command}
                            </code>
                            <button
                              onClick={() => copyInstall(alt.command)}
                              title={t("lspCopy")}
                              aria-label={t("lspCopy")}
                              className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                            >
                              <Copy size={12} />
                            </button>
                            <button
                              onClick={() => runInstall(alt.command)}
                              title={t("lspInstallInTerminal")}
                              aria-label={t("lspInstallInTerminal")}
                              className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                            >
                              <Terminal size={12} />
                            </button>
                          </div>
                        ))}
                    </>
                  ) : (
                    <span className="text-tyba-text-muted">
                      {t("lspNoInstaller", { server: lspStat.server })}
                    </span>
                  )}
                </div>
              )}

              {conflict && (
                <div className="flex items-center gap-2 border-b border-tyba-amber/40 bg-tyba-amber/10 px-3 py-1.5 text-[11px] text-tyba-text">
                  <span className="min-w-0 flex-1 truncate">
                    {t("filesConflictBanner")}
                  </span>
                  <button
                    onClick={() => void viewConflictDiff()}
                    className="shrink-0 rounded-[3px] px-1.5 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
                  >
                    {t("filesConflictDiff")}
                  </button>
                  <button
                    onClick={() => void reloadFromDisk()}
                    className="shrink-0 rounded-[3px] px-1.5 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
                  >
                    {t("filesConflictReload")}
                  </button>
                  <button
                    onClick={() => void overwrite()}
                    className="shrink-0 rounded-[3px] px-1.5 py-0.5 text-tyba-red hover:bg-tyba-red/10"
                  >
                    {t("filesConflictOverwrite")}
                  </button>
                </div>
              )}

              <div className="relative min-h-0 flex-1 overflow-hidden">
                {editing && editBaseline ? (
                  <CodeEditor
                    ref={editorRef}
                    key={`${selected}:${docVersion}`}
                    doc={editBaseline.text}
                    filename={selected}
                    dark={isDark}
                    markers={gutter}
                    lsp={lspBridge}
                    diagnostics={currentDiagnostics}
                    onDirtyChange={setDirty}
                    onSave={() => void save()}
                  />
                ) : (
                  <div className="h-full overflow-auto">
                    {contentError ? (
                      <div className="p-4 text-[12px] text-tyba-red">
                        {t("filesLoadError")}
                      </div>
                    ) : !content ? (
                      <div className="p-4 text-[12px] text-tyba-text-faint">
                        {t("diffLoading")}
                      </div>
                    ) : content.kind === "image" ? (
                      <div className="flex items-center justify-center p-4">
                        <img
                          src={`data:${content.mime};base64,${content.data}`}
                          alt={selected}
                          className="max-h-full max-w-full object-contain"
                        />
                      </div>
                    ) : content.kind === "binary" ? (
                      <div className="p-4 text-[12px] text-tyba-text-faint">
                        {t("filesBinaryPlaceholder", {
                          size: formatBytes(content.total),
                        })}
                      </div>
                    ) : isMarkdown && !markdownSource ? (
                      <div className="files-markdown">
                        <ReactMarkdown
                          remarkPlugins={[remarkGfm]}
                          urlTransform={safeMarkdownUrl}
                          components={mdComponents}
                        >
                          {content.text}
                        </ReactMarkdown>
                      </div>
                    ) : (
                      <div className="min-w-0 py-3 font-mono text-[12px] leading-[1.5] text-tyba-text">
                        {sourceLines.map((_, i) => {
                          const kind = gutterMap.get(i + 1);
                          return (
                            <div key={i} className="flex">
                              <span
                                className={`mx-1 w-0.5 shrink-0 self-stretch ${
                                  kind ? GUTTER_CLASS[kind] : ""
                                }`}
                              />
                              <span className="min-w-0 whitespace-pre pr-4">
                                {renderSourceLine(i)}
                              </span>
                            </div>
                          );
                        })}
                      </div>
                    )}
                    {content && content.kind === "text" && content.truncated && (
                      <div className="flex items-center gap-2 border-t border-tyba-border px-4 py-2 text-[11px] text-tyba-text-faint">
                        <span>{t("filesTruncatedNote")}</span>
                        <button
                          onClick={() => void loadAll()}
                          className="rounded-[3px] bg-tyba-text/[.08] px-2 py-0.5 text-tyba-text hover:bg-tyba-text/[.14]"
                        >
                          {t("filesLoadAll")}
                        </button>
                      </div>
                    )}
                  </div>
                )}

                {conflictDiff && (
                  <div className="absolute inset-0 flex flex-col bg-tyba-bg">
                    <div className="flex h-7 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
                      <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-muted">
                        {t("filesConflictDiffTitle")}
                      </span>
                      <button
                        onClick={() => setConflictDiff(null)}
                        aria-label={t("tunnelsClose")}
                        className="text-tyba-text-faint hover:text-tyba-text"
                      >
                        <X size={14} />
                      </button>
                    </div>
                    <div className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[12px] leading-[1.5]">
                      {lineDiff(conflictDiff.disk, conflictDiff.edited).map(
                        (r, i) => (
                          <div
                            key={i}
                            className={`whitespace-pre px-3 ${
                              r.kind === "added"
                                ? "bg-tyba-green/10 text-tyba-green"
                                : r.kind === "removed"
                                  ? "bg-tyba-red/10 text-tyba-red"
                                  : "text-tyba-text-muted"
                            }`}
                          >
                            <span className="mr-2 select-none text-tyba-text-faint">
                              {r.kind === "added"
                                ? "+"
                                : r.kind === "removed"
                                  ? "-"
                                  : " "}
                            </span>
                            {r.text === "" ? " " : r.text}
                          </div>
                        ),
                      )}
                    </div>
                    <div className="flex shrink-0 items-center gap-3 border-t border-tyba-border px-3 py-1.5 text-[10px] text-tyba-text-faint">
                      <span className="text-tyba-red">− {t("filesConflictDisk")}</span>
                      <span className="text-tyba-green">
                        + {t("filesConflictEdited")}
                      </span>
                    </div>
                  </div>
                )}
              </div>
            </>
          ) : (
            <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
              {t("filesViewerEmpty")}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
