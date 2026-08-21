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
 * O boot normal termina em ~80 ms. Com tick de 150 ms, o evento perdido custa
 * no máximo um piscar até a janela preencher — e no caminho feliz nenhum tick
 * chega a disparar, porque o `app://ready` desmonta o efeito antes disso.
 */
const BOOT_POLL_FAST_MS = 150;
const BOOT_POLL_FAST_UNTIL_MS = 2_000;
/** Passados 2 s o boot não é normal: é disco lento ou diálogo na frente. */
const BOOT_POLL_SLOW_MS = 1_000;
const BOOT_POLL_SLOW_UNTIL_MS = 30_000;
/** Meio minuto parado é gente: alguém tem um diálogo do macOS para clicar. */
const BOOT_POLL_IDLE_MS = 5_000;

/**
 * Quanto esperar até perguntar de novo se o core terminou de carregar — ou
 * `null` para parar.
 *
 * > [!warning] O `app://ready` pode nunca chegar. `listen()` do Tauri é
 * > assíncrono: entre pedir o registro e o listener existir de fato há uma
 * > janela, e o evento emitido dentro dela se perde — o core não reenvia. Se
 * > naquele mesmo intervalo o `boot_snapshot` tiver lido `ready: false` (ele lê
 * > o `ready` antes de montar o resto, de propósito), o front fica sem nenhuma
 * > notícia de que o boot acabou: sessões e layout nunca chegam, o splash já
 * > desistiu aos 4 s e a janela fica vazia até uma chamada não relacionada
 * > acontecer. O poll é a saída que não depende da entrega de um evento.
 *
 * Os intervalos afrouxam porque a espera muda de natureza: o boot normal
 * termina em ~80 ms, mas o caso ruim é um diálogo de permissão do macOS
 * segurando a thread até alguém clicar, o que leva minutos. Perguntar a cada
 * 150 ms por minutos seria desperdício; perguntar a cada 5 s no primeiro
 * segundo seria lentidão visível.
 *
 * Não há teto: desistir devolveria exatamente o estado que este poll existe
 * para fechar — janela vazia, e agora sem nada que a conserte. O laço para no
 * instante em que a resposta vem `ready`, que no caminho feliz é antes mesmo
 * do primeiro tick, e o custo de continuar é uma chamada assíncrona a cada 5 s,
 * fora da main thread.
 */
export function nextBootPoll(input: {
  ready: boolean;
  elapsedMs: number;
}): number | null {
  if (input.ready) return null;
  if (input.elapsedMs < BOOT_POLL_FAST_UNTIL_MS) return BOOT_POLL_FAST_MS;
  if (input.elapsedMs < BOOT_POLL_SLOW_UNTIL_MS) return BOOT_POLL_SLOW_MS;
  return BOOT_POLL_IDLE_MS;
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
