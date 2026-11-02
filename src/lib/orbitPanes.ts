/**
 * Regras puras do painel da aba Terminal emprestado ao nó Terminal do Orbit
 * (ver `TerminalPaneHost`). A sessão do nó não está em layout nenhum: o que a
 * grade decide pela sessão ativa, aqui se decide por sessão presa a um nó.
 */

/**
 * Tecla com blocos marcados e o Orbit aberto. Só vale para a marcação de uma
 * sessão presa a um nó: ⌘C copia (perde para seleção de texto de verdade) e
 * Esc limpa (fora de campo de texto — lá o Esc é do campo). O resto dos
 * atalhos do app segue bloqueado por baixo do Orbit.
 */
export function orbitPickKey(input: {
  key: string;
  isCopy: boolean;
  /** A sessão da marcação acesa, se houver. */
  pickSession: string | null;
  attached: (id: string) => boolean;
  inTextField: boolean;
  /** Há texto selecionado (janela ou xterm): o ⌘C é dele. */
  textSelected: boolean;
}): "copy" | "clear" | null {
  if (!input.pickSession || !input.attached(input.pickSession)) return null;
  if (input.isCopy && !input.textSelected) return "copy";
  if (input.key === "Escape" && !input.inTextField) return "clear";
  return null;
}

/**
 * Título da janela: só a sessão da grade acende o "precisa de você". A do nó
 * não está em aba nenhuma, então nada a marcaria vista — acenderia para sempre.
 */
export function gridAttention(
  sessions: readonly { id: string; attention: boolean }[],
  inLayout: (id: string) => boolean,
): boolean {
  return sessions.some((s) => s.attention && inLayout(s.id));
}

/**
 * Sessões de nó que soltaram o painel e já fecharam: saem da lista do `App`
 * (assinaturas, blocos). A que só saiu da tela ou foi para o modo foco segue
 * viva e fica.
 */
export function orbitPanesToPrune(input: {
  detached: ReadonlySet<string>;
  sessions: readonly { id: string; finished: boolean }[];
  attached: (id: string) => boolean;
  inLayout: (id: string) => boolean;
}): string[] {
  return input.sessions
    .filter(
      (s) =>
        input.detached.has(s.id) && s.finished && !input.attached(s.id) && !input.inLayout(s.id),
    )
    .map((s) => s.id);
}

/** Espera entre as tentativas de ler a sessão do nó: limitada, sem laço. */
export const ATTACH_RETRY_MS: readonly number[] = [300, 1000, 3000];

/**
 * `load` até dar certo: a primeira na hora, depois uma por espera de
 * `delays`. Devolve se alguma deu certo.
 */
export async function loadWithRetry(
  load: () => Promise<boolean>,
  delays: readonly number[] = ATTACH_RETRY_MS,
  sleep: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
): Promise<boolean> {
  for (let i = 0; ; i++) {
    if (await load().catch(() => false)) return true;
    if (i >= delays.length) return false;
    await sleep(delays[i]);
  }
}
