// Abas de conteúdo: editor de arquivo e container (logs, shell, arquivos).
//
// A aba em si — quem existe, qual está ativa, o histórico do ⌘[ — vive no
// core (`layout`). Daqui saem só as intenções (`openEditorTab`,
// `openContainerTab`) e o que é efêmero de tela: a linha para onde rolar ao
// abrir, quais editores têm alteração não salva (o buffer é do CodeMirror) e o
// nome do container para o rótulo da aba.

import { useSyncExternalStore } from "react";

import {
  containerOpenTab,
  openDiffContentTab,
  openEditorTabCmd,
  openGitLogTab,
  type ContainerTabKind,
  type EditorTabOpened,
  type SessionId,
} from "./ipc";

export const EDITOR_PREFIX = "editor:";
export const CONTAINER_LOGS_PREFIX = "container-logs:";
export const CONTAINER_FILES_PREFIX = "container-files:";
export const SESSION_DIFF_PREFIX = "session-diff:";
export const GIT_LOG_PREFIX = "git-log:";

export type ContentView =
  | { kind: "editor"; session: SessionId; rel: string }
  | { kind: "container-logs"; containerId: string }
  | { kind: "session-diff"; session: SessionId }
  | { kind: "git-log"; session: SessionId };

const UUID_LEN = 36;

/** `editor:<sessão>:<caminho>` / `container-logs:<id>` → partes. */
export function parseContentView(view: string | null | undefined): ContentView | null {
  if (!view) return null;
  if (view.startsWith(EDITOR_PREFIX)) {
    const rest = view.slice(EDITOR_PREFIX.length);
    const session = rest.slice(0, UUID_LEN);
    if (rest.charAt(UUID_LEN) !== ":" || session.length !== UUID_LEN) return null;
    const rel = rest.slice(UUID_LEN + 1);
    return rel ? { kind: "editor", session, rel } : null;
  }
  if (view.startsWith(SESSION_DIFF_PREFIX)) {
    const session = view.slice(SESSION_DIFF_PREFIX.length);
    return session.length === UUID_LEN ? { kind: "session-diff", session } : null;
  }
  if (view.startsWith(GIT_LOG_PREFIX)) {
    const session = view.slice(GIT_LOG_PREFIX.length);
    return session.length === UUID_LEN ? { kind: "git-log", session } : null;
  }
  if (view.startsWith(CONTAINER_LOGS_PREFIX)) {
    const containerId = view.slice(CONTAINER_LOGS_PREFIX.length);
    return containerId ? { kind: "container-logs", containerId } : null;
  }
  return null;
}

/** Painel lateral `container-files:<id>` → id. */
export function containerFilesTarget(sideView: string | null | undefined): string | null {
  if (!sideView?.startsWith(CONTAINER_FILES_PREFIX)) return null;
  return sideView.slice(CONTAINER_FILES_PREFIX.length) || null;
}

export function editorView(session: SessionId, rel: string): string {
  return `${EDITOR_PREFIX}${session}:${rel}`;
}

// --- estado efêmero, com assinatura ----------------------------------------

let version = 0;
const listeners = new Set<() => void>();
function bump() {
  version += 1;
  for (const fn of listeners) fn();
}
function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

/** Re-renderiza quem mostra rótulo/ponto de não-salvo das abas. */
export function useContentTabsVersion(): number {
  return useSyncExternalStore(subscribe, () => version);
}

const reveals = new Map<string, number>();

/** Linha pedida para a aba `view` (consome: só a primeira montagem rola). */
export function takeReveal(view: string): number | undefined {
  const line = reveals.get(view);
  reveals.delete(view);
  return line;
}

export function requestReveal(view: string, line: number) {
  reveals.set(view, line);
  bump();
}

const previews = new Set<string>();

/** A aba abre em "Visualizar" (Markdown vindo do painel Arquivos). Consome. */
export function takePreview(view: string): boolean {
  return previews.delete(view);
}

const dirty = new Set<string>();

export function setEditorDirty(view: string, isDirty: boolean) {
  if (isDirty === dirty.has(view)) return;
  if (isDirty) dirty.add(view);
  else dirty.delete(view);
  bump();
}

export function isEditorDirty(view: string | null | undefined): boolean {
  return !!view && dirty.has(view);
}

/** Editores com alteração não salva que pertencem a uma destas sessões —
 *  fechar o terminal delas fecha o editor junto. */
export function dirtyEditorsOf(sessions: readonly SessionId[]): string[] {
  return [...dirty].filter((view) => {
    const parsed = parseContentView(view);
    return parsed?.kind === "editor" && sessions.includes(parsed.session);
  });
}

