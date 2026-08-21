import type { StartupMode } from "../components/SettingsView";
import type { Loaded } from "./ipc";

/** Espelha `StartupMode::parse` no core: valor desconhecido ou ausente cai em
 * `resume`, que é o que um terminal faz — reabre onde você parou. */
export function parseStartupMode(raw: string | null | undefined): StartupMode {
  if (raw === "keep_layout") return "keep_layout";
  if (raw === "fresh") return "fresh";
  return "resume";
}

/**
 * Abrir o modal de "nova sessão" no boot?
 *
 * A pergunta é "não há workspace nenhum?", e ela só tem resposta quando o core
 * terminou de carregar.
 *
 * > [!warning] `ready: false` não é layout vazio. A thread de boot responde
 * > `boot_snapshot` antes de o `load_remapped` rodar, e nesse intervalo
 * > `workspaces` vem vazio por ainda não ter sido lido — não por não haver
 * > nada. Decidir por esse valor abre o modal por cima dos workspaces que estão
 * > voltando; e como a decisão vale uma vez só por janela, ela nunca se
 * > reavaliaria quando o layout real chegasse.
 */
export function shouldPromptNewSession(input: {
  ready: boolean;
  workspaces: number;
  prompted: boolean;
}): boolean {
  if (!input.ready || input.prompted) return false;
  return input.workspaces === 0;
}

/** O que uma resposta do core que carrega `ready` muda no estado do boot. */
export interface BootUpdate<T> {
  /** `ready` depois desta resposta. */
  ready: boolean;
  /** O dado a aplicar, ou `null` quando a resposta não vale como estado. */
  value: T | null;
}

/**
 * Funde uma resposta do core com o que a janela já sabe sobre o boot.
 *
 * Duas invariantes, e elas moram aqui porque são três os pontos que recebem
 * resposta com `ready` — `boot_snapshot`, `list_sessions` e `layout_state` —, e
 * lembrar da mesma regra em três lugares já falhou uma vez:
 *
 * 1. **`ready` não regride.** Uma vez verdadeiro, permanece.
 * 2. **O dado só se aplica quando a resposta vem `ready`.** Vazio de boot não é
 *    vazio: a lista veio assim por ainda não ter sido lida.
 *
 * > [!warning] A resposta pode chegar velha, e velha ela diz `false`. Os
 * > listeners são registrados antes de `boot_snapshot()` resolver, e o core
 * > marca ready → emite `layout://changed` → emite `app://ready` enquanto a
 * > resposta do snapshot ainda está sendo montada — ela lê `ready` ANTES do
 * > valor, de propósito, para que o pior caso seja anunciar `false` com dado
 * > que já era bom. Se o evento vencer a corrida, rebaixar o `ready` por causa
 * > da resposta atrasada apagava sessões e layout, e `app://ready` não dispara
 * > de novo: a janela ficava vazia — com o splash já fora, que desiste aos 4 s
 * > — até alguma chamada não relacionada acontecer. A parte do front no
 * > contrato é reconsultar em vez de regredir.
 */
export function mergeLoaded<T>(
  known: boolean,
  response: Loaded<T> | null | undefined,
): BootUpdate<T> {
  if (!response?.ready) return { ready: known, value: null };
  return { ready: true, value: response.value };
}

/**
 * O app terminou de carregar e o splash pode sair.
 *
 * Evento de DOM, e não de IPC: quem escuta é o `main.tsx`, que roda antes do
 * React e não fala com o core.
 */
export const SPLASH_DONE_EVENT = "tyba:ready";

/**
 * Teto de espera do splash, em ms.
 *
 * O splash sai quando o app está pronto — mas "pronto" depende do core, e o
 * core pode ficar parado num diálogo de permissão do macOS, que segura a
 * thread até alguém clicar. Sem teto, um boot que trava deixa o usuário preso
 * olhando um logo, que é pior do que a UI vazia que ele substituiu.
 */
export const SPLASH_CEILING_MS = 4000;
