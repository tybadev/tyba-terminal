/**
 * Navegador nativo do nó (Fase 5, `fase5-navegador`): a view nativa fica por
 * cima de todo o HTML, então o front só decide **quando** ela pode aparecer
 * (oclusão) e manda a intenção de geometria ao core. Estado de navegador
 * (url, título, foco, print) é do core: chega pelo evento `orbit-browser`.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface BrowserRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type BrowserFocus = "none" | "owner" | "agent";

/** O que um agente fez por último na página (`orbit::browser::AgentStep`). */
export interface AgentStep {
  by: string;
  /** `snapshot`, `click`, `type`, `console` ou `screenshot`. */
  action: string;
  /** O nome do elemento (clicar, digitar). */
  target: string | null;
  at: number;
}

/** Espelha `orbit::browser::BrowserState`. */
export interface BrowserState {
  url: string;
  title: string;
  loading: boolean;
  focus: BrowserFocus;
  live: boolean;
  previewAt: number | null;
  refused: string | null;
  ephemeral: boolean;
  agent: AgentStep | null;
  consoleErrors: number | null;
}

/** Quanto tempo depois do último passo o card segue em "<agente> usando". */
export const AGENT_ACTIVE_MS = 8000;

/**
 * Quanto falta do estado ativo "<agente> usando" (0: volta a "controlado
 * por"). Vale enquanto o core segura o foco "agente" ou até
 * {@link AGENT_ACTIVE_MS} depois do último passo — entre um passo e outro o
 * card não pisca.
 */
export function agentActiveLeft(state: BrowserState | null, now: number): number {
  if (!state?.agent) return 0;
  const left = Math.max(0, state.agent.at + AGENT_ACTIVE_MS - now);
  return state.focus === "agent" ? Math.max(left, 1) : left;
}

/** As ações que o rodapé conhece; outra cai em `snapshot`. */
const STEP_ACTIONS = new Set(["snapshot", "click", "type", "console", "screenshot"]);

/** A chave i18n do passo do agente ("clicou em “X”"). */
export function agentStepKey(step: AgentStep): string {
  const action = STEP_ACTIONS.has(step.action) ? step.action : "snapshot";
  const bare = (action === "click" || action === "type") && !step.target;
  return `orbitBrowserStep_${action}${bare ? "Bare" : ""}`;
}

/** Faixa de zoom do canvas em que a view fica viva (fora: só o print). */
export const NATIVE_ZOOM_MIN = 0.5;
export const NATIVE_ZOOM_MAX = 2;
/** Canvas parado por este tempo antes de a view voltar (debounce). */
export const SETTLE_MS = 150;

/**
 * O que, desenhado por cima do marcador, esconde a view: menus e popovers
 * (portais do Radix), diálogos, listas e o que se marcar com `data-occludes`
 * (paleta, foco do nó, seletor de cor).
 */
export const OCCLUDER_SELECTOR =
  "[data-radix-popper-content-wrapper],[role=dialog],[role=menu],[role=listbox],[data-occludes]";

export function rectInside(inner: BrowserRect, outer: BrowserRect): boolean {
  return (
    inner.x >= outer.x - 0.5 &&
    inner.y >= outer.y - 0.5 &&
    inner.x + inner.w <= outer.x + outer.w + 0.5 &&
    inner.y + inner.h <= outer.y + outer.h + 0.5
  );
}

export function rectsIntersect(a: BrowserRect, b: BrowserRect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** Mesmo retângulo, a meio px. */
export function sameRect(a: BrowserRect | null, b: BrowserRect | null): boolean {
  if (!a || !b) return a === b;
  return (
    Math.abs(a.x - b.x) < 0.5 &&
    Math.abs(a.y - b.y) < 0.5 &&
    Math.abs(a.w - b.w) < 0.5 &&
    Math.abs(a.h - b.h) < 0.5
  );
}

/**
 * Pontos de amostra sobre o marcador: grade 5×5 e os quatro cantos, 2 px para
 * dentro (a borda arredondada não conta).
 */
export function samplePoints(r: BrowserRect, n = 5): Array<[number, number]> {
  const inset = 2;
  const x0 = r.x + inset;
  const y0 = r.y + inset;
  const w = Math.max(0, r.w - 2 * inset);
  const h = Math.max(0, r.h - 2 * inset);
  const pts: Array<[number, number]> = [];
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      pts.push([x0 + (w * (i + 0.5)) / n, y0 + (h * (j + 0.5)) / n]);
    }
  }
  pts.push([x0, y0], [x0 + w, y0], [x0, y0 + h], [x0 + w, y0 + h]);
  return pts;
}