const containerNames = new Map<string, string>();

export function rememberContainerName(id: string, name: string) {
  if (!name || containerNames.get(id) === name) return;
  containerNames.set(id, name);
  bump();
}

export function containerName(id: string): string {
  return containerNames.get(id) ?? id.slice(0, 12);
}

// --- intenções ----------------------------------------------------------------

/**
 * Abre (ou foca) a aba de container. `logs` e `shell` viram abas ao lado das
 * de terminal do workspace em uso; `files` abre o painel lateral só leitura.
 */
export function openContainerTab(
  kind: ContainerTabKind,
  containerId: string,
  opts?: { host?: string | null; name?: string },
): Promise<void> {
  if (opts?.name) rememberContainerName(containerId, opts.name);
  return containerOpenTab(kind, containerId, opts?.host ?? null);
}

/**
 * Abre (ou foca) a aba de editor. `path` absoluto (link de log já mapeado para
 * o host) ou relativo à raiz/repo da sessão; sem `session`, o core tenta as do
 * workspace ativo. `line` é 1-based.
 */
export async function openEditorTab(
  path: string,
  line?: number,
  opts?: { session?: SessionId; preview?: boolean },
): Promise<EditorTabOpened> {
  const opened = await openEditorTabCmd(opts?.session ?? null, path);
  if (opts?.preview) previews.add(opened.view);
  if (line && line > 0) requestReveal(opened.view, line);
  return opened;
}

/** Diff da sessão como aba (o conteúdo é o mesmo do painel de diff). */
export function openDiffTab(sessionId: SessionId): Promise<void> {
  return openDiffContentTab(sessionId);
}

/** Histórico (log + grafo) do repo da sessão como aba. */
export function openHistoryTab(sessionId: SessionId): Promise<void> {
  return openGitLogTab(sessionId);
}

// --- apresentação -----------------------------------------------------------

export function baseName(rel: string): string {
  const i = rel.lastIndexOf("/");
  return i < 0 ? rel : rel.slice(i + 1);
}

const GLYPHS: Record<string, { text: string; className: string }> = {
  js: { text: "JS", className: "text-tyba-amber" },
  jsx: { text: "JS", className: "text-tyba-amber" },
  mjs: { text: "JS", className: "text-tyba-amber" },
  cjs: { text: "JS", className: "text-tyba-amber" },
  ts: { text: "TS", className: "text-tyba-blue" },
  tsx: { text: "TS", className: "text-tyba-blue" },
  rs: { text: "RS", className: "text-tyba-amber" },
  py: { text: "PY", className: "text-tyba-blue" },
  go: { text: "GO", className: "text-tyba-blue" },
  json: { text: "{}", className: "text-tyba-amber" },
  md: { text: "MD", className: "text-tyba-text-muted" },
  css: { text: "CSS", className: "text-tyba-blue" },
  html: { text: "<>", className: "text-tyba-amber" },
  yml: { text: "YML", className: "text-tyba-red" },
  yaml: { text: "YML", className: "text-tyba-red" },
  toml: { text: "TML", className: "text-tyba-text-muted" },
  sh: { text: "SH", className: "text-tyba-green" },
};

/** O selo de linguagem da aba de editor (`JS`, `TS`, …). */
export function langGlyph(rel: string): { text: string; className: string } {
  const name = baseName(rel).toLowerCase();
  const dot = name.lastIndexOf(".");
  const ext = dot < 0 ? "" : name.slice(dot + 1);
  return GLYPHS[ext] ?? { text: "·", className: "text-tyba-text-faint" };
}

const LANG_NAMES: Record<string, string> = {
  js: "JavaScript",
  jsx: "JavaScript",
  mjs: "JavaScript",
  cjs: "JavaScript",
  ts: "TypeScript",
  tsx: "TypeScript",
  rs: "Rust",
  py: "Python",
  go: "Go",
  json: "JSON",
  md: "Markdown",
  css: "CSS",
  html: "HTML",
  yml: "YAML",
  yaml: "YAML",
  toml: "TOML",
  sh: "Shell",
  sql: "SQL",
  java: "Java",
};

export function languageName(rel: string): string {
  const name = baseName(rel).toLowerCase();
  const dot = name.lastIndexOf(".");
  const ext = dot < 0 ? "" : name.slice(dot + 1);
  return LANG_NAMES[ext] ?? (ext ? ext.toUpperCase() : "Texto");
}

export function isMarkdownPath(rel: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(rel);
}

export function eolOf(text: string): "LF" | "CRLF" {
  return text.includes("\r\n") ? "CRLF" : "LF";
}
