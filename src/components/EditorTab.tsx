import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowSquareOut, Warning, Wrench } from "@phosphor-icons/react";

import { GitIcon } from "./icons/GitIcon";

import type {
  GutterMarker,
  LspDiagnostic,
  LspLocation,
  LspQuickFix,
  Session,
} from "@/lib/ipc";
import {
  filesRead,
  filesEditBegin,
  filesEditEnd,
  filesGutter,
  filesOpenExternal,
  filesPanelInfo,
  filesWrite,
  lspChange,
  lspCloseDoc,
  lspCodeActions,
  lspCompletion,
  lspDefinition,
  lspDidSave,
  lspHover,
  lspOpen,
  lspOpenExternal,
  lspSignature,
  lspSymbolPath,
  onFilesConflict,
  onFilesGutter,
  onLspDiagnostics,
} from "@/lib/ipc";
import type { LspBridge } from "@/lib/cmLsp";
import { createChangeBatcher } from "@/lib/lspChangeBatch";
import {
  eolOf,
  isMarkdownPath,
  takePreview,
  languageName,
  openEditorTab,
  setEditorDirty,
  takeReveal,
  useContentTabsVersion,
} from "@/lib/contentTabs";
import { requestConfirm } from "@/lib/confirm";
import { IS_MAC } from "@/lib/platform";
import { toastError } from "@/lib/toast";
import { getEffectiveBase, onEffectiveBaseChange } from "@/theme";
import { lineDiff } from "@/lib/lineDiff";
import { CodeEditor, type CodeEditorHandle } from "./CodeEditor";
import { HighlightedSource, MarkdownView } from "./MarkdownView";
import { LspBanners, LspStatusChip, useLspStatus } from "./LspStatusUi";
import { LoadingState } from "./ui/loading";

interface Props {
  session: Session;
  rel: string;
  view: string;
  active: boolean;
  /** Nome do editor externo preferido (Configurações). */
  editor: string;
  /** Rótulo da raiz (nome do repo) no começo do breadcrumb. */
  rootLabel: string;
  onOpenDiff: () => void;
  onRunInTerminal: (command: string) => void;
}

type Loaded =
  | { kind: "loading"; at: number }
  | { kind: "error"; message: string }
  | { kind: "ready"; text: string; version: number };

const LSP_LIVE = new Set(["starting", "restarting", "available", "installing"]);
/** Folga do didChange em lote: digitando, o server recebe o texto a cada pausa. */
const LSP_CHANGE_DELAY_MS = 120;

/**
 * Aba de editor: um arquivo da sessão, sempre editável. Fica montada enquanto
 * a aba existe (escondida quando inativa) — o buffer não salvo é do
 * CodeMirror, e desmontar seria perdê-lo.
 */
