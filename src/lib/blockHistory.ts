import type { Block } from "./ipc";

/**
 * Junta os blocos persistidos com os que já estão na tela.
 *
 * O histórico do SQLite chega assíncrono. Um comando que termina nesse meio
 * tempo já pôs um bloco na lista — e descartar o histórico por causa dele
 * apagaria a sessão inteira da tela, justamente no caso em que o usuário
 * reabriu para ver o que tinha ficado.
 *
 * A junção é por id, e o histórico vai na frente: ele é mais antigo que
 * qualquer coisa que tenha acontecido depois desta janela abrir.
 */
export function mergeBlockHistory(live: Block[], loaded: Block[]): Block[] {
  if (loaded.length === 0) return live;
  if (live.length === 0) return loaded;
  const seen = new Set(live.map((block) => block.id));
  const history = loaded.filter((block) => !seen.has(block.id));
  if (history.length === 0) return live;
  return [...history, ...live];
}
