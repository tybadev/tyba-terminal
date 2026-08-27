import { describe, expect, it } from "bun:test";

import {
  filete,
  rodaParaHorizontal,
  temAntes,
  temDepois,
  trazerParaVista,
} from "./tabScroll";

/** Faixa de 800px visíveis sobre 2000px de abas, rolada até `scrollLeft`. */
const faixa = (scrollLeft: number, scrollWidth = 2000, clientWidth = 800) => ({
  scrollLeft,
  clientWidth,
  scrollWidth,
});

describe("bordas da faixa", () => {
  it("no começo só há conteúdo depois", () => {
    expect(temAntes(faixa(0))).toBe(false);
    expect(temDepois(faixa(0))).toBe(true);
  });

  it("no fim só há conteúdo antes", () => {
    expect(temAntes(faixa(1200))).toBe(true);
    expect(temDepois(faixa(1200))).toBe(false);
  });

  it("no meio há dos dois lados", () => {
    expect(temAntes(faixa(600))).toBe(true);
    expect(temDepois(faixa(600))).toBe(true);
  });

  it("faixa que cabe inteira não tem borda nenhuma", () => {
    // É o caso comum — duas ou três abas. Fade aqui apagaria conteúdo sem ter
    // o que esconder.
    const cabe = faixa(0, 500, 800);
    expect(temAntes(cabe)).toBe(false);
    expect(temDepois(cabe)).toBe(false);
  });

  it("meio pixel de sobra não acende o fade", () => {
    // Arredondamento de layout: 800,4px de faixa com 800px de conteúdo. Sem a
    // folga o fade pisca enquanto a janela é redimensionada.
    expect(temDepois(faixa(0, 801, 800))).toBe(false);
  });
});

describe("filete", () => {
  it("proporção é a parte visível sobre o total", () => {
    // 800 de 2000 = 40%.
    expect(filete(faixa(0)).largura).toBeCloseTo(40, 5);
  });

  it("começa colado na esquerda e termina colado na direita", () => {
    expect(filete(faixa(0)).esquerda).toBeCloseTo(0, 5);
    const fim = filete(faixa(1200));
    expect(fim.esquerda + fim.largura).toBeCloseTo(100, 5);
  });

  it("nunca some quando há aba demais", () => {
    // 13 abas dão ~8%, que numa faixa estreita é um ponto de 6px. O piso
    // custa precisão e compra legibilidade.
    expect(filete(faixa(0, 10000)).largura).toBeGreaterThanOrEqual(12);
  });

  it("com o piso ativo, ainda assim não passa da borda", () => {
    // A armadilha: deslocar sobre a largura TODA faria o filete começar em 92%
    // com 12% de comprimento, vazando 4% para fora.
    const fim = filete(faixa(9200, 10000));
    expect(fim.esquerda + fim.largura).toBeLessThanOrEqual(100.0001);
  });

  it("sem nada a rolar ocupa a faixa inteira", () => {
    // Encolher para o piso aqui seria mentir que há mais conteúdo.
    expect(filete(faixa(0, 500, 800))).toEqual({ esquerda: 0, largura: 100 });
  });
});

describe("trazer para a vista", () => {
  it("não mexe no que já está visível", () => {
    // Rolar para onde já se está cancela o gesto em curso, e a faixa dá um
    // tranco a cada troca de aba.
    expect(trazerParaVista(faixa(0), { esquerda: 300, largura: 100 })).toBeNull();
  });

  it("volta para trás quando o alvo ficou atrás", () => {
    const destino = trazerParaVista(faixa(600), { esquerda: 100, largura: 100 });
    expect(destino).toBe(76);
  });

  it("avança quando o alvo ficou adiante", () => {
    const destino = trazerParaVista(faixa(0), { esquerda: 900, largura: 100 });
    expect(destino).toBe(224);
  });

  it("deixa margem para o alvo não nascer debaixo do fade", () => {
    // Encostado na borda, a aba recém-escolhida apareceria meio apagada.
    const destino = trazerParaVista(faixa(600), { esquerda: 600, largura: 100 });
    expect(destino).toBe(576);
  });

  it("não rola além do fim do conteúdo", () => {
    const destino = trazerParaVista(faixa(0), { esquerda: 1900, largura: 100 });
    expect(destino).toBeLessThanOrEqual(1200);
  });
});

describe("roda do mouse", () => {
  it("mouse comum move a faixa na horizontal", () => {
    // Só tem deltaY. Sem isto, girar a roda não faz nada — pior que não ter.
    expect(rodaParaHorizontal({ deltaX: 0, deltaY: 120 })).toBe(120);
  });

  it("trackpad horizontal é deixado em paz", () => {
    // O navegador já rola sozinho; somar aqui rolaria em dobro.
    expect(rodaParaHorizontal({ deltaX: 90, deltaY: 4 })).toBe(0);
  });

  it("gesto diagonal segue o eixo dominante", () => {
    expect(rodaParaHorizontal({ deltaX: 10, deltaY: 80 })).toBe(80);
  });
});
