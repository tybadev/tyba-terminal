// Seção Compose da tela de Docker (feature telas-cheias): os projetos compose
// do alvo da sessão, montados a partir da mesma lista de containers que a
// seção Containers usa. Só leitura e formatação — agir é do core.

import { exitCode, exitedAgoMs, healthOf, uptimeMs } from "./dockerPanel";
import type { ComposeOutput, ComposeRun, ContainerInfo, PathStatus } from "./ipc";

/** Mesmo teto do core (`COMPOSE_OUTPUT_CAP`). */
export const COMPOSE_OUTPUT_CAP = 2000;

/**
 * Junta um lote de saída ao que a tela tem, pela execução e pela posição: o
 * que já chegou não repete; execução nova começa do zero (o lote de abertura
 * já basta para mostrar); lote depois de um buraco espera o próximo retrato.
 */
export function mergeComposeOutput(
  runs: Record<string, ComposeRun>,
  batch: ComposeOutput,
): Record<string, ComposeRun> {
  const run = runs[batch.project];
  if (!run || batch.run > run.run) {
    if (batch.start !== 0) return runs;
    return {
      ...runs,
      [batch.project]: {
        run: batch.run,
        working_dir: run?.working_dir ?? null,
        op: batch.op,
        last_op: batch.op,
        error: null,
        output: batch.lines.slice(-COMPOSE_OUTPUT_CAP),
        total: batch.lines.length,
      },
    };
  }
  if (batch.run < run.run || batch.start > run.total) return runs;
  const fresh = batch.lines.slice(run.total - batch.start);
  if (fresh.length === 0) return runs;
  return {
    ...runs,
    [batch.project]: {
      ...run,
      output: [...run.output, ...fresh].slice(-COMPOSE_OUTPUT_CAP),
      total: run.total + fresh.length,
    },
  };
}

/**
 * O retrato periódico do core manda, menos onde a tela já está à frente: na
 * mesma execução, linhas que chegaram por evento depois do retrato ser lido
 * não somem; execução mais nova vista por evento não volta para a antiga.
 */
export function mergeComposeSnapshot(
  current: Record<string, ComposeRun>,
  snapshot: Record<string, ComposeRun>,
): Record<string, ComposeRun> {
  const out: Record<string, ComposeRun> = { ...snapshot };
  for (const [name, mine] of Object.entries(current)) {
    const snap = snapshot[name];
    if (!snap || mine.run > snap.run) {
      if (snap || mine.op) out[name] = mine;
      continue;
    }
    if (mine.run === snap.run && mine.total > snap.total) {
      out[name] = { ...snap, output: mine.output, total: mine.total };
    }
  }
  return out;
}

export type ComposeProject = {
  name: string;
  workingDir: string | null;
  dirStatus: PathStatus | null;
  fileStatus: PathStatus | null;
  /** Nome do primeiro arquivo compose ("compose.yml"), se o label diz. */
  composeFile: string | null;
  /** O projeto do repo da sessão (decidido pelo core, `in_repo`). */
  inRepo: boolean;
  services: ContainerInfo[];
  running: number;
  /** Já rodou compose nesta execução do app (a saída fica na tela). */
  hasRun: boolean;
};

/** Todos rodando, alguns, ou nenhum: a cor do ponto do projeto. */
export type ProjectRun = "all" | "some" | "none";

export function projectRun(p: Pick<ComposeProject, "running" | "services">): ProjectRun {
  if (p.running === 0) return "none";
  return p.running === p.services.length ? "all" : "some";
}

function firstFile(raw: string | null): string | null {
  const first = raw?.split(",")[0]?.trim();
  if (!first) return null;
  return first.split("/").pop() || null;
}

/**
 * Agrupa os containers por projeto compose (avulsos ficam de fora: moram em
 * Containers). Ordem: o projeto do repo primeiro, depois por nome. Serviços
 * por nome.
 */
export function composeProjects(
  list: readonly ContainerInfo[],
  runs: Readonly<Record<string, ComposeRun>> = {},
): ComposeProject[] {
  const byName = new Map<string, ContainerInfo[]>();
  for (const c of list) {
    if (!c.compose_project) continue;
    byName.set(c.compose_project, [...(byName.get(c.compose_project) ?? []), c]);
  }
  const out = [...byName.entries()].map(([name, items]): ComposeProject => {
    const anchor = items.find((c) => c.compose_working_dir) ?? items[0];
    const services = [...items].sort((a, b) =>
      (a.service ?? a.name).localeCompare(b.service ?? b.name),
    );
    return {
      name,
      workingDir: anchor.compose_working_dir,
      dirStatus: anchor.working_dir_status,
      fileStatus: anchor.compose_file_status,
      composeFile: firstFile(anchor.config_files),
      inRepo: items.some((c) => c.in_repo),
      services,
      running: items.filter((c) => c.state === "running").length,
      hasRun: name in runs,
    };
  });
  // Projeto que rodou compose e ficou sem container (depois do `down`):
  // continua na coluna, com a saída e as ações.
  for (const [name, run] of Object.entries(runs)) {
    if (byName.has(name)) continue;
    out.push({
      name,
      workingDir: run.working_dir,
      dirStatus: null,
      fileStatus: null,
      composeFile: null,
      inRepo: false,
      services: [],
      running: 0,
      hasRun: true,
    });
  }
  return out.sort((a, b) =>
    a.inRepo !== b.inRepo ? (a.inRepo ? -1 : 1) : a.name.localeCompare(b.name),
  );
}

/** "Ocultar parados": some o projeto sem nada rodando (menos o que tem saída de compose). */
export function visibleProjects(
  projects: readonly ComposeProject[],
  hideStopped: boolean,
): ComposeProject[] {
  return hideStopped ? projects.filter((p) => p.running > 0 || p.hasRun) : [...projects];
}

/**
 * O projeto que a seção mostra: o escolhido (se ainda está na lista), senão o
 * do repo, senão o primeiro. `null` sem projeto.
 */
export function pickProject(
  projects: readonly ComposeProject[],
  selected: string | null,
): string | null {
  if (selected && projects.some((p) => p.name === selected)) return selected;
  return (projects.find((p) => p.inRepo) ?? projects[0])?.name ?? null;
}

/** Portas publicadas como no mockup: `:3000 → 3000`, sem repetir IPv4/IPv6. */
export function portMappings(ports: string): string[] {
  const seen: string[] = [];
  for (const part of ports.split(",")) {
    const match = /:(\d+)->(\d+)/.exec(part.trim());
    if (!match) continue;
    const label = `:${match[1]} → ${match[2]}`;
    if (!seen.includes(label)) seen.push(label);
  }
  return seen;
}

export type ServiceState =
  | { tone: "green" | "amber" | "red"; key: "running" | "healthy" | "unhealthy" | "starting"; span: number | null }
  | { tone: "red" | "faint"; key: "exited"; code: number | null; span: number | null };

/** O texto da coluna Estado: rodando/saudável há X, saiu com N há X. */
export function serviceState(c: Pick<ContainerInfo, "state" | "status">): ServiceState {
  if (c.state === "running") {
    const health = healthOf(c.status);
    const span = uptimeMs(c.status);
    if (health === "unhealthy") return { tone: "red", key: "unhealthy", span };
    if (health === "starting") return { tone: "amber", key: "starting", span };
    return { tone: "green", key: health === "healthy" ? "healthy" : "running", span };
  }
  const code = exitCode(c.status);
  return {
    tone: code !== null && code !== 0 ? "red" : "faint",
    key: "exited",
    code,
    span: exitedAgoMs(c.status),
  };
}
