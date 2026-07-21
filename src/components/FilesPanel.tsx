import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ArrowsInSimple,
  ArrowsOutSimple,
  CaretDown,
  CaretRight,
  Crosshair,
  Folder,
  FolderOpen,
  PlusMinus,
  ArrowSquareOut,
  TreeStructure,
  X,
} from "@phosphor-icons/react";

import type {
  FileContent,
  FileDecoStatus,
  FileEntry,
  FilesPanelInfo,
  Session,
} from "@/lib/ipc";
import {
  filesDecorations,
  filesListDir,
  filesOpenExternal,
  filesPanelInfo,
  filesRead,
  filesReanchor,
  filesUnwatchDir,
  filesWatchDir,
  onFilesDecorations,
  onFilesTree,
} from "@/lib/ipc";
import { langOfPath } from "@/lib/diff";
import { fileIcon } from "@/lib/fileIcon";
import { highlightBlock, type TokenSpan } from "@/lib/highlight";
import { isRemoteUrl, safeMarkdownUrl } from "@/lib/markdownUrl";
import { getEffectiveBase, onEffectiveBaseChange } from "@/theme";

interface Props {
  session: Session;
  editor: string;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
  onJumpToDiff: () => void;
}

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

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

const MD_COMPONENTS: Components = {
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
      <a
        href={href || undefined}
        onClick={(e) => e.preventDefault()}
        className="text-tyba-green underline"
      >
        {children}
      </a>
    );
  },
};

export function FilesPanel({
  session,
  editor,
  expanded,
  onToggleExpand,
  onClose,
  onJumpToDiff,
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
  const [content, setContent] = useState<FileContent | null>(null);
  const [contentError, setContentError] = useState<string | null>(null);
  const [markdownSource, setMarkdownSource] = useState(false);
  const [tokens, setTokens] = useState<TokenSpan[][] | null>(null);
  const [dirMeta, setDirMeta] = useState<
    Record<string, { total: number; truncated: boolean }>
  >({});
  const [isDark, setIsDark] = useState(() => getEffectiveBase() === "dark");

  useEffect(
    () => onEffectiveBaseChange(() => setIsDark(getEffectiveBase() === "dark")),
    [],
  );

  const entriesRef = useRef(entriesByDir);
  entriesRef.current = entriesByDir;
  const reqId = useRef(0);

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
    void filesListDir(session.id, "")
      .then((listing) => {
        if (!alive) return;
        setEntriesByDir({ "": listing.entries });
        setDirMeta({ "": { total: listing.total, truncated: listing.truncated } });
      })
      .catch(() => {});
    void filesWatchDir(session.id, "").catch(() => {});
    void filesDecorations(session.id).catch(() => {});
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
    return () => {
      void unTree.then((f) => f());
      void unDeco.then((f) => f());
    };
  }, [session.id, relist]);

  const toggleDir = useCallback(
    (dir: string) => {
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

  const openFile = useCallback(
    (rel: string) => {
      const my = ++reqId.current;
      setSelected(rel);
      setContent(null);
      setContentError(null);
      setMarkdownSource(false);
      setTokens(null);
      void filesRead(session.id, rel, 0)
        .then((c) => {
          if (reqId.current === my) setContent(c);
        })
        .catch((e) => {
          if (reqId.current === my) setContentError(String(e));
        });
    },
    [session.id],
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

  const reanchor = useCallback(async () => {
    reqId.current += 1;
    await filesReanchor(session.id).catch(() => {});
    setExpandedDirs(new Set());
    setSelected(null);
    setContent(null);
    const listing = await filesListDir(session.id, "").catch(() => null);
    if (listing) {
      setEntriesByDir({ "": listing.entries });
      setDirMeta({ "": { total: listing.total, truncated: listing.truncated } });
    }
    const next = await filesPanelInfo(session.id).catch(() => null);
    if (next) setInfo(next);
  }, [session.id]);

  const isMarkdown = selected ? langOfPath(selected) === "markdown" : false;

  useEffect(() => {
    setTokens(null);
    if (!content || content.kind !== "text" || !selected) return;
    if (isMarkdown && !markdownSource) return;
    const lang = langOfPath(selected);
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

  const selectedDecorated = selected ? decorations.has(selected) : false;

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

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-tyba-bg">
      <header className="flex h-8 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
        <TreeStructure size={14} className="shrink-0 text-tyba-text-faint" />
        <span className="min-w-0 truncate text-[12px] text-tyba-text">
          {t("filesTitle")}
        </span>
        {info && (
          <span className="min-w-0 truncate font-mono text-[11px] text-tyba-text-muted">
            {info.root}
          </span>
        )}
        <div className="flex-1" />
        <button
          onClick={() => void reanchor()}
          aria-label={t("filesReanchor")}
          title={t("filesReanchor")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          <Crosshair size={14} />
        </button>
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

      <div className="flex min-h-0 flex-1">
        <div className="w-[240px] shrink-0 overflow-auto border-r border-tyba-border py-1">
          {rows.length === 0 ? (
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
              return (
                <div
                  key={entry.rel_path}
                  onClick={() =>
                    entry.is_dir
                      ? toggleDir(entry.rel_path)
                      : openFile(entry.rel_path)
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
                  <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                  {deco && (
                    <span
                      className={`shrink-0 font-mono text-[10px] ${DECO_CLASS[deco]}`}
                    >
                      {DECO_LABEL[deco]}
                    </span>
                  )}
                  {deco && (
                    <button
                      onClick={(ev) => {
                        ev.stopPropagation();
                        onJumpToDiff();
                      }}
                      aria-label={t("filesJumpToDiff")}
                      title={t("filesJumpToDiff")}
                      className="hidden size-4 shrink-0 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-green group-hover:flex"
                    >
                      <PlusMinus size={11} />
                    </button>
                  )}
                </div>
              );
            })
          )}
        </div>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {selected ? (
            <>
              <div className="flex h-7 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
                <span className="min-w-0 truncate font-mono text-[11px] text-tyba-text-muted">
                  {selected}
                </span>
                <div className="flex-1" />
                {isMarkdown && (
                  <button
                    onClick={() => setMarkdownSource((v) => !v)}
                    className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
                  >
                    {markdownSource ? t("filesShowPreview") : t("filesShowSource")}
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
                <button
                  onClick={() =>
                    void filesOpenExternal(session.id, selected, editor).catch(
                      (e) => setContentError(String(e)),
                    )
                  }
                  aria-label={t("filesOpenExternal")}
                  title={t("filesOpenExternal")}
                  className="flex size-5 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-text"
                >
                  <ArrowSquareOut size={13} />
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-auto">
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
                  <div className="files-markdown px-4 py-3 text-[13px] leading-relaxed text-tyba-text">
                    <ReactMarkdown
                      remarkPlugins={[remarkGfm]}
                      urlTransform={safeMarkdownUrl}
                      components={MD_COMPONENTS}
                    >
                      {content.text}
                    </ReactMarkdown>
                  </div>
                ) : (
                  <pre className="min-w-0 px-4 py-3 font-mono text-[12px] leading-[1.5] text-tyba-text">
                    {tokens
                      ? tokens.map((line, i) => (
                          <div key={i}>
                            {line.length === 0 ? (
                              "\n"
                            ) : (
                              line.map((tk, j) => (
                                <span key={j} style={{ color: tk.color }}>
                                  {tk.text}
                                </span>
                              ))
                            )}
                          </div>
                        ))
                      : content.text}
                  </pre>
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
