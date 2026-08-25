/**
 * Espelho da política de aviso do sistema, que mora em
 * `src-tauri/src/agent/notify.rs`.
 *
 * A duplicação é deliberada e mínima. A tela de Ajustes precisa mostrar o
 * estado que o core vai aplicar **antes** de o usuário ter escolhido qualquer
 * coisa, e a preferência ausente é justamente o caso em que não há o que ler —
 * sem o default de fábrica aqui, o interruptor apareceria ligado para uma
 * espécie que nasce desligada, que é pior do que duplicar três booleanos.
 *
 * O que não pode divergir são as **chaves** e o **default por espécie**. Os
 * testes dos dois lados afirmam os mesmos valores; mudar um sem o outro derruba
 * um gate.
 */

/** Espelha `NotifyKind` do core. */
export const NOTIFY_KINDS = [
  {
    id: "request",
    label: "notifyRequest",
    hint: "notifyRequestHint",
    enabledKey: "pref.notify.request.enabled",
    soundKey: "pref.notify.request.sound",
    defaultSound: "Ping",
    defaultEnabled: true,
  },
  {
    id: "done",
    label: "notifyDone",
    hint: "notifyDoneHint",
    enabledKey: "pref.notify.done.enabled",
    soundKey: "pref.notify.done.sound",
    defaultSound: "Glass",
    defaultEnabled: true,
  },
  // Espécie própria porque o interruptor é próprio: o de cima é o agente
  // falando por um hook, este é o TYBA lendo a tela de um programa que não sabe
  // que está sendo lido. Juntá-los faria desligar o palpite desligar o fato.
  //
  // Nasce **desligada**: quem autoriza um palpite a interromper é o `notifies`
  // do manifesto, que é escrito por nós. Ligada de fábrica, o usuário
  // consentiria uma vez ao conceito e o TYBA passaria a escolher por release
  // quem pode interrompê-lo.
  {
    id: "observedRequest",
    label: "notifyObservedRequest",
    hint: "notifyObservedRequestHint",
    enabledKey: "pref.notify.observed_request.enabled",
    soundKey: "pref.notify.observed_request.sound",
    defaultSound: "Ping",
    defaultEnabled: false,
  },
] as const;

export type NotifyKindSpec = (typeof NOTIFY_KINDS)[number];

/**
 * Espelha o `resolve` do core: valor irreconhecível cai no default **da
 * espécie**, e não em "ligado".
 *
 * Para o pedido do hook isso não muda nada de observável — o default dele já é
 * ligado. Para o palpite muda tudo: um valor corrompido não pode ligar uma
 * interrupção que o usuário nunca habilitou.
 */
export function notifyEnabled(
  raw: string | null | undefined,
  defaultEnabled: boolean,
): boolean {
  if (raw === "on" || raw === "true" || raw === "1") return true;
  if (raw === "off" || raw === "false" || raw === "0") return false;
  return defaultEnabled;
}
