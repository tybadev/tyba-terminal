// didChange em lote: o editor produz uma lista de mudanças por tecla, e cada
// uma virava um `lsp_change` (IPC + escrita no stdin do server) na hora. Aqui
// as listas se acumulam e saem juntas depois de `delayMs` sem digitar.
//
// Concatenar é seguro: cada lista vem pronta para aplicação em sequência
// (`changeExtension` inverte a ordem dentro da transação) e o protocolo aplica
// `contentChanges` na ordem. Quem pergunta algo ao server sobre a posição
// atual (completar, hover, símbolo, salvar) chama `flush()` antes — o pedido
// sai depois do texto, e o server responde sobre o documento certo.

import type { LspContentChange } from "./ipc";

export interface ChangeBatcher {
  push: (changes: LspContentChange[]) => void;
  /** Envia o que está pendente agora (no-op se nada pendente). */
  flush: () => void;
  /** Descarta o pendente — o documento foi reaberto com texto inteiro. */
  clear: () => void;
}

interface Timers {
  setTimeout: (cb: () => void, ms: number) => number;
  clearTimeout: (handle: number) => void;
}

export function createChangeBatcher(
  send: (changes: LspContentChange[]) => void,
  delayMs: number,
  timers: Timers = window,
): ChangeBatcher {
  let pending: LspContentChange[] = [];
  let timer: number | null = null;

  const cancel = () => {
    if (timer !== null) timers.clearTimeout(timer);
    timer = null;
  };
  const flush = () => {
    cancel();
    if (pending.length === 0) return;
    const batch = pending;
    pending = [];
    send(batch);
  };

  return {
    push(changes) {
      if (changes.length === 0) return;
      pending.push(...changes);
      cancel();
      timer = timers.setTimeout(flush, delayMs);
    },
    flush,
    clear() {
      cancel();
      pending = [];
    },
  };
}
