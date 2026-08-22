import { describe, expect, it } from "bun:test";

import { bootFailureTitleKey } from "./bootFailure";
import type { BootFailureKind } from "./ipc";

describe("bootFailureTitleKey", () => {
  it("thread de boot morta promete o que de fato se perdeu", () => {
    // Aqui o app está vazio: não há sessão nem layout. É o único caso em que
    // "sessões e layout podem estar faltando" é verdade.
    expect(bootFailureTitleKey("bootThreadDied")).toBe("bootFailed");
  });

  it("banco degradado NÃO diz que o app não carregou", () => {
    // O bug que este arquivo existe para fechar: as duas origens levavam o
    // mesmo título. Com o banco degradado o arranque terminou inteiro — a
    // frase do outro ramo diria ao usuário que ele perdeu o que está vendo.
    expect(bootFailureTitleKey("storeDegraded")).toBe("bootStoreDegraded");
  });

  it("cada origem tem título próprio, e não há título repetido", () => {
    // Sem esta, um retorno constante passaria nas duas de cima se alguém
    // trocasse as chaves por engano — e voltaríamos ao aviso único.
    const kinds: BootFailureKind[] = ["bootThreadDied", "storeDegraded"];
    const titles = kinds.map(bootFailureTitleKey);
    expect(new Set(titles).size).toBe(kinds.length);
  });
});
