import { expect, test } from "bun:test";

import { nodeTerminalClass, nodeTerminalWebgl } from "./nodeTerminal";

test("WebGL só em 100%; fora disso renderer DOM (texto nítido sob o zoom)", () => {
  expect(nodeTerminalWebgl(1)).toBe(true);
  expect(nodeTerminalWebgl(0.999)).toBe(true);
  expect(nodeTerminalWebgl(0.6)).toBe(false);
  expect(nodeTerminalWebgl(1.5)).toBe(false);
});

test("a roda é do terminal só com o nó selecionado; senão é do canvas", () => {
  expect(nodeTerminalClass(true).split(" ")).toContain("nowheel");
  expect(nodeTerminalClass(false).split(" ")).not.toContain("nowheel");
  for (const selected of [true, false]) {
    const cls = nodeTerminalClass(selected).split(" ");
    expect(cls).toContain("nodrag");
    expect(cls).toContain("nokey");
  }
});
