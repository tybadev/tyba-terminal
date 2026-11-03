// Lógica da tela de sessão vazia (`EmptySessions`), fora do componente para
// poder testar sem DOM.

import type { Host } from "./ipc";
import type { Bindings } from "./keys";

/** Quantos hosts a seção "Recentes" mostra no máximo. */
export const RECENT_HOSTS_MAX = 4;

/**
 * Os hosts SSH usados por último, do mais recente para o mais antigo. Host que
 * nunca conectou fica fora: "Recentes" é o que a pessoa já usou, não a lista
 * de Conexões SSH inteira.
 */
export function recentHosts(hosts: Host[], max = RECENT_HOSTS_MAX): Host[] {
  return hosts
    .filter((h) => h.last_connected_at !== null)
    .sort((a, b) =>
      (b.last_connected_at ?? "").localeCompare(a.last_connected_at ?? ""),
    )
    .slice(0, max);
}

/**
 * A ação principal e o atalho dela. Sem workspace, é "Nova sessão" com o
 * atalho de nova sessão (⌘N no mac); com workspace e sem aba, é "Nova tab"
 * com o de nova aba (⌘T). Sempre o binding do usuário, nunca tecla fixa.
 */
export function primaryAction(
  hasWorkspaces: boolean,
  bindings: Pick<Bindings, "newSession" | "newTab">,
): { kind: "newSession" | "newTab"; combo: string } {
  return hasWorkspaces
    ? { kind: "newTab", combo: bindings.newTab }
    : { kind: "newSession", combo: bindings.newSession };
}
