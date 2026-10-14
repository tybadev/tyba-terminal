import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ClockCounterClockwise,
  FilePlus,
  Folder,
  FolderOpen,
  FolderSimplePlus,
  MagnifyingGlass,
  X,
  type Icon,
} from "@phosphor-icons/react";

import "./gitScreen.css";
import { EditorTab } from "./EditorTab";
import { FilesTree } from "./FilesTree";
import { requestConfirm } from "../lib/confirm";
import {
  baseName,
  isEditorDirty,
  langGlyph,
  openEditorTab,
  useContentTabsVersion,
} from "../lib/contentTabs";
import { translateError } from "../lib/errors";
import { fileIcon } from "../lib/fileIcon";
import {
  filesGrep,
  filesPanelInfo,
  filesScreenActivate,
  filesScreenClose,
  filesScreenPin,
  filesScreenSection,
  type FilesPanelInfo,
  type FilesSection,
  type GrepOutcome,
  type RepoSnapshot,
  type Session,
  type SessionId,
  type Workspace,
} from "../lib/ipc";
import {
  FILES_SECTIONS,
  activeFile,
  escClosesScreen,
  recentFiles,
  sessionFiles,
  shouldPin,
  type ScreenFile,
} from "../lib/screens";
import { toastError } from "../lib/toast";

const RAIL: Record<FilesSection, { icon: Icon; label: string }> = {
  files: { icon: Folder, label: "filesScreenFiles" },
  search: { icon: MagnifyingGlass, label: "filesScreenSearch" },
  recent: { icon: ClockCounterClockwise, label: "filesScreenRecent" },
};

const BTN =
  "flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[6px] border border-tyba-border-strong px-2.5 py-1 text-[12px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50";

const SEARCH_DEBOUNCE_MS = 220;

/** "Buscar no projeto": texto nos arquivos rastreados, agrupado por arquivo. */
function SearchColumn({ session, onOpen }: { session: Session; onOpen: (rel: string, line: number) => void }) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<GrepOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setResult(null);
      setError(null);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      setBusy(true);
      void filesGrep(session.id, q)
        .then((r) => {
          if (!alive) return;
          setResult(r);
          setError(null);
        })
        .catch((e) => {
          if (alive) setError(translateError(e, t));
        })
        .finally(() => {
          if (alive) setBusy(false);
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [query, session.id, t]);

  const groups = useMemo(() => {
    const out: { path: string; hits: GrepOutcome["hits"] }[] = [];
    for (const hit of result?.hits ?? []) {
      const last = out[out.length - 1];
      if (last && last.path === hit.path) last.hits.push(hit);
      else out.push({ path: hit.path, hits: [hit] });
    }
    return out;
  }, [result]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="shrink-0 border-b border-[var(--tyba-border-soft)] px-3 py-2.5">
        <label className="flex items-center gap-2 rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-[5px] text-[12px] text-tyba-text-faint">
          <MagnifyingGlass size={13} className={`shrink-0 ${busy ? "animate-pulse" : ""}`} />
          <input
            type="text"
            autoFocus
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query) {
                e.preventDefault();
                setQuery("");
              }
            }}
            placeholder={t("filesScreenSearchPlaceholder")}
            aria-label={t("filesScreenSearchPlaceholder")}
            className="min-w-0 flex-1 bg-transparent text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </label>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-1.5 py-2 text-[12px]">
        {error ? (
          <p role="alert" className="px-2.5 py-1 text-[11px] text-tyba-red">
            {error}
          </p>
        ) : result === null ? (
          <p className="px-2.5 py-1 text-[11px] leading-4 text-tyba-text-faint">
            {t("filesScreenSearchHint")}
          </p>
        ) : groups.length === 0 ? (
          <p className="px-2.5 py-1 text-[11px] text-tyba-text-faint">{t("filesScreenSearchEmpty")}</p>
        ) : (
          groups.map((g) => {
            const GIcon = fileIcon(baseName(g.path));
            return (
              <section key={g.path} className="pb-1.5">
                <div
                  className="flex h-[26px] items-center gap-[7px] px-2.5 text-[12.5px] text-tyba-text"
                  title={g.path}
                >
                  <GIcon size={15} className={`shrink-0 ${langGlyph(g.path).className}`} />
                  <span className="min-w-0 flex-1 truncate">{g.path}</span>
                  <span className="shrink-0 text-[11px] text-tyba-text-faint">{g.hits.length}</span>
                </div>
                {g.hits.map((hit) => (
                  <button
                    key={`${hit.line}`}
                    type="button"
                    onClick={() => onOpen(hit.path, hit.line)}
                    className="flex w-full items-baseline gap-2 rounded-[5px] py-0.5 pl-[32px] pr-2.5 text-left font-mono text-[11.5px] text-tyba-text-muted outline-none hover:bg-tyba-text/[.04] hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
                  >
                    <span className="w-8 shrink-0 text-right text-tyba-text-faint">{hit.line}</span>
                    <span className="min-w-0 flex-1 truncate">{hit.text.trim()}</span>
                  </button>
                ))}
              </section>
            );
          })
        )}
        {result?.truncated && (
          <p className="px-2.5 py-1 text-[11px] text-tyba-text-faint">
            {t("filesScreenSearchTruncated", { count: result.hits.length })}
          </p>
        )}
      </div>
    </div>
  );
}

