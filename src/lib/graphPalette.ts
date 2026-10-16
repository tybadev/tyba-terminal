// A paleta que cria nós no canvas (no lugar do "+ Nó"; quadro
// OrbitCanvasLiso): duplo clique, ⌘K ou "/" abrem uma busca ali, e soltar
// uma ligação no vazio abre a mesma busca filtrada pelos que aceitam o que a
// origem produz. Puro: as entradas e o filtro; quem desenha é o componente.

import type { Snippet } from "./ipc";
import { connectionKinds, shortDir, type Integration, type NodeKind } from "./orbit";

export type PaletteIcon =
  | "claude"
  | "codex"
  | "process"
  | "service"
  | "browser"
  | "note"
  | "integration"
  | "snippet"
  | "folder";

export type PaletteGroup = "type" | "integration" | "snippet" | "folder";

export interface PaletteEntry {
  id: string;
  group: PaletteGroup;
  icon: PaletteIcon;
  /** O que a linha diz. */
  label: string;
  /** Texto apagado à direita (o comando do snippet, o que a pasta abre). */
  detail?: string;
  /** Título do nó que nasce. */
  title: string;
  /** O nó que nasce (sem id nem posição). */
  kind: NodeKind;
}

/** Os rótulos dos tipos, já traduzidos. */
export interface PaletteLabels {
  claude: string;
  codex: string;
  process: string;
  service: string;
  browser: string;
  note: string;
}

export interface PaletteInput {
  labels: PaletteLabels;
  /** Pasta onde nasce o nó que tem pasta. */
  dir: string;
  integrations: readonly Integration[];
  snippets: readonly Pick<Snippet, "id" | "name" | "command">[];
  /** Pastas recentes (a mais nova primeiro); cada uma abre um Claude nela. */
  dirs: readonly string[];
}

const agent = (runner: "claude" | "codex", dir: string): NodeKind => ({
  type: "agent",
  runner,
  role: "",
  prompt: "",
  dir,
});

const process = (command: string, dir: string): NodeKind => ({
  type: "process",
  command,
  output: "log",
  file: null,
  dir,
});

/**
 * Tudo que a paleta oferece, na ordem do quadro: os tipos (Agente Claude,
 * Agente Codex, Processo, Serviço, Navegador, Nota), as integrações salvas,
 * os snippets (viram Processo com o comando) e as pastas recentes.
 */
export function paletteEntries({
  labels,
  dir,
  integrations,
  snippets,
  dirs,
}: PaletteInput): PaletteEntry[] {
  const types: PaletteEntry[] = (
    [
      {
        id: "type:claude",
        icon: "claude",
        label: labels.claude,
        title: "Claude",
        kind: agent("claude", dir),
      },
      {
        id: "type:codex",
        icon: "codex",
        label: labels.codex,
        title: "Codex",
        kind: agent("codex", dir),
      },
      {
        id: "type:process",
        icon: "process",
        label: labels.process,
        title: labels.process,
        kind: process("", dir),
      },
      {
        id: "type:service",
        icon: "service",
        label: labels.service,
        title: labels.service,
        kind: { type: "service", source: { type: "container", containerId: "", name: "" } },
      },
      {
        id: "type:browser",
        icon: "browser",
        label: labels.browser,
        title: labels.browser,
        kind: { type: "browser", url: "" },
      },
      {
        id: "type:note",
        icon: "note",
        label: labels.note,
        title: labels.note,
        kind: { type: "note", markdown: "" },
      },
    ] satisfies Omit<PaletteEntry, "group">[]
  ).map((e) => ({ ...e, group: "type" as const }));
  return [
    ...types,
    ...integrations.map((i) => ({
      id: `integration:${i.id}`,
      group: "integration" as const,
      icon: "integration" as const,
      label: i.name,
      title: i.name,
      kind: { type: "integration" as const, integrationId: i.id, dir },
    })),
    ...snippets.map((s) => ({
      id: `snippet:${s.id}`,
      group: "snippet" as const,
      icon: "snippet" as const,
      label: s.name,
      detail: s.command,
      title: s.name,
      kind: process(s.command, dir),
    })),
    ...[...new Set(dirs.filter((d) => d.trim() !== ""))].map((d) => ({
      id: `folder:${d}`,
      group: "folder" as const,
      icon: "folder" as const,
      label: shortDir(d),
      detail: labels.claude,
      title: "Claude",
      kind: agent("claude", d),
    })),
  ];
}

/** Minúsculas e sem acento: "integracao" acha "Integração". */
const fold = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

/** 0 = começa com a busca, 1 = uma palavra começa, 2 = contém; `null` = não acha. */
function rank(entry: PaletteEntry, query: string): number | null {
  const label = fold(entry.label);
  if (label.startsWith(query)) return 0;
  if (label.split(/[\s/·._-]+/).some((w) => w.startsWith(query))) return 1;
  if (label.includes(query) || fold(entry.detail ?? "").includes(query)) return 2;
  return null;
}

/** De onde a ligação solta no vazio veio: só entram os nós que a aceitam. */
export interface PaletteLink {
  source: NodeKind;
  integrations: ReadonlyMap<string, Integration>;
}

/**
 * O que a paleta mostra para a busca. Sem busca: só os tipos (o resto
 * aparece ao digitar). Com busca: o que acha em todos os grupos, o mais
 * parecido primeiro. Com ligação: só quem aceita o que a origem produz, e os
 * tipos que aceitam continuam embaixo mesmo sem bater com a busca (quadro:
 * "nav" mostra Navegador e, abaixo, Claude, Codex e Processo).
 */
export function filterPalette(
  entries: readonly PaletteEntry[],
  query: string,
  link: PaletteLink | null = null,
): PaletteEntry[] {
  const fits = (e: PaletteEntry) =>
    !link || connectionKinds(link.source, e.kind, link.integrations).length > 0;
  const pool = entries.filter(fits);
  const q = fold(query.trim());
  if (q === "") return pool.filter((e) => e.group === "type");
  const hits = pool
    .map((e, i) => ({ e, i, r: rank(e, q) }))
    .filter((x): x is { e: PaletteEntry; i: number; r: number } => x.r !== null)
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((x) => x.e);
  if (!link) return hits;
  const shown = new Set(hits.map((e) => e.id));
  return [...hits, ...pool.filter((e) => e.group === "type" && !shown.has(e.id))];
}

/** As pastas recentes: a última usada e as dos nós do grafo, sem repetir. */
export function recentDirs(last: string | null, kinds: readonly NodeKind[]): string[] {
  const out: string[] = [];
  const add = (d: string | null | undefined) => {
    if (d && d.trim() !== "" && !out.includes(d)) out.push(d);
  };
  add(last);
  for (const k of kinds) if ("dir" in k) add(k.dir);
  return out.slice(0, 8);
}
