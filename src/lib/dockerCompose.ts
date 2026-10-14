// Seção Compose da tela de Docker (feature telas-cheias): os projetos compose
// do alvo da sessão, montados a partir da mesma lista de containers que a
// seção Containers usa. Só leitura e formatação — agir é do core.

import { exitCode, exitedAgoMs, healthOf, uptimeMs } from "./dockerPanel";
import type { ContainerInfo, PathStatus } from "./ipc";

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
export function composeProjects(list: readonly ContainerInfo[]): ComposeProject[] {
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
    };
  });
  return out.sort((a, b) =>
    a.inRepo !== b.inRepo ? (a.inRepo ? -1 : 1) : a.name.localeCompare(b.name),
  );
}

/** "Ocultar parados": some o projeto sem nada rodando. */
export function visibleProjects(
  projects: readonly ComposeProject[],
  hideStopped: boolean,
): ComposeProject[] {
  return hideStopped ? projects.filter((p) => p.running > 0) : [...projects];
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