function RecentColumn({ rels, onOpen }: { rels: string[]; onOpen: (rel: string) => void }) {
  const { t } = useTranslation();
  return (
    <div className="min-h-0 flex-1 overflow-auto px-1.5 py-2">
      {rels.length === 0 ? (
        <p className="px-2.5 py-1 text-[11px] text-tyba-text-faint">{t("filesScreenRecentEmpty")}</p>
      ) : (
        rels.map((rel) => {
          const RIcon = fileIcon(baseName(rel));
          return (
            <button
              key={rel}
              type="button"
              onClick={() => onOpen(rel)}
              title={rel}
              className="flex h-[26px] w-full items-center gap-[7px] rounded-[5px] px-2.5 text-left text-[12.5px] text-tyba-text-muted outline-none hover:bg-tyba-text/[.04] hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
            >
              <RIcon size={15} className="shrink-0 text-tyba-text-faint" />
              <span className="shrink-0">{baseName(rel)}</span>
              <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-faint">
                {rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : ""}
              </span>
            </button>
          );
        })
      )}
    </div>
  );
}

/**
 * Tela cheia de Arquivos (mockup ArquivosTela): mesmo esqueleto da de Git —
 * trilho à direita, barra de topo, coluna de 280px e o editor com abas
 * INTERNAS da tela. Os editores ficam montados mesmo com a tela fechada (o
 * buffer não salvo é do CodeMirror); fechada, a tela só some.
 */
