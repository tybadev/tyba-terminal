// Lógica da tela inicial sem sessão (`EmptySessions`), fora do componente para
// poder testar sem DOM. O cartão de lançamento só escolhe; quem abre é o App,
// com as intenções que já existem (sessão de shell, de agente, `connect_host`).

import type { Host } from "./ipc";
import { compactPath } from "./workspaceCwd";

/** Quantos itens cada coluna de recentes mostra: a tela cabe em 900px. */
export const RECENT_MAX = 3;

/**
 * Os hosts SSH usados por último, do mais recente para o mais antigo. Host que
 * nunca conectou fica fora: "Recentes" é o que a pessoa já usou, não a lista
 * de Conexões SSH inteira.
 */
export function recentHosts(hosts: Host[], max = RECENT_MAX): Host[] {
  return hosts
    .filter((h) => h.last_connected_at !== null)
    .sort((a, b) =>
      (b.last_connected_at ?? "").localeCompare(a.last_connected_at ?? ""),
    )
    .slice(0, max);
}

/** `user@host:porta`, só com o que o host tem. */
export function hostAddress(h: Host): string {
  const user = h.username ? `${h.username}@` : "";
  const port = h.port ? `:${h.port}` : "";
  return `${user}${h.hostname}${port}`;
}

export type LaunchWhere = "local" | "folder" | "host";
export type LaunchWhat = "shell" | "claude" | "codex";

/** O que o cartão tem escolhido. `folder`/`hostId` sobrevivem à troca do Onde. */
export interface LaunchChoice {
  where: LaunchWhere;
  what: LaunchWhat;
  folder: string | null;
  hostId: string | null;
}

export const INITIAL_CHOICE: LaunchChoice = {
  where: "local",
  what: "shell",
  folder: null,
  hostId: null,
};

/** Agente só abre na máquina: em host SSH a sessão é shell. */
export function agentsAllowed(choice: LaunchChoice): boolean {
  return choice.where !== "host";
}

/** O "Com quê" que vale de fato (com host, sempre shell). */
export function effectiveWhat(choice: LaunchChoice): LaunchWhat {
  return agentsAllowed(choice) ? choice.what : "shell";
}

/** O host escolhido, se ainda existe na lista. */
export function chosenHost(choice: LaunchChoice, hosts: Host[]): Host | null {
  return hosts.find((h) => h.id === choice.hostId) ?? null;
}

/**
 * O resumo do pé do cartão ("claude em ~/projetos/loja"), em partes para o
 * i18n juntar. `place` é `null` quando o Onde ainda não tem alvo.
 */
export function launchSummary(
  choice: LaunchChoice,
  hosts: Host[],
): { what: LaunchWhat; place: string | null } {
  const what = effectiveWhat(choice);
  if (choice.where === "local") return { what, place: "~" };
  if (choice.where === "folder") {
    return { what, place: choice.folder ? compactPath(choice.folder) : null };
  }
  const host = chosenHost(choice, hosts);
  return { what, place: host ? hostAddress(host) : null };
}

/** O que "Abrir sessão" pede ao App; `null` = falta escolher o alvo. */
export type LaunchPlan =
  | { kind: "shell"; cwd: string | null }
  | { kind: "agent"; runner: "claude_code" | "codex"; cwd: string | null }
  | { kind: "ssh"; host: Host };

export function launchPlan(choice: LaunchChoice, hosts: Host[]): LaunchPlan | null {
  if (choice.where === "host") {
    const host = chosenHost(choice, hosts);
    return host ? { kind: "ssh", host } : null;
  }
  let cwd: string | null = null;
  if (choice.where === "folder") {
    if (!choice.folder) return null;
    cwd = choice.folder;
  }
  if (choice.what === "shell") return { kind: "shell", cwd };
  return {
    kind: "agent",
    runner: choice.what === "codex" ? "codex" : "claude_code",
    cwd,
  };
}

/** Clicar numa pasta recente escolhe o Onde (não abre). */
export function pickFolder(choice: LaunchChoice, path: string): LaunchChoice {
  return { ...choice, where: "folder", folder: path };
}

/** Clicar num host recente escolhe o Onde (não conecta). */
export function pickHost(choice: LaunchChoice, hostId: string): LaunchChoice {
  return { ...choice, where: "host", hostId };
}
