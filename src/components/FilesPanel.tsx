import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  ArrowsInSimple,
  ArrowsOutSimple,
  CaretDown,
  CaretRight,
  Crosshair,
  FilePlus,
  Folder,
  FolderOpen,
  FolderSimplePlus,
  HardDrives,
  MagnifyingGlass,
  PencilSimple,
  PlusMinus,
  TrashSimple,
  X,
} from "@phosphor-icons/react";

import type {
  FileDecoStatus,
  FileEntry,
  FilesPanelInfo,
  Session,
} from "@/lib/ipc";
import {
  filesCreate,
  filesDecorations,
  filesDelete,
  filesListDir,
  filesPanelInfo,
  filesReanchor,
  filesRefresh,
  filesRename,
  filesSearch,
  filesUnwatchDir,
  filesWatchDir,
  onFilesDecorations,
  onFilesTree,
  onSessionCwd,
} from "@/lib/ipc";
import { requestConfirm } from "@/lib/confirm";
import { isSensitivePath } from "@/lib/sensitivePaths";
import { toastError } from "@/lib/toast";
import { LspStatusChip, useLspStatus } from "./LspStatusUi";

interface Props {
  session: Session;
  expanded: boolean;
  /** Abre o arquivo numa aba de editor (ver `EditorTab`). */
  onOpenFile: (rel: string) => void;
  onToggleExpand: () => void;
  onClose: () => void;
  onJumpToDiff: () => void;
  /** Rótulo do atalho do buscador de arquivos, no placeholder da busca. */
  finderShortcut?: string;
  /** Há aba de editor aberta desta sessão: o LSP fica na linha de status dela. */
  editorOpen?: boolean;
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

function parentOf(rel: string): string {
  const i = rel.lastIndexOf("/");
  return i < 0 ? "" : rel.slice(0, i);
}

/** Rodapé do painel: o chip do LSP do arquivo selecionado. Só aparece quando
 *  nenhuma aba de editor da sessão está aberta — com editor aberto, o estado
 *  do LSP mora na linha de status dele, e repetir aqui era ruído. */
function FilesFooter({
  session,
  rel,
  remote,
}: {
  session: Session["id"];
  rel: string;
  remote: boolean;
}) {
  const { stat, refresh, retry } = useLspStatus(session, rel, remote);
  useEffect(() => {
    refresh();
  }, [refresh]);
  if (!stat || stat.state === "unsupported") return null;
  return (
    <div className="flex shrink-0 items-center gap-1.5 tyba-divide-t px-4 py-1.5 text-[11px] text-tyba-text-faint">
      <LspStatusChip stat={stat} truncated={false} onRetry={retry} />
    </div>
  );
}

/**
 * Árvore de arquivos da sessão (painel lateral, 380px). Abrir um arquivo vira uma
 * aba de editor ao lado das de terminal; o painel só navega e organiza
 * (criar, renomear, apagar).
 */
export function FilesPanel({
  session,
  expanded,
  onOpenFile,
  onToggleExpand,
  onClose,
  onJumpToDiff,
  finderShortcut,
  editorOpen = false,
}: Props) {
  const { t } = useTranslation();
  const [treeQuery, setTreeQuery] = useState("");
  const [treeHits, setTreeHits] = useState<string[] | null>(null);
  useEffect(() => {
    const q = treeQuery.trim();
    if (!q) {
      setTreeHits(null);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      void filesSearch(session.id, q, 200)
        .then((r) => {
          if (alive) setTreeHits(r.paths);
        })
        .catch(() => {
          if (alive) setTreeHits([]);
        });
    }, 120);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [treeQuery, session.id]);
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
  const [dirMeta, setDirMeta] = useState<
    Record<string, { total: number; truncated: boolean }>
  >({});
  const [treeEdit, setTreeEdit] = useState<TreeEdit | null>(null);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const remote = info?.remote ?? false;
  const connectionDown = session.connection && session.connection !== "live";

  const entriesRef = useRef(entriesByDir);
  entriesRef.current = entriesByDir;

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

  const openFile = useCallback(
    (rel: string) => {
      setSelected(rel);
      setContextDir(parentOf(rel));
      onOpenFile(rel);
    },
    [onOpenFile],
  );

  useEffect(() => {
    let alive = true;
    setEntriesByDir({});
    setDirMeta({});
    setExpandedDirs(new Set());
    setSelected(null);
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
      void unCwd.then((f) => f());
    };
  }, [session.id, relist]);

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
    await filesReanchor(session.id).catch(() => {});
    setExpandedDirs(new Set());
    setSelected(null);
    const listing = await filesListDir(session.id, "").catch(() => null);
    if (listing) {
      setEntriesByDir({ "": listing.entries });
      setDirMeta({ "": { total: listing.total, truncated: listing.truncated } });
    }
    const next = await filesPanelInfo(session.id).catch(() => null);
    if (next) setInfo(next);
  }, [session.id]);


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
          if (selected === treeEdit.rel) setSelected(target);
        }
        setTreeEdit(null);
        setTreeError(null);
      } catch (e) {
        setTreeError(String(e));
      }
    },
    [treeEdit, session.id, relist, selected, remote],
  );

  const deleteEntry = useCallback(
    async (entry: FileEntry) => {
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
        }
        // Remoto não tem watcher: reflete a remoção relistando o pai.
        if (remote) relist(parentOf(entry.rel_path));
      } catch (e) {
        toastError(t("filesDeleteError"), e);
      }
    },
    [session.id, selected, t, remote, relist],
  );


  const selectedEntry = useMemo(() => {
    if (!selected) return null;
    const siblings = entriesByDir[parentOf(selected)] ?? [];
    return siblings.find((e) => e.rel_path === selected) ?? null;
  }, [selected, entriesByDir]);
  const rootLabel = info
    ? info.root.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || info.root
    : t("filesRootLabel");
  const driftLabel = info?.drifted_to
    ? info.drifted_to.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ||
      info.drifted_to
    : null;

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

  const inlineInput = (
    initial: string,
    placeholder: string,
    onCommit: (value: string) => void,
  ) => (
    <input
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
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
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {/* Fundo do painel lateral que o contém; cabeçalho como o dos mockups
          Painel*: `padding: 12px 16px`, ícone 16, título 13px. */}
      <header className="flex shrink-0 items-center gap-2.5 tyba-divide-b px-4 py-3">
        <FolderOpen size={16} className="shrink-0 text-tyba-cyan" />
        <span className="shrink-0 text-[13px] text-tyba-text">
          {t("filesTitle")}
        </span>
        {info && (
          <span
            title={info.root}
            className="min-w-0 truncate font-mono text-[11px] text-tyba-text-muted"
          >
            {rootLabel}
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

      {/* Só a árvore: clicar num arquivo abre uma ABA de editor ao lado das de
          terminal (ver `EditorTab`), não um visualizador dentro do painel. */}
      <div className="shrink-0 tyba-divide-b px-4 py-2.5">
        <label className="flex items-center gap-2 rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-1.5 text-[12px] text-tyba-text-faint">
          <MagnifyingGlass size={13} className="shrink-0" />
          <input
            type="text"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={treeQuery}
            onChange={(e) => setTreeQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && treeQuery) {
                e.preventDefault();
                setTreeQuery("");
              }
            }}
            placeholder={
              finderShortcut
                ? t("filesSearchPlaceholderShortcut", { shortcut: finderShortcut })
                : t("filesSearchPlaceholder")
            }
            aria-label={t("filesSearchPlaceholder")}
            className="min-w-0 flex-1 bg-transparent font-mono text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </label>
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          {treeHits !== null ? (
            <ul className="min-h-0 flex-1 overflow-auto px-2 py-1">
              {treeHits.length === 0 && (
                <li className="px-2 py-2 text-[11px] text-tyba-text-faint">
                  {t("filesSearchEmpty")}
                </li>
              )}
              {treeHits.map((rel) => {
                const deco = decorations.get(rel);
                return (
                  <li key={rel}>
                    <button
                      type="button"
                      onClick={() => openFile(rel)}
                      className="flex h-7 w-full items-center gap-2 rounded-[6px] px-2 text-left font-mono text-[12px] text-tyba-text-muted outline-none hover:bg-tyba-text/[.04] hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
                    >
                      <span className="min-w-0 flex-1 truncate" title={rel}>
                        {rel}
                      </span>
                      {deco && (
                        <span className={`shrink-0 text-[10px] ${DECO_CLASS[deco]}`}>
                          {DECO_LABEL[deco]}
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          ) : (
          <>
          <div className="flex h-6 shrink-0 items-center gap-1 tyba-divide-b px-2">
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
            className="min-h-0 flex-1 overflow-auto px-2 py-1"
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
                const renaming =
                  treeEdit?.kind === "rename" && treeEdit.rel === entry.rel_path;
                return (
                  <div
                    key={entry.rel_path}
                    onClick={() =>
                      entry.is_dir
                        ? toggleDir(entry.rel_path)
                        : openFile(entry.rel_path)
                    }
                    // Mockup PainelArquivos: linha de 24px, cantos de 5px,
                    // 12,5px, sem ícone de arquivo; o que mudou fica aceso e o
                    // apagado vem riscado em vermelho.
                    className={`group flex h-6 cursor-pointer items-center gap-1 rounded-[5px] pr-2 text-[12.5px] ${
                      isSelected
                        ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-row-raised)]"
                        : deco === "deleted"
                          ? "text-tyba-red hover:bg-tyba-text/[.04]"
                          : deco && deco !== "untracked"
                            ? "text-tyba-text hover:bg-tyba-text/[.04]"
                            : "text-tyba-text-muted hover:bg-tyba-text/[.04]"
                    } ${entry.gitignored ? "opacity-50" : ""}`}
                    style={{ paddingLeft: `${4 + depth * 15}px` }}
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
                    {renaming ? (
                      inlineInput(treeEdit.name, entry.name, commitTreeEdit)
                    ) : (
                      <span
                        className={`min-w-0 flex-1 truncate ${
                          deco === "deleted" ? "line-through" : ""
                        }`}
                      >
                        {entry.name}
                      </span>
                    )}
                    {!renaming && !entry.is_dir && isSensitivePath(entry.name) && (
                      <span
                        title={t("filesProtectedHint")}
                        className="shrink-0 text-[11px] text-tyba-text-faint group-hover:hidden"
                      >
                        {t("filesProtected")}
                      </span>
                    )}
                    {!renaming && deco && (
                      <span
                        className={`shrink-0 font-mono text-[11px] ${DECO_CLASS[deco]} group-hover:hidden`}
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
          </>
          )}
          {selected && !editorOpen && (
            <FilesFooter key={selected} session={session.id} rel={selected} remote={remote} />
          )}
        </div>
      </div>
    </div>
  );
}
