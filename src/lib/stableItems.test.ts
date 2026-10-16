import { expect, test } from "bun:test";

import { keepIfSame } from "./stableItems";

test("canvas-7: mesmos itens devolvem a lista anterior (arrastar não redesenha)", () => {
  const a = { id: "a" };
  const b = { id: "b" };
  const prev = [a, b];
  expect(keepIfSame(prev, [a, b])).toBe(prev);
});

test("canvas-7: item trocado, a mais ou fora de ordem devolve a lista nova", () => {
  const a = { id: "a" };
  const b = { id: "b" };
  const prev = [a, b];
  const edited = [a, { ...b }];
  expect(keepIfSame(prev, edited)).toBe(edited);
  const grown = [a, b, { id: "c" }];
  expect(keepIfSame(prev, grown)).toBe(grown);
  const swapped = [b, a];
  expect(keepIfSame(prev, swapped)).toBe(swapped);
  const first = [a];
  expect(keepIfSame(null, first)).toBe(first);
});
