import { describe, expect, test } from "bun:test";

import { NOTIFY_KINDS, notifyEnabled } from "./notifyPrefs";

const byId = (id: string) => {
  const kind = NOTIFY_KINDS.find((k) => k.id === id);
  if (!kind) throw new Error(`espécie ${id} sumiu`);
  return kind;
};

describe("chaves de preferência", () => {
  // Estas strings são as do `NotifyKind::enabled_key`/`sound_key` no core,
  // escritas à mão de propósito: se alguém renomear de um lado só, é aqui que
  // aparece. Mudá-las também faz a preferência já gravada deixar de ser
  // encontrada, e o usuário volta ao default sem ter mexido em nada.
  test("são as mesmas do core", () => {
    expect(byId("request").enabledKey).toBe("pref.notify.request.enabled");
    expect(byId("request").soundKey).toBe("pref.notify.request.sound");
    expect(byId("done").enabledKey).toBe("pref.notify.done.enabled");
    expect(byId("done").soundKey).toBe("pref.notify.done.sound");
    expect(byId("observedRequest").enabledKey).toBe(
      "pref.notify.observed_request.enabled",
    );
    expect(byId("observedRequest").soundKey).toBe(
      "pref.notify.observed_request.sound",
    );
  });

  test("nenhuma espécie divide chave com outra", () => {
    const chaves = NOTIFY_KINDS.flatMap((k) => [k.enabledKey, k.soundKey]);
    expect(new Set(chaves).size).toBe(chaves.length);
  });
});

describe("default por espécie", () => {
  test("o que o hook declara nasce ligado", () => {
    expect(byId("request").defaultEnabled).toBe(true);
    expect(byId("done").defaultEnabled).toBe(true);
  });

  // Quem autoriza um palpite a interromper é o `notifies` do manifesto, que é
  // escrito por nós. Ligado de fábrica, o TYBA escolheria por release quem tem
  // licença de interromper a máquina do usuário.
  test("o palpite da tela nasce desligado", () => {
    expect(byId("observedRequest").defaultEnabled).toBe(false);
  });
});

describe("notifyEnabled", () => {
  test("preferência ausente cai no default da espécie", () => {
    expect(notifyEnabled(null, true)).toBe(true);
    expect(notifyEnabled(undefined, false)).toBe(false);
  });

  test("a escolha explícita ganha do default, nas duas direções", () => {
    expect(notifyEnabled("off", true)).toBe(false);
    expect(notifyEnabled("on", false)).toBe(true);
  });

  // O caso que discrimina o fail-open antigo: com "irreconhecível = ligado", a
  // metade de cima passa e a de baixo não — lixo no banco ligaria uma
  // interrupção que o usuário nunca habilitou.
  test("valor irreconhecível cai no default da espécie, e não em ligado", () => {
    expect(notifyEnabled("talvez", true)).toBe(true);
    expect(notifyEnabled("talvez", false)).toBe(false);
  });
});
