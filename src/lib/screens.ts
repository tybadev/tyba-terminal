// Telas cheias de Arquivos e Docker (feature telas-cheias): mesmo esqueleto da
// tela de Git. Qual tela está aberta é o `side_view` do workspace (`files:`,
// `docker:`, `git:`); o que cada uma tem aberto e selecionado é
// `workspace.screens`, também do core. Daqui só saem leituras desse estado.

import { parseContentView } from "./contentTabs";
import type {
  ContainerPane,
  DockerSection,
  FilesScreenState,
  FilesSection,
  ServicesSection,
  SessionId,
  Workspace,
  WorkspaceId,
} from "./ipc";
import { parseSideView } from "./sidePanel";

export type ScreenKind = "git" | "files" | "docker" | "services";

/** Ordem do trilho, de cima para baixo. */
export const FILES_SECTIONS: readonly FilesSection[] = ["files", "search", "recent"];
export const DOCKER_SECTIONS: readonly DockerSection[] = ["containers", "compose", "images", "volumes"];
export const CONTAINER_PANES: readonly ContainerPane[] = ["logs", "shell", "files", "details"];
export const SERVICES_SECTIONS: readonly ServicesSection[] = ["ports", "processes", "tunnels"];

/** O `side_view` é uma tela cheia (cobre abas e shell), não painel lateral? */
export function screenOf(
  sideView: string | null | undefined,
): { kind: ScreenKind; session: SessionId } | null {
  const view = parseSideView(sideView);
  if (!view) return null;
  if (
    view.kind === "git" ||
    view.kind === "files" ||
    view.kind === "docker" ||
    view.kind === "services"
  ) {
    return { kind: view.kind, session: view.session };
  }
  return null;
}

/** O que sobra para o slot lateral: tela cheia não ocupa coluna. */
export function sideSlotView(sideView: string | null): string | null {
  return screenOf(sideView) ? null : sideView;
}

export type ScreenFile = {
  view: string;
  session: SessionId;
  rel: string;
  preview: boolean;
  /** Aba só leitura com um arquivo compose (de qualquer projeto). */
  compose?: { project: string; index: number };
};

const COMPOSE_FILE_PREFIX = "compose-file:";
const UUID_LEN = 36;

/** `compose-file:<sessão>:<projeto>:<índice>:<nome>` → partes. */
export function parseComposeFileView(
  view: string,
): { session: SessionId; project: string; index: number; name: string } | null {
  if (!view.startsWith(COMPOSE_FILE_PREFIX)) return null;
  const rest = view.slice(COMPOSE_FILE_PREFIX.length);
  const session = rest.slice(0, UUID_LEN);
  if (session.length !== UUID_LEN || rest.charAt(UUID_LEN) !== ":") return null;
  const [project, index, ...name] = rest.slice(UUID_LEN + 1).split(":");
  const n = Number(index);
  if (!project || !Number.isInteger(n) || n < 0 || name.join(":") === "") return null;
  return { session, project, index: n, name: name.join(":") };
}

/** Abas internas da tela que pertencem à sessão dela, na ordem do core. */
export function sessionFiles(files: FilesScreenState, session: SessionId): ScreenFile[] {
  return files.tabs.flatMap((tab) => {
    const parsed = parseContentView(tab.view);
    if (parsed?.kind === "editor" && parsed.session === session) {
      return [{ view: tab.view, session, rel: parsed.rel, preview: tab.preview }];
    }
    const compose = parseComposeFileView(tab.view);
    return compose?.session === session
      ? [
          {
            view: tab.view,
            session,
            rel: `${compose.project}/${compose.name}`,
            preview: tab.preview,
            compose: { project: compose.project, index: compose.index },
          },
        ]
      : [];
  });
}

/** A aba em foco da sessão: a ativa do core se for dela, senão a primeira. */
export function activeFile(files: FilesScreenState, session: SessionId): string | null {
  const own = sessionFiles(files, session);
  if (files.active && own.some((f) => f.view === files.active)) return files.active;
  return own[0]?.view ?? null;
}

/** Recentes da sessão (mais recente primeiro), como caminho relativo. */
export function recentFiles(files: FilesScreenState, session: SessionId): string[] {
  return files.recent.flatMap((view) => {
    const parsed = parseContentView(view);
    return parsed?.kind === "editor" && parsed.session === session ? [parsed.rel] : [];
  });
}

/**
 * Todos os editores abertos em telas de Arquivos, de todos os workspaces: ficam
 * montados mesmo com a tela fechada, porque o buffer não salvo é do CodeMirror.
 */
export function allScreenEditors(
  workspaces: readonly Workspace[],
): (ScreenFile & { workspace: WorkspaceId })[] {
  const seen = new Set<string>();
  const out: (ScreenFile & { workspace: WorkspaceId })[] = [];
  for (const ws of workspaces) {
    for (const tab of ws.screens?.files.tabs ?? []) {
      const parsed = parseContentView(tab.view);
      if (parsed?.kind !== "editor" || seen.has(tab.view)) continue;
      seen.add(tab.view);
      out.push({
        workspace: ws.id,
        view: tab.view,
        session: parsed.session,
        rel: parsed.rel,
        preview: tab.preview,
      });
    }
  }
  return out;
}

/** Editar uma aba em pré-visualização a fixa (pedido ao core). */
export function shouldPin(preview: boolean, dirty: boolean): boolean {
  return preview && dirty;
}

/** Esc fecha a tela, exceto digitando num campo (ou já tratado por alguém). */
export function escClosesScreen(event: {
  key: string;
  defaultPrevented: boolean;
  target: { isContentEditable?: boolean; tagName?: string } | null;
}): boolean {
  if (event.key !== "Escape" || event.defaultPrevented) return false;
  const t = event.target;
  if (!t) return true;
  if (t.isContentEditable) return false;
  return !["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName ?? "");
}

/**
 * O que ⌘W fecha com uma tela cheia aberta: o pane de terminal fica escondido
 * por baixo, então nunca é ele. Na de Arquivos, a aba interna ativa; sem aba
 * (ou em Git/Docker), a própria tela. Sem tela, o pane como sempre.
 */
export function closeTarget(
  screen: ScreenKind | null,
  activeFile: string | null,
): "file" | "screen" | "pane" {
  if (!screen) return "pane";
  return screen === "files" && activeFile ? "file" : "screen";
}
