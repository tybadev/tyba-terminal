import { expect, test } from "bun:test";

import { DEFAULT_SIZE, newNode } from "./maestro";
import { firstLine } from "./nodeSummary";

test("nos-9: a linha recolhida é a primeira com texto", () => {
  expect(firstLine("subir ambiente\nabrir o Code")).toBe("subir ambiente");
  expect(firstLine("\n   \n  bun dev  \n")).toBe("bun dev");
  expect(firstLine("")).toBe("");
  expect(firstLine(" \n ")).toBe("");
});

test("nos-9: agente e processo nascem com tamanho de trabalho", () => {
  expect(DEFAULT_SIZE.agent).toEqual({ w: 560, h: 440 });
  expect(DEFAULT_SIZE.process).toEqual({ w: 560, h: 440 });
  const agent = newNode(
    "a",
    "dev",
    { type: "agent", runner: "claude", role: "", prompt: "", dir: "" },
    { x: 0, y: 0 },
  );
  expect(agent.size).toEqual({ w: 560, h: 440 });
});