export function EditorTab({
  session,
  rel,
  view,
  active,
  editor,
  rootLabel,
  onOpenDiff,
  onRunInTerminal,
}: Props) {
  const { t } = useTranslation();
  const [loaded, setLoaded] = useState<Loaded>(() => ({ kind: "loading", at: Date.now() }));
  const [savedHash, setSavedHash] = useState("");
  const [dirty, setDirty] = useState(false);
  const [conflict, setConflict] = useState<{ diskHash: string | null } | null>(null);
  const [gutter, setGutter] = useState<GutterMarker[]>([]);
  const [diagnostics, setDiagnostics] = useState<LspDiagnostic[]>([]);
  const [diagsTruncated, setDiagsTruncated] = useState(false);
  const [cursor, setCursor] = useState({ line: 1, col: 1 });
  const [symbols, setSymbols] = useState<string[]>([]);
  const [fixes, setFixes] = useState<LspQuickFix[]>([]);
  const [isDark, setIsDark] = useState(() => getEffectiveBase() === "dark");
  const isMarkdown = isMarkdownPath(rel);
  // Markdown aberto pelo painel Arquivos chega lendo; "Editar" é um clique.
  const [mode, setMode] = useState<"edit" | "view">(() =>
    isMarkdownPath(rel) && takePreview(view) ? "view" : "edit",
  );
  const [showSource, setShowSource] = useState(false);
  const [previewText, setPreviewText] = useState<string | null>(null);
  const [conflictDiff, setConflictDiff] = useState<{ disk: string; edited: string } | null>(
    null,
  );
  const editorRef = useRef<CodeEditorHandle>(null);
  const savedHashRef = useRef(savedHash);
  savedHashRef.current = savedHash;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  const revealVersion = useContentTabsVersion();
  const remote = session.kind.type === "ssh";
  // No servidor, a etapa diz qual: o alias vem do core, uma vez por sessão.
  const [host, setHost] = useState<string | null>(null);
  useEffect(() => {
    if (!remote) return;
    let alive = true;
    void filesPanelInfo(session.id)
      .then((info) => {
        if (alive) setHost(info.host);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [session.id, remote]);
  // `lsp_open` é async no core (pode subir o server, fora da main thread):
  // o que fala do documento — mudança, pedido, salvar, fechar — sai depois
  // dele, na ordem em que foi pedido.
  const opened = useRef<Promise<unknown>>(Promise.resolve());
  const afterOpen = useCallback(
    <T,>(fn: () => Promise<T>): Promise<T> => opened.current.then(fn),
    [],
  );
  const changes = useMemo(
    () =>
      createChangeBatcher(
        (batch) => void afterOpen(() => lspChange(session.id, rel, batch)).catch(() => {}),
        LSP_CHANGE_DELAY_MS,
      ),
    [session.id, rel, afterOpen],
  );

  useEffect(
    () => onEffectiveBaseChange(() => setIsDark(getEffectiveBase() === "dark")),
    [],
  );

  const {
    stat: lspStat,
    setStat: setLspStat,
    refresh: refreshLsp,
    retry: retryLsp,
  } = useLspStatus(session.id, rel, remote);

  // Carga inicial: o arquivo já entra em edição (o slot de conflito do core
  // passa a vigiar este arquivo).
  useEffect(() => {
    let alive = true;
    setLoaded({ kind: "loading", at: Date.now() });
    void filesEditBegin(session.id, rel)
      .then((base) => {
        if (!alive) return;
        setSavedHash(base.hash);
        setLoaded({ kind: "ready", text: base.text, version: 1 });
        if (!remote) {
          const open = lspOpen(session.id, rel, base.text, true);
          opened.current = open.catch(() => {});
          void open
            .then((s) => {
              if (!alive) return;
              setLspStat(s);
              if (LSP_LIVE.has(s.state)) refreshLsp();
            })
            .catch(() => {});
        }
      })
      .catch((e) => {
        if (alive) setLoaded({ kind: "error", message: String(e) });
      });
    void filesGutter(session.id, rel)
      .then((m) => {
        if (alive) setGutter(m);
      })
      .catch(() => {});
    return () => {
      alive = false;
      changes.clear();
      if (!remote) void afterOpen(() => lspCloseDoc(session.id, rel)).catch(() => {});
      setEditorDirty(view, false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.id, rel]);

  // Voltou a ser a aba ativa: rearma o slot de conflito para este arquivo e
  // confere se o disco mudou enquanto ela estava escondida.
  const wasActive = useRef(active);
  useEffect(() => {
    const became = active && !wasActive.current;
    wasActive.current = active;
    if (!became || loaded.kind !== "ready") return;
    void filesEditBegin(session.id, rel)
      .then((base) => {
        if (base.hash === savedHashRef.current) return;
        if (dirtyRef.current) {
          setConflict({ diskHash: base.hash });
        } else {
          setSavedHash(base.hash);
          setLoaded((prev) =>
            prev.kind === "ready"
              ? { kind: "ready", text: base.text, version: prev.version + 1 }
              : prev,
          );
        }
      })
      .catch(() => {});
  }, [active, loaded.kind, session.id, rel]);

  useEffect(
    () => () => {
      if (wasActive.current) void filesEditEnd(session.id).catch(() => {});
    },
    [session.id],
  );

  useEffect(() => {
    const unGutter = onFilesGutter(session.id, (path, markers) => {
      if (path === rel) setGutter(markers);
    });
    const unConflict = onFilesConflict(session.id, (path, diskHash) => {
      if (path === rel && diskHash !== savedHashRef.current) setConflict({ diskHash });
    });
    const unDiag = onLspDiagnostics(session.id, (files, filesTruncated) => {
      if (filesTruncated) setDiagsTruncated(true);
      const mine = files.find((f) => f.path === rel);
      if (mine) setDiagnostics(mine.diagnostics);
    });
    return () => {
      void unGutter.then((f) => f());
      void unConflict.then((f) => f());
      void unDiag.then((f) => f());
    };
  }, [session.id, rel]);

  useEffect(() => setEditorDirty(view, dirty), [view, dirty]);

  // Linha pedida (link de log, ir para definição): rola quando o texto está
  // na tela. Reabrir a mesma aba com outra linha também chega aqui.
  useEffect(() => {
    if (loaded.kind !== "ready") return;
    const line = takeReveal(view);
    if (line) window.requestAnimationFrame(() => editorRef.current?.revealLine(line));
  }, [loaded, view, revealVersion]);

  const save = useCallback(async () => {
    const ed = editorRef.current;
    if (!ed) return;
    const value = ed.getValue();
    try {
      const result = await filesWrite(session.id, rel, value, savedHashRef.current);
      if (result.status === "written") {
        ed.markSaved();
        setSavedHash(result.hash);
        setDirty(false);
        setConflict(null);
        if (!remote) {
          changes.flush();
          void afterOpen(() => lspDidSave(session.id, rel)).catch(() => {});
        }
      } else {
        setConflict({ diskHash: result.disk_hash });
      }
    } catch (e) {
      toastError(t("filesSaveError"), e);
    }
  }, [session.id, rel, remote, changes, afterOpen, t]);

  const switchMode = useCallback((next: "edit" | "view") => {
    // O texto da leitura é o do buffer, não o do disco: o que se está editando.
    if (next === "view") setPreviewText(editorRef.current?.getValue() ?? null);
    setMode(next);
  }, []);

  const viewConflictDiff = useCallback(async () => {
    const edited = editorRef.current?.getValue() ?? "";
    let disk = "";
    let offset = 0;
    for (;;) {
      const page = await filesRead(session.id, rel, offset).catch(() => null);
      if (!page || page.kind !== "text") break;
      disk += page.text;
      if (!page.truncated) break;
      offset = page.next_offset;
    }
    setConflictDiff({ disk, edited });
  }, [session.id, rel]);

  const overwrite = useCallback(async () => {
    const ed = editorRef.current;
    if (!ed || !conflict) return;
    try {
      const result = await filesWrite(session.id, rel, ed.getValue(), conflict.diskHash ?? "");
      if (result.status === "written") {
        ed.markSaved();
        setSavedHash(result.hash);
        setDirty(false);
        setConflict(null);
        if (!remote) {
          changes.flush();
          void afterOpen(() => lspDidSave(session.id, rel)).catch(() => {});
        }
      } else {
        setConflict({ diskHash: result.disk_hash });
      }
    } catch (e) {
      toastError(t("filesSaveError"), e);
    }
  }, [conflict, session.id, rel, remote, changes, afterOpen, t]);

  const reload = useCallback(async () => {
    if (dirtyRef.current) {
      const ok = await requestConfirm({
        title: t("filesDiscardTitle"),
        detail: t("filesDiscardDetail"),
        confirmLabel: t("filesDiscardConfirm"),
        destructive: true,
      });
      if (!ok) return;
    }
    try {
      const base = await filesEditBegin(session.id, rel);
      setSavedHash(base.hash);
      setDirty(false);
      setConflict(null);
      setConflictDiff(null);
      setPreviewText(null);
      setLoaded((prev) => ({
        kind: "ready",
        text: base.text,
        version: prev.kind === "ready" ? prev.version + 1 : 1,
      }));
      changes.clear();
      if (!remote) {
        opened.current = lspOpen(session.id, rel, base.text, true).catch(() => {});
      }
    } catch (e) {
      toastError(t("filesEditError"), e);
    }
  }, [session.id, rel, remote, changes, afterOpen, t]);

  // ⌘S com a aba ativa, mesmo com o foco fora do editor (barra, breadcrumb).
  useEffect(() => {
    if (!active) return;
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
  }, [active, save]);

  const gotoDefinition = useCallback(
    async (loc: LspLocation) => {
      if (loc.in_root) {
        await openEditorTab(loc.path, loc.line + 1, { session: session.id }).catch((e) =>
          toastError(t("editorOpenError"), e),
        );
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
    [session.id, editor, t],
  );

  const bridge = useMemo<LspBridge | null>(() => {
    if (remote) return null;
    // Todo pedido sobre a posição atual manda antes o texto pendente.
    return {
      completion: (line, character) => {
        changes.flush();
        return afterOpen(() => lspCompletion(session.id, rel, line, character));
      },
      hover: (line, character) => {
        changes.flush();
        return afterOpen(() => lspHover(session.id, rel, line, character));
      },
      signature: (line, character) => {
        changes.flush();
        return afterOpen(() => lspSignature(session.id, rel, line, character));
      },
      definition: (line, character) => {
        changes.flush();
        return afterOpen(() => lspDefinition(session.id, rel, line, character));
      },
      change: changes.push,
      gotoDefinition: (loc) => void gotoDefinition(loc),
    };
  }, [session.id, rel, remote, gotoDefinition, changes, afterOpen]);

  const lspReady = lspStat?.state === "ready";

  // Símbolo sob o cursor (fim do breadcrumb) e correções da linha — pedidos
  // ao LSP com folga, não a cada tecla.
  const lineDiags = useMemo(
    () =>
      diagnostics.filter(
        (d) => cursor.line - 1 >= d.range.start.line && cursor.line - 1 <= d.range.end.line,
      ),
    [diagnostics, cursor.line],
  );
  useEffect(() => {
    if (!lspReady || !active) return;
    const timer = window.setTimeout(() => {
      changes.flush();
      void afterOpen(() => lspSymbolPath(session.id, rel, cursor.line - 1, cursor.col - 1))
        .then(setSymbols)
        .catch(() => {});
    }, 300);
    return () => window.clearTimeout(timer);
  }, [lspReady, active, session.id, rel, cursor.line, cursor.col, changes, afterOpen]);
  useEffect(() => {
    setFixes([]);
    if (!lspReady || lineDiags.length === 0) return;
    let alive = true;
    const timer = window.setTimeout(() => {
      changes.flush();
      void afterOpen(() => lspCodeActions(session.id, rel, lineDiags))
        .then((f) => {
          if (alive) setFixes(f);
        })
        .catch(() => {});
    }, 250);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [lspReady, lineDiags, session.id, rel, changes, afterOpen]);

  const errors = diagnostics.filter((d) => d.severity === 1).length;
  const warnings = diagnostics.filter((d) => d.severity === 2).length;
  const parts = rel.split("/");
  const text = loaded.kind === "ready" ? loaded.text : "";
  // Varre o arquivo inteiro: só quando o texto base muda, não a cada tecla.
  const eol = useMemo(() => eolOf(text), [text]);


  return (
    <div
      className={`absolute inset-0 flex min-h-0 min-w-0 flex-col bg-tyba-sunken ${
        active ? "" : "hidden"
      }`}
    >
      <div className="flex shrink-0 items-center gap-3.5 border-b border-[var(--tyba-border-soft)] px-4 py-1.5 text-[11px] text-tyba-text-faint">
        <nav
          aria-label={t("editorBreadcrumb")}
          className="flex min-w-0 flex-1 items-center gap-1 truncate font-mono"
        >
          <span>{rootLabel}</span>
          {parts.map((part, i) => (
            <span key={i} className="flex items-center gap-1">
              <span aria-hidden>›</span>
              <span className={i === parts.length - 1 ? "text-tyba-text" : ""}>{part}</span>
            </span>
          ))}
          {symbols.map((sym, i) => (
            <span key={`s${i}`} className="flex items-center gap-1">
              <span aria-hidden>›</span>
              <span className="text-tyba-text-muted">{sym}</span>
            </span>
          ))}
        </nav>
        {isMarkdown && (
          <div
            role="group"
            aria-label={t("editorMode")}
            className="flex shrink-0 overflow-hidden rounded-[5px] border border-tyba-border-field"
          >
            {(["edit", "view"] as const).map((m) => (
              <button
                key={m}
                type="button"
                aria-pressed={mode === m}
                onClick={() => switchMode(m)}
                className={`px-2 py-0.5 ${
                  mode === m
                    ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-edge)]"
                    : "hover:text-tyba-text"
                }`}
              >
                {m === "edit" ? t("editorModeEdit") : t("editorModeView")}
              </button>
            ))}
          </div>
        )}
        <button
          type="button"
          onClick={onOpenDiff}
          className="flex shrink-0 items-center gap-1 hover:text-tyba-text"
        >
          <GitIcon size={12} />
          {t("editorDiffHead")}
        </button>
        <button
          type="button"
          onClick={() =>
            void filesOpenExternal(session.id, rel, editor || undefined).catch((e) =>
              toastError(t("filesOpenExternal"), e),
            )
          }
          className="flex shrink-0 items-center gap-1 hover:text-tyba-text"
        >
          <ArrowSquareOut size={12} />
          {t("editorOpenDefault")}
        </button>
        <button
          type="button"
          onClick={() => void save()}
          disabled={loaded.kind !== "ready"}
          className="shrink-0 rounded-[5px] border border-tyba-border-field bg-tyba-raised px-2 py-0.5 text-tyba-text shadow-[var(--tyba-edge)] hover:bg-tyba-text/[.06] disabled:opacity-50"
        >
          {t("editorSave", { combo: IS_MAC ? "⌘S" : "Ctrl+S" })}
        </button>
      </div>

      {conflict && (
        <div
          role="alert"
          className="flex shrink-0 items-center gap-2 border-b border-tyba-amber/40 bg-tyba-amber/10 px-4 py-1.5 text-[11px] text-tyba-text"
        >
          <Warning size={13} className="shrink-0 text-tyba-amber" />
          <span className="min-w-0 flex-1 truncate">{t("filesConflictBanner")}</span>
          <button
            type="button"
            onClick={() => void viewConflictDiff()}
            className="rounded-[3px] px-1.5 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
          >
            {t("filesConflictDiff")}
          </button>
          <button
            type="button"
            onClick={() => void reload()}
            className="rounded-[3px] px-1.5 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
          >
            {t("filesConflictReload")}
          </button>
          <button
            type="button"
            onClick={() => void overwrite()}
            className="rounded-[3px] px-1.5 py-0.5 text-tyba-amber hover:bg-tyba-amber/20"
          >
            {t("filesConflictOverwrite")}
          </button>
        </div>
      )}

      {!remote && (
        <LspBanners
          session={session.id}
          rel={rel}
          stat={lspStat}
          setStat={setLspStat}
          refresh={refreshLsp}
          onRunInTerminal={onRunInTerminal}
        />
      )}

      <div className="relative min-h-0 flex-1">
        {loaded.kind === "loading" && (
          <LoadingState
            op="editor"
            remote={remote}
            step={
              remote && host
                ? t("loadFilesReadRemote", { host, path: rel })
                : t("loadFilesRead", { path: rel })
            }
            em={remote && host ? [host, rel] : [rel]}
            startedAt={loaded.at}
            slowCause={
              !remote ? t("loadFilesSlow") : host ? t("loadRemoteSlow", { host }) : undefined
            }
            shape="lines"
            rows={8}
            className="absolute inset-0"
          />
        )}
        {loaded.kind === "error" && (
          <div className="flex flex-col items-start gap-2 p-4 text-[12px] text-tyba-text-muted">
            <span>{loaded.message}</span>
            <button
              type="button"
              onClick={() =>
                void filesOpenExternal(session.id, rel, editor || undefined).catch((e) =>
                  toastError(t("filesOpenExternal"), e),
                )
              }
              className="text-tyba-blue hover:underline"
            >
              {t("editorOpenDefault")}
            </button>
          </div>
        )}
        {loaded.kind === "ready" && mode === "view" && (
          <div className="absolute inset-0 overflow-auto">
            <div className="flex justify-end px-4 pt-2">
              <button
                type="button"
                onClick={() => setShowSource((v) => !v)}
                className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
              >
                {showSource ? t("filesShowPreview") : t("filesShowSource")}
              </button>
            </div>
            {showSource ? (
              <HighlightedSource text={previewText ?? loaded.text} path={rel} dark={isDark} />
            ) : (
              <MarkdownView text={previewText ?? loaded.text} dark={isDark} />
            )}
          </div>
        )}
        {loaded.kind === "ready" && (
          <div className={mode === "view" ? "hidden" : "h-full"}>
          <CodeEditor
            key={loaded.version}
            ref={editorRef}
            doc={loaded.text}
            filename={rel}
            dark={isDark}
            markers={gutter}
            lsp={bridge}
            diagnostics={diagnostics}
            onDirtyChange={setDirty}
            onSave={() => void save()}
            onCursor={(line, col) => setCursor({ line, col })}
          />
          </div>
        )}
        {conflictDiff && (
          <div className="absolute inset-0 z-20 flex flex-col bg-tyba-bg">
            <div className="flex h-7 shrink-0 items-center gap-2 tyba-divide-b px-4">
              <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-muted">
                {t("filesConflictDiffTitle")}
              </span>
              <button
                type="button"
                onClick={() => setConflictDiff(null)}
                aria-label={t("tunnelsClose")}
                className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
              >
                ✕
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[12px] leading-[1.5]">
              {lineDiff(conflictDiff.disk, conflictDiff.edited).map((r, i) => (
                <div
                  key={i}
                  className={`whitespace-pre px-4 ${
                    r.kind === "added"
                      ? "bg-tyba-green/10 text-tyba-green"
                      : r.kind === "removed"
                        ? "bg-tyba-red/10 text-tyba-red"
                        : "text-tyba-text-muted"
                  }`}
                >
                  <span className="mr-2 select-none text-tyba-text-faint">
                    {r.kind === "added" ? "+" : r.kind === "removed" ? "-" : " "}
                  </span>
                  {r.text === "" ? " " : r.text}
                </div>
              ))}
            </div>
            <div className="flex shrink-0 items-center gap-3 tyba-divide-t px-4 py-1.5 text-[10px] text-tyba-text-faint">
              <span className="text-tyba-red">− {t("filesConflictDisk")}</span>
              <span className="text-tyba-green">+ {t("filesConflictEdited")}</span>
            </div>
          </div>
        )}
        {active && lineDiags.length > 0 && (
          <div
            role="status"
            className="absolute bottom-3 right-4 z-10 flex max-w-[460px] flex-col gap-1.5 rounded-[8px] border border-tyba-border-field bg-tyba-raised px-3 py-2.5 text-[12px] shadow-[var(--tyba-edge),var(--tyba-shadow-sm)]"
          >
            {lineDiags.slice(0, 3).map((d, i) => (
              <div key={i} className="flex gap-2">
                <span className={d.severity === 1 ? "text-tyba-red" : "text-tyba-amber"}>
                  {d.severity === 1 ? "✗" : "!"}
                </span>
                <span className="min-w-0 text-tyba-text">{d.message}</span>
              </div>
            ))}
            {(fixes.length > 0 || lineDiags[0]?.code != null) && (
              <div className="flex flex-wrap items-center gap-3 text-[11px] text-tyba-text-muted">
                {fixes.slice(0, 3).map((fix, i) => (
                  <button
                    key={i}
                    type="button"
                    onClick={() => editorRef.current?.applyEdits(fix.edits)}
                    className="flex items-center gap-1 text-tyba-green hover:underline"
                  >
                    <Wrench size={11} />
                    {t("editorQuickFix", { title: fix.title })}
                  </button>
                ))}
                {lineDiags[0]?.code != null && (
                  <span className="text-tyba-text-faint">
                    {lineDiags[0].source ? `${lineDiags[0].source}(${lineDiags[0].code})` : String(lineDiags[0].code)}
                  </span>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="flex h-[26px] shrink-0 items-center gap-4 border-t border-tyba-border bg-[var(--tyba-panel-bg)] px-4 text-[11px] text-tyba-text-faint">
        {!remote && (
          <LspStatusChip stat={lspStat} truncated={diagsTruncated} onRetry={retryLsp} />
        )}
        <span className={errors > 0 ? "text-tyba-red" : ""}>
          {t("editorErrors", { count: errors })}
        </span>
        <span className={warnings > 0 ? "text-tyba-amber" : ""}>
          {t("editorWarnings", { count: warnings })}
        </span>
        <span className="flex-1" />
        <span>{t("editorCursor", { line: cursor.line, col: cursor.col })}</span>
        <span>UTF-8</span>
        <span>{eol}</span>
        <span>{languageName(rel)}</span>
      </div>
    </div>
  );
}
