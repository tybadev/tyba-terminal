import { describe, expect, test } from "bun:test";

import { withEntry } from "./perSession";

describe("withEntry", () => {
  test("guarda o valor da sessão sem tocar nas vizinhas", () => {
    const before = { a: 21.5, b: 16.2 };
    expect(withEntry(before, "a", 30)).toEqual({ a: 30, b: 16.2 });
  });

  test("cria a entrada quando a sessão ainda não tinha medida", () => {
    expect(withEntry({}, "a", 27)).toEqual({ a: 27 });
  });

  test("valor igual devolve o MESMO objeto", () => {
    // É isto que impede o laço: a medida vem de um `ResizeObserver`, e um
    // objeto novo a cada relatório re-renderiza, o que remede, o que relata
    // de novo. Comparar por valor e devolver a referência anterior corta o
    // ciclo no primeiro passo.
    const before = { a: 21.5 };
    expect(withEntry(before, "a", 21.5)).toBe(before);
  });

  test("serve para qualquer valor comparável, não só medida em px", () => {
    // O modo do tty é booleano e cai na mesma regra — a razão de a função ser
    // genérica em vez de uma cópia por tipo de estado.
    const before = { a: true };
    expect(withEntry(before, "a", true)).toBe(before);
    expect(withEntry(before, "a", false)).toEqual({ a: false });
  });

  test("uma sessão não enxerga a medida da outra", () => {
    // O defeito que motivou o `Record`: em split, os dois painéis medem
    // alturas diferentes, e com valor único o último a medir mandava na lista
    // do vizinho.
    const measures = withEntry(withEntry({}, "a", 21.5), "b", 16.2);
    expect(measures.a).toBe(21.5);
    expect(measures.b).toBe(16.2);
  });
});
