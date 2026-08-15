import { describe, expect, it } from "bun:test";

import { bodyLines, columnsFor, visualLines } from "./blockMetrics";

describe("visualLines", () => {
  it("linha que cabe ocupa uma linha visual", () => {
    expect(visualLines("elyra-connect", 80)).toBe(1);
  });

  it("linha vazia ainda ocupa uma linha", () => {
    expect(visualLines("", 80)).toBe(1);
  });

  it("linha exatamente na largura não quebra", () => {
    expect(visualLines("x".repeat(80), 80)).toBe(1);
  });

  it("linha maior que a largura conta as linhas da quebra", () => {
    expect(visualLines("x".repeat(200), 80)).toBe(3);
    expect(visualLines("x".repeat(81), 80)).toBe(2);
  });

  // A estimativa alimenta o virtualizador, e virtualizador com altura absurda
  // deixa a barra de rolagem mentindo. Largura desconhecida cai no
  // comportamento de antes — uma linha lógica, uma linha visual.
  it("largura desconhecida devolve uma linha", () => {
    expect(visualLines("x".repeat(200), 0)).toBe(1);
    expect(visualLines("x".repeat(200), Number.NaN)).toBe(1);
    expect(visualLines("x".repeat(200), -3)).toBe(1);
  });
});

describe("bodyLines", () => {
  const line = (text: string) => ({ text, runs: [] });

  it("soma as linhas visuais de cada linha lógica", () => {
    const lines = [line("curta"), line("y".repeat(150)), line("outra")];
    expect(bodyLines(lines, 50, 100)).toBe(1 + 3 + 1);
  });

  it("para no limite de linhas desenhadas", () => {
    const lines = Array.from({ length: 500 }, () => line("z".repeat(100)));
    // 200 linhas desenhadas × 2 linhas visuais cada.
    expect(bodyLines(lines, 50, 200)).toBe(400);
  });

  it("corpo vazio não ocupa altura", () => {
    expect(bodyLines([], 50, 200)).toBe(0);
  });
});

describe("columnsFor", () => {
  // O padding chega somado dos dois lados — é a largura que sobra para o texto.
  it("desconta o padding lateral antes de dividir", () => {
    expect(columnsFor(820, 8, 20)).toBe(100);
  });

  it("painel estreito demais não devolve zero colunas", () => {
    expect(columnsFor(10, 40, 8)).toBe(1);
  });

  it("medida ausente devolve largura desconhecida", () => {
    expect(columnsFor(0, 8, 10)).toBe(0);
    expect(columnsFor(800, 0, 10)).toBe(0);
    expect(columnsFor(800, Number.NaN, 10)).toBe(0);
  });

  // O padding é constante do layout, não medida: se vier sujo, a conta segue
  // sem ele em vez de derrubar a estimativa inteira para "desconhecida".
  it("padding inválido conta como zero", () => {
    expect(columnsFor(800, 8, Number.NaN)).toBe(100);
  });
});