export interface OcclusionInput {
  /** O marcador, em px CSS da janela. */
  marker: BrowserRect;
  /** A área útil do canvas. */
  area: BrowserRect;
  /** Zoom do canvas. */
  zoom: number;
  /** Pan/zoom nos últimos 150 ms, ou arraste de nó, ligação ou seleção. */
  moving: boolean;
  /** Retângulos de menus, diálogos e afins abertos. */
  occluders: BrowserRect[];
  /** Algum ponto de amostra caiu em outro elemento (algo por cima). */
  covered: boolean;
  /** O dono está usando a página. */
  owner: boolean;
}

/**
 * A view pode ficar viva? Só com o marcador inteiro na área útil, zoom na
 * faixa, canvas parado e nada do DOM por cima. Com o dono usando, só algo
 * por cima a derruba (o canvas não recebe pan com o teclado na página).
 */
export function occlusionVerdict(i: OcclusionInput): boolean {
  const over = i.covered || i.occluders.some((o) => rectsIntersect(o, i.marker));
  if (i.marker.w < 1 || i.marker.h < 1) return false;
  if (i.owner) return !over;
  return (
    !over &&
    !i.moving &&
    i.zoom >= NATIVE_ZOOM_MIN &&
    i.zoom <= NATIVE_ZOOM_MAX &&
    rectInside(i.marker, i.area)
  );
}

/** Onde o marcador estava e desde quando (para o debounce do pan/zoom). */
export interface Motion {
  rect: BrowserRect | null;
  changedAt: number;
}

/**
 * O marcador mexeu? Retângulo novo reinicia o relógio; parado há menos de
 * [`SETTLE_MS`] ainda conta como movimento (a view só volta depois).
 */
export function trackMotion(
  prev: Motion,
  rect: BrowserRect,
  now: number,
): { motion: Motion; moving: boolean } {
  const motion = sameRect(prev.rect, rect) ? prev : { rect, changedAt: now };
  return { motion, moving: now - motion.changedAt < SETTLE_MS };
}

/** "HH:MM" do print. */
export function previewTime(at: number): string {
  const d = new Date(at);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// ── core ──────────────────────────────────────────────────────────────────

let native: Promise<boolean> | null = null;
/** O sistema tem a view nativa? (Senão o nó usa o iframe.) */
export const browserNative = (): Promise<boolean> =>
  (native ??= invoke<boolean>("orbit_browser_native").catch(() => false));

export const browserNavigate = (graphId: string, nodeId: string, url: string) =>
  invoke<void>("orbit_browser_navigate", { graphId, nodeId, url });

export const browserView = (
  graphId: string,
  nodeId: string,
  rect: BrowserRect | null,
  zoom: number,
  center: [number, number] | null,
) => invoke<void>("orbit_browser_view", { graphId, nodeId, rect, zoom, center });

export const browserBack = (graphId: string, nodeId: string) =>
  invoke<void>("orbit_browser_back", { graphId, nodeId });

export const browserReload = (graphId: string, nodeId: string) =>
  invoke<void>("orbit_browser_reload", { graphId, nodeId });

export const browserState = (graphId: string, nodeId: string) =>
  invoke<BrowserState | null>("orbit_browser_state", { graphId, nodeId });

/** O último print da página (JPEG em base64), para a prévia do card. */
export const browserPreview = (graphId: string, nodeId: string) =>
  invoke<string | null>("orbit_browser_preview", { graphId, nodeId });

/** Só ficam vivos os navegadores que o canvas mostra (vazio: todos saem). */
export const browserRetain = (graphId: string, nodeIds: string[]) =>
  invoke<void>("orbit_browser_retain", { graphId, nodeIds });

export const onBrowserState = (
  handler: (e: { graph: string; node: string; state: BrowserState }) => void,
): Promise<UnlistenFn> =>
  listen<{ graph: string; node: string; state: BrowserState }>("orbit-browser", (e) =>
    handler(e.payload),
  );