export function FilesScreen({
  open,
  exiting,
  workspace,
  session,
  editors,
  findSession,
  snapshot,
  editorPref,
  finderShortcut,
  rootLabelOf,
  onClose,
  onOpenDiff,
  onRunInTerminal,
}: {
  /** A tela está aberta no workspace em uso. */
  open: boolean;
  exiting: boolean;
  workspace: Workspace | null;
  /** Sessão da tela (`files:<id>`); `null` com a tela fechada. */
  session: Session | null;
  /** Todos os editores de telas de Arquivos (ver `allScreenEditors`). */
  editors: (ScreenFile & { workspace: string })[];
  findSession: (id: SessionId) => Session | undefined;
  snapshot: RepoSnapshot | undefined;
  editorPref: string;
  finderShortcut?: string;
  rootLabelOf: (session: Session, workspace: string) => string;
  onClose: () => void;
  onOpenDiff: (session: SessionId) => void;
  onRunInTerminal: (command: string) => void;
}) {
  const { t } = useTranslation();
  useContentTabsVersion();
  const visible = (open || exiting) && session !== null && workspace !== null;
  const files = workspace?.screens?.files ?? null;
  const section = files?.section ?? "files";
  const tabs = files && session ? sessionFiles(files, session.id) : [];
  const activeView = files && session ? activeFile(files, session.id) : null;
  const activeRel = tabs.find((f) => f.view === activeView)?.rel ?? null;
  const [info, setInfo] = useState<FilesPanelInfo | null>(null);
  const [createRequest, setCreateRequest] = useState<{ isDir: boolean } | null>(null);

  const sectionRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (open) sectionRef.current?.focus({ preventScroll: true });
  }, [open, session?.id]);

  useEffect(() => {
    if (!open || !session) return;
    let alive = true;
    void filesPanelInfo(session.id)
      .then((i) => {
        if (alive) setInfo(i);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [open, session]);

  // Editar a aba em pré-visualização a fixa: o core é quem guarda.
  const workspaceId = workspace?.id ?? null;
  const toPin = tabs
    .filter((f) => shouldPin(f.preview, isEditorDirty(f.view)))
    .map((f) => f.view)
    .join("\n");
  useEffect(() => {
    if (!workspaceId || !toPin) return;
    for (const view of toPin.split("\n")) {
      void filesScreenPin(workspaceId, view).catch(() => {});
    }
  }, [workspaceId, toPin]);

  const go = (next: FilesSection) => {
    if (workspace) void filesScreenSection(workspace.id, next).catch(() => {});
  };

  const openRel = (rel: string, line?: number, preview = false) => {
    if (!session) return;
    void openEditorTab(rel, line, { session: session.id, preview }).catch((e) =>
      toastError(t("editorOpenError"), e),
    );
  };

  const closeTab = async (view: string) => {
    if (!workspace) return;
    if (isEditorDirty(view)) {
      const ok = await requestConfirm({
        title: t("filesDiscardTitle"),
        detail: t("filesDiscardDetail"),
        confirmLabel: t("filesDiscardConfirm"),
        destructive: true,
      });
      if (!ok) return;
    }
    void filesScreenClose(workspace.id, view).catch(() => {});
  };

  const create = (isDir: boolean) => {
    if (section !== "files") go("files");
    setCreateRequest({ isDir });
  };

  const changed = snapshot?.status?.changed ?? 0;

  return (
    <section
      ref={sectionRef}
      tabIndex={-1}
      aria-label={t("filesTitle")}
      aria-hidden={!visible}
      data-exiting={exiting}
      className={`tyba-git-screen absolute inset-0 z-30 flex-row-reverse bg-tyba-bg outline-none ${
        visible ? "flex" : "hidden"
      }`}
      onKeyDown={(e) => {
        if (
          escClosesScreen({
            key: e.key,
            defaultPrevented: e.defaultPrevented,
            target: e.target as HTMLElement,
          })
        ) {
          e.preventDefault();
          onClose();
        }
      }}
    >
      {visible && (
        <nav
          aria-label={t("filesScreenSections")}
          className="relative flex w-[52px] shrink-0 flex-col items-center gap-1.5 tyba-divide-l bg-[var(--tyba-panel-bg)] pt-3"
        >
          {FILES_SECTIONS.map((key) => {
            const { icon: SectionIcon, label } = RAIL[key];
            const on = key === section;
            return (
              <button
                key={key}
                type="button"
                aria-label={t(label)}
                aria-pressed={on}
                title={t(label)}
                onClick={() => go(key)}
                className={`relative flex size-9 items-center justify-center rounded-[8px] transition-colors ${
                  on ? "bg-tyba-cyan/[.12] text-tyba-cyan" : "text-tyba-text-faint hover:text-tyba-text"
                }`}
              >
                <SectionIcon size={17} />
              </button>
            );
          })}
          <span className="flex-1" />
          <button
            type="button"
            aria-label={t("filesScreenClose")}
            title={t("filesScreenClose")}
            onClick={onClose}
            className="mb-2.5 flex size-9 items-center justify-center rounded-[8px] text-tyba-text-faint hover:text-tyba-text"
          >
            <X size={15} />
          </button>
        </nav>
      )}

      <div className="flex min-w-0 flex-1 flex-col">
        {visible && session && (
          <div className="flex h-[46px] shrink-0 items-center gap-3 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px]">
            <FolderOpen size={16} className="shrink-0 text-tyba-cyan" />
            <span className="shrink-0 text-[14px] font-medium text-tyba-text">{t("filesTitle")}</span>
            <span className="min-w-0 flex-1 truncate text-tyba-text-faint" title={info?.root}>
              {info?.root ?? session.title}
            </span>
            {snapshot?.branch && (
              <span className="shrink-0 font-mono text-tyba-blue">{snapshot.branch}</span>
            )}
            {changed > 0 && (
              <span className="shrink-0 text-tyba-text-muted">
                {t("filesScreenChanges", { count: changed })}
              </span>
            )}
            <button type="button" className={BTN} onClick={() => create(false)}>
              <FilePlus size={13} />
              {t("filesNewFile")}
            </button>
            <button type="button" className={BTN} onClick={() => create(true)}>
              <FolderSimplePlus size={13} />
              {t("filesNewFolder")}
            </button>
          </div>
        )}

        <div className="flex min-h-0 flex-1">
          {visible && session && (
            <div className="flex w-[280px] shrink-0 flex-col border-r border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)]">
              {section === "files" && (
                <FilesTree
                  key={session.id}
                  session={session}
                  onOpenFile={(rel, preview) => openRel(rel, undefined, preview)}
                  onJumpToDiff={() => onOpenDiff(session.id)}
                  finderShortcut={finderShortcut}
                  activeRel={activeRel}
                  createRequest={createRequest}
                  onCreateHandled={() => setCreateRequest(null)}
                />
              )}
              {section === "search" && (
                <SearchColumn
                  key={session.id}
                  session={session}
                  onOpen={(rel, line) => openRel(rel, line)}
                />
              )}
              {section === "recent" && files && (
                <RecentColumn rels={recentFiles(files, session.id)} onOpen={(rel) => openRel(rel)} />
              )}
            </div>
          )}

          <div className="flex min-w-0 flex-1 flex-col bg-tyba-sunken">
            {visible && tabs.length > 0 && (
              <div
                role="tablist"
                aria-label={t("filesScreenTabs")}
                className="flex h-[34px] shrink-0 items-end overflow-x-auto tyba-divide-b bg-[var(--tyba-panel-bg)]"
              >
                {tabs.map((f) => {
                  const on = f.view === activeView;
                  const dirty = isEditorDirty(f.view);
                  const TIcon = fileIcon(baseName(f.rel));
                  return (
                    <div
                      key={f.view}
                      role="tab"
                      aria-selected={on}
                      tabIndex={0}
                      title={f.preview ? `${f.rel}\n${t("filesScreenPreviewHint")}` : f.rel}
                      onClick={() =>
                        workspace && void filesScreenActivate(workspace.id, f.view).catch(() => {})
                      }
                      onDoubleClick={() =>
                        workspace && void filesScreenPin(workspace.id, f.view).catch(() => {})
                      }
                      onAuxClick={(e) => {
                        if (e.button === 1) void closeTab(f.view);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && workspace) {
                          void filesScreenActivate(workspace.id, f.view).catch(() => {});
                        }
                      }}
                      // Ativa: fundo do editor com fio dos dois lados (mockup).
                      className={`group flex h-[34px] shrink-0 cursor-pointer items-center gap-2 px-3.5 text-[12.5px] outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-tyba-cyan ${
                        on
                          ? "bg-tyba-sunken text-tyba-text shadow-[inset_1px_0_0_var(--tyba-border-soft),inset_-1px_0_0_var(--tyba-border-soft)]"
                          : "text-tyba-text-faint hover:text-tyba-text"
                      }`}
                    >
                      <TIcon
                        size={15}
                        className={`shrink-0 ${on ? langGlyph(f.rel).className : ""}`}
                      />
                      <span className={f.preview ? "italic" : ""}>{baseName(f.rel)}</span>
                      <span className="relative flex size-4 shrink-0 items-center justify-center">
                        {dirty && (
                          <span
                            aria-label={t("filesScreenUnsaved")}
                            title={t("filesScreenUnsaved")}
                            className="size-[7px] rounded-full bg-tyba-text group-hover:hidden"
                          />
                        )}
                        <button
                          type="button"
                          aria-label={t("filesScreenCloseTab", { name: baseName(f.rel) })}
                          title={t("filesScreenCloseTab", { name: baseName(f.rel) })}
                          onClick={(e) => {
                            e.stopPropagation();
                            void closeTab(f.view);
                          }}
                          className={`absolute inset-0 items-center justify-center rounded-[3px] text-tyba-text-faint hover:bg-tyba-text/[.08] hover:text-tyba-text ${
                            dirty ? "hidden group-hover:flex" : on ? "flex" : "hidden group-hover:flex"
                          }`}
                        >
                          <X size={11} />
                        </button>
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
            <div className="relative min-h-0 flex-1">
              {visible && tabs.length === 0 && (
                <div className="absolute inset-0 flex items-center justify-center px-6 text-center text-[12px] text-tyba-text-faint">
                  {finderShortcut
                    ? t("filesScreenEmpty", { shortcut: finderShortcut })
                    : t("filesScreenEmptyNoShortcut")}
                </div>
              )}
              {editors.map((e) => {
                const owner = findSession(e.session);
                if (!owner) return null;
                return (
                  <EditorTab
                    key={e.view}
                    session={owner}
                    rel={e.rel}
                    view={e.view}
                    active={open && !exiting && e.view === activeView}
                    editor={editorPref}
                    rootLabel={rootLabelOf(owner, e.workspace)}
                    onOpenDiff={() => onOpenDiff(e.session)}
                    onRunInTerminal={onRunInTerminal}
                  />
                );
              })}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
