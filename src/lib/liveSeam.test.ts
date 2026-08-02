import { describe, expect, it } from "bun:test";

import {
  blocksRect,
  hiddenFraction,
  liveHeight,
  liveRect,
  LIVE_FRACTION,
  padSlackPx,
  termRect,
  usedFraction,
  type SeamRect,
} from "./liveSeam";

const pane: SeamRect = { left: 0, top: 0, width: 100, height: 100 };

describe("usedFraction", () => {
  it("mede a saída contra a altura da tela", () => {
    expect(usedFraction(12, 24)).toBeCloseTo(0.5);
    expect(usedFraction(6, 24)).toBeCloseTo(0.25);
  });

  it("satura em 1 quando a saída rolou a tela", () => {
    expect(usedFraction(3, 24, true)).toBe(1);
  });

  it("nunca some de todo — comando sem saída ainda mostra o cursor", () => {
    expect(usedFraction(0, 24)).toBeGreaterThan(0);
    expect(usedFraction(0, 24)).toBeLessThan(0.2);
  });

  it("não passa de 1 nem com o cursor além da última linha", () => {
    expect(usedFraction(30, 24)).toBe(1);
  });

  it("degrada para a faixa cheia com números impossíveis", () => {
    expect(usedFraction(5, 0)).toBe(1);
    expect(usedFraction(Number.NaN, 24)).toBe(1);
    expect(usedFraction(5, Number.POSITIVE_INFINITY)).toBe(1);
  });
});

describe("padSlackPx", () => {
  it("é máximo com a faixa quase fechada e some com ela cheia", () => {
    expect(padSlackPx(20, 1)).toBe(0);
    expect(padSlackPx(20, 0)).toBeCloseTo(20);
    expect(padSlackPx(20, 0.5)).toBeCloseTo(10);
  });

  it("sem padding não há folga a compensar", () => {
    expect(padSlackPx(0, 0.3)).toBe(0);
    expect(padSlackPx(Number.NaN, 0.3)).toBe(0);
  });

  it("cobre a última linha do prompt que pergunta sem quebrar linha", () => {
    // O caso real: `Ok to proceed? (y)` com o cursor NA linha do texto. Sem a
    // compensação, o recorte cai acima do fim dela.
    const pane: SeamRect = { left: 0, top: 0, width: 100, height: 100 };
    const used = usedFraction(4, 24);
    expect(liveHeight(pane, used) > 0).toBe(true);
    expect(padSlackPx(20, used)).toBeGreaterThan(15);
  });
});

describe("hiddenFraction", () => {
  it("é o complemento do que aparece", () => {
    expect(hiddenFraction(0.25)).toBeCloseTo(0.75);
    expect(hiddenFraction(1)).toBe(0);
  });

  it("fica nos limites mesmo com entrada fora deles", () => {
    expect(hiddenFraction(1.5)).toBe(0);
    expect(hiddenFraction(-1)).toBe(1);
  });
});

describe("termRect", () => {
  it("é sempre a mesma caixa, e é ela que o PTY enxerga", () => {
    // A regra que não pode cair: nenhum argumento de comando entra aqui.
    expect(termRect(pane)).toEqual({
      left: 0,
      top: 50,
      width: 100,
      height: 50,
    });
  });

  it("acompanha o painel, não o comando", () => {
    const half: SeamRect = { left: 10, top: 20, width: 40, height: 60 };
    expect(termRect(half)).toEqual({
      left: 10,
      top: 50,
      width: 40,
      height: 30,
    });
  });
});

describe("liveRect", () => {
  it("cola a saída no fim do painel, com a altura que ela usa", () => {
    const r = liveRect(pane, 0.5);
    expect(r.height).toBeCloseTo(25);
    expect(r.top + r.height).toBeCloseTo(100);
  });

  it("com a faixa cheia coincide com a caixa do terminal", () => {
    expect(liveRect(pane, 1)).toEqual(termRect(pane));
  });
});

describe("blocksRect", () => {
  it("ociosa, a lista cobre o painel inteiro e esconde o terminal", () => {
    expect(blocksRect(pane, false)).toEqual(pane);
  });

  it("cede só a altura que a saída usa", () => {
    // 12 linhas de 24: a faixa fica com um quarto do painel, não com metade.
    const used = usedFraction(12, 24);
    expect(blocksRect(pane, true, used).height).toBeCloseTo(75);
  });

  it("saída curta devolve à lista o espaço que a faixa fixa desperdiçava", () => {
    const curta = blocksRect(pane, true, usedFraction(3, 24)).height;
    const cheia = blocksRect(pane, true, 1).height;
    expect(curta).toBeGreaterThan(cheia);
    expect(cheia).toBeCloseTo(pane.height * (1 - LIVE_FRACTION));
  });

  it("a lista e a faixa se encaixam sem sobra nem sobreposição", () => {
    for (const used of [0.1, 0.25, 0.5, 0.75, 1]) {
      const blocks = blocksRect(pane, true, used);
      const live = liveRect(pane, used);
      expect(blocks.top + blocks.height).toBeCloseTo(live.top);
      expect(live.top + live.height).toBeCloseTo(pane.top + pane.height);
    }
  });

  it("o fim da lista é onde a saída começa — é isso que faz o cartão nascer no lugar", () => {
    const used = usedFraction(12, 24);
    const live = liveRect(pane, used);
    expect(blocksRect(pane, true, used).height).toBeCloseTo(live.top);
    expect(liveHeight(pane, used)).toBeCloseTo(live.height);
  });
});
