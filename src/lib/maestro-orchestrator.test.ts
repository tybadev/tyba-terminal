import { describe, expect, test } from "bun:test";

import {
  createdDetail,
  ephemeralCreated,
  importItems,
  newNode,
  type Created,
  type Graph,
  type Run,
} from "./maestro";

// Fixtures sintéticas: nada aqui veio de sessão real.
const created = (id: string, patch: Partial<Created> = {}): Created => ({
  node: newNode(
    id,
    id,
    { type: "process", command: "bun run dev", output: "log", file: null, dir: "/srv/api" },
    { x: 0, y: 0 },
  ),
  createdBy: "o",
  createdAt: 1_000,
  edges: [],
  kept: false,
  ...patch,
});

const run = (status: Run["status"], list: Created[]): Run => ({
  id: "r",
  graphId: "g",
  startedAt: 0,
  finishedAt: null,
  status,
  nodes: {},
  created: list,
});

describe("orquestrador: nós criados", () => {
  test("só o run rodando desenha os criados, e só os ainda não mantidos", () => {
    const list = [created("orq-1"), created("orq-2", { kept: true })];
    expect(ephemeralCreated(run("running", list)).map((c) => c.node.id)).toEqual(["orq-1"]);
    // ■ Parar ou fim do run: somem.
    expect(ephemeralCreated(run("stopped", list))).toEqual([]);
    expect(ephemeralCreated(run("done", list))).toEqual([]);
    expect(ephemeralCreated(null)).toEqual([]);
    // Run gravado antes do orquestrador não tem `created`.
    expect(ephemeralCreated({ ...run("running", []), created: undefined })).toEqual([]);
  });

  test("o painel mostra comando e pasta, ou a origem da url do navegador", () => {
    expect(createdDetail(created("orq-1"), () => undefined)).toEqual({
      text: "bun run dev",
      from: null,
      dir: "/srv/api",
    });
    const browser = created("orq-2", {
      node: newNode("orq-2", "code", { type: "browser", url: "" }, { x: 0, y: 0 }),
      edges: [{ id: "orq-2-url", from: "orq-1", to: "orq-2", kind: "url" }],
    });
    const titles: Record<string, string> = { "orq-1": "code-platform" };
    expect(createdDetail(browser, (id) => titles[id])).toEqual({
      text: null,
      from: "code-platform",
      dir: null,
    });
  });

  test("a prévia de importação mostra que o nó orquestra", () => {
    const graph: Graph = {
      id: "g",
      name: "subir",
      dir: "/tmp",
      variables: {},
      edges: [],
      nodes: [
        newNode(
          "o",
          "Orquestrador",
          { type: "agent", runner: "claude", role: "", prompt: "suba", dir: "", orchestrate: true },
          { x: 0, y: 0 },
        ),
        newNode("a", "Claude", { type: "agent", runner: "claude", role: "", prompt: "x", dir: "" }, {
          x: 0,
          y: 0,
        }),
      ],
    };
    const items = importItems({ graph, integrations: [] });
    expect(items[0]).toMatchObject({ type: "agent", orchestrate: true });
    // Grafo de antes do orquestrador: campo ausente = não orquestra.
    expect(items[1]).toMatchObject({ type: "agent", orchestrate: false });
  });
});
