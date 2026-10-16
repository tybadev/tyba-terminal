import { describe, expect, test } from "bun:test";

import {
  NEW_IN_HANDLE,
  OUT_HANDLE,
  accepts,
  connectionKinds,
  connectionProblem,
  edgeKindChoices,
  emptyGraph,
  fromFlow,
  integrationsById,
  newNode,
  pickConnection,
  produces,
  toFlow,
  type EdgeKind,
  type Graph,
  type Integration,
  type OrbitNode,
  lastDir,
  lastGraphId,
  missingImportDirs,
  nodeDir,
  pickGraph,
  rememberDir,
  rememberGraph,
  shortDir,
} from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const claude = newNode(
  "a",
  "Claude · API",
  { type: "agent", runner: "claude", role: "implementador", prompt: "GET /saude", dir: "" },
  { x: 40, y: 40 },
);
const tests = newNode(
  "t",
  "Testes",
  { type: "process", command: "bun test", output: "json", file: null, dir: "" },
  { x: 540, y: 230 },
);
const note = newNode("n", "Decisões", { type: "note", markdown: "- UTC" }, { x: 540, y: 560 });
const lint = newNode("i", "Lint", { type: "integration", integrationId: "lint", dir: "" }, { x: 1040, y: 120 });
const manualHook = newNode(
  "m",
  "Avisar",
  { type: "integration", integrationId: "avisar", dir: "" },
  { x: 1040, y: 400 },
);

const integrations = integrationsById([
  {
    id: "lint",
    name: "Lint",
    script: "eslint .",
    trigger: { type: "onInput", kind: "diff" },
    output: "log",
    file: null,
  },
  {
    id: "avisar",
    name: "Avisar",
    script: "echo pronto",
    trigger: { type: "manual" },
    output: "json",
    file: null,
  },
] satisfies Integration[]);

const nodes: OrbitNode[] = [claude, tests, note, lint, manualHook];

const graph: Graph = {
  id: "g1",
  name: "rota-saude",
  dir: "/tmp/rota-saude",
  variables: { porta: "5173" },
  nodes,
  edges: [
    { id: "e1", from: "a", to: "t", kind: "diff" },
    { id: "e2", from: "n", to: "a", kind: "file" },
  ],
};

describe("produces / accepts", () => {
  test("seguem a tabela de rules", () => {
    expect(produces(claude.kind, integrations)).toEqual([
      "context",
      "diff",
      "plan",
      "control",
      "talk",
      "memory",
    ]);
    expect(produces(tests.kind, integrations)).toEqual(["json", "url"]);
    expect(produces(note.kind, integrations)).toEqual(["context", "file"]);
    expect(produces(lint.kind, integrations)).toEqual(["log"]);
    expect(accepts(note.kind, integrations)).toEqual(["plan", "memory"]);
    expect(accepts(claude.kind, integrations)).toEqual([
      "context",
      "diff",
      "file",
      "log",
      "json",
      "url",
      "talk",
    ]);
    expect(accepts(lint.kind, integrations)).toEqual(["diff"]);
    expect(accepts(manualHook.kind, integrations)).toHaveLength(6);
  });

  test("integração apagada não produz nem aceita nada", () => {
    const orphan = { type: "integration", integrationId: "sumiu", dir: "" } as const;
    expect(produces(orphan, integrations)).toEqual([]);
    expect(accepts(orphan, integrations)).toEqual([]);
  });
});

describe("connectionProblem", () => {
  const check = (from: string, to: string, kind: Parameters<typeof connectionProblem>[5]) =>
    connectionProblem(nodes, graph.edges, integrations, from, to, kind);

  test("ligação válida", () => {
    expect(check("a", "i", "diff")).toBeNull();
    expect(check("t", "m", "json")).toBeNull();
  });

  test("nota não recebe nada", () => {
    expect(check("a", "n", "diff")).toBe("notAccepted");
  });

  test("tipo que a origem não produz", () => {
    expect(check("t", "i", "diff")).toBe("notProduced");
    expect(check("n", "t", "diff")).toBe("notProduced");
  });

  test("integração por gatilho só aceita o tipo do gatilho", () => {
    expect(check("a", "i", "context")).toBe("notAccepted");
  });

  test("ciclo, laço e repetida recusados", () => {
    // t → a fecharia a → t → a.
    expect(check("t", "a", "json")).toBe("cycle");
    expect(check("a", "a", "context")).toBe("self");
    expect(check("a", "t", "diff")).toBe("duplicate");
  });

  test("nó inexistente", () => {
    expect(check("x", "t", "log")).toBe("unknownNode");
  });
});

describe("toFlow / fromFlow", () => {
  test("round-trip preserva o grafo", () => {
    const flow = toFlow(graph);
    expect(fromFlow(graph, flow.nodes, flow.edges)).toEqual(graph);
  });

  test("portas: uma saída e uma entrada por nó, o tipo vai na ligação", () => {
    const { nodes: fn, edges: fe } = toFlow(graph);
    expect(fn[0]).toMatchObject({
      id: "a",
      type: "agent",
      position: { x: 40, y: 40 },
      width: 560,
      height: 440,
    });
    expect(fe[0]).toMatchObject({
      source: "a",
      target: "t",
      sourceHandle: OUT_HANDLE,
      targetHandle: NEW_IN_HANDLE,
      data: { kind: "diff" },
    });
  });

  test("posição e tamanho saem do canvas", () => {
    const flow = toFlow(graph);
    const moved = flow.nodes.map((n) =>
      n.id === "a" ? { ...n, position: { x: 10, y: 20 }, width: 500, height: 420 } : n,
    );
    const back = fromFlow(graph, moved, flow.edges);
    expect(back.nodes[0].position).toEqual({ x: 10, y: 20 });
    expect(back.nodes[0].size).toEqual({ w: 500, h: 420 });
  });

  test("grafo de antes da porta única abre com todas as ligações e os tipos", () => {
    // Antes: uma porta por tipo (`out:diff`, `in:<id>`). O core nunca guardou
    // porta, só o tipo — a migração é o próprio `toFlow`.
    const old: Graph = {
      ...graph,
      edges: [
        { id: "e1", from: "a", to: "t", kind: "diff" },
        { id: "e2", from: "n", to: "a", kind: "file" },
        { id: "e3", from: "a", to: "m", kind: "context" },
        { id: "e4", from: "t", to: "m", kind: "url" },
      ],
    };
    const flow = toFlow(old);
    expect(flow.edges.map((e) => [e.id, e.sourceHandle, e.targetHandle, e.data.kind])).toEqual([
      ["e1", OUT_HANDLE, NEW_IN_HANDLE, "diff"],
      ["e2", OUT_HANDLE, NEW_IN_HANDLE, "file"],
      ["e3", OUT_HANDLE, NEW_IN_HANDLE, "context"],
      ["e4", OUT_HANDLE, NEW_IN_HANDLE, "url"],
    ]);
    expect(fromFlow(old, flow.nodes, flow.edges)).toEqual(old);
  });

  test("ligação sem tipo não vai para o core", () => {
    const back = fromFlow(graph, toFlow(graph).nodes, [
      { id: "e9", source: "a", target: "i", data: { kind: "diff" } },
      { id: "e10", source: "a", target: "i", sourceHandle: "out:diff" },
    ]);
    expect(back.edges).toEqual([{ id: "e9", from: "a", to: "i", kind: "diff" }]);
  });
});

describe("tipo da ligação calculado (canvas-3)", () => {
  const browser = newNode("b", "Página", { type: "browser", url: "" }, { x: 0, y: 0 });
  const service = newNode(
    "s",
    "API",
    { type: "service", source: { type: "port", sessionId: null, port: 3000 } },
    { x: 0, y: 0 },
  );
  const reviewer = newNode(
    "r",
    "Revisor",
    { type: "agent", runner: "codex", role: "", prompt: "", dir: "" },
    { x: 0, y: 0 },
  );
  const logProc = newNode(
    "p",
    "API",
    { type: "process", command: "bun dev", output: "log", file: null, dir: "" },
    { x: 0, y: 0 },
  );
  const kinds = (from: OrbitNode, to: OrbitNode) =>
    connectionKinds(from.kind, to.kind, integrations);

  test("padrão por par vem primeiro", () => {
    expect(kinds(note, claude)[0]).toBe("context");
    expect(kinds(claude, reviewer)[0]).toBe("context");
    expect(kinds(claude, tests)).toEqual(["diff", "context"]);
    expect(kinds(claude, manualHook)[0]).toBe("diff");
    expect(kinds(logProc, browser)).toEqual(["url"]);
    expect(kinds(service, browser)).toEqual(["url"]);
    expect(kinds(logProc, claude)[0]).toBe("log");
  });

  test("par sem tipo em comum não liga", () => {
    expect(kinds(note, browser)).toEqual([]);
    expect(kinds(note, note)).toEqual([]);
    // Integração por gatilho de diff: só o diff do agente.
    expect(kinds(claude, lint)).toEqual(["diff"]);
    expect(kinds(note, lint)).toEqual([]);
  });

  test("pickConnection: tipo calculado ou o motivo", () => {
    const all = [...nodes, browser, reviewer, logProc];
    const pick = (from: string, to: string) =>
      pickConnection(all, graph.edges, integrations, from, to);
    expect(pick("a", "r")).toEqual({ kind: "context" });
    expect(pick("p", "b")).toEqual({ kind: "url" });
    expect(pick("a", "i")).toEqual({ kind: "diff" });
    expect(pick("r", "n")).toEqual({ kind: "plan" });
    // A nota já dá contexto ao a: o plano de volta não leva valor, então não
    // fecha ciclo (espelha `check_acyclic`).
    expect(pick("a", "n")).toEqual({ kind: "plan" });
    expect(pick("n", "b")).toEqual({ problem: "notAccepted" });
    expect(pick("a", "a")).toEqual({ problem: "self" });
    expect(pick("x", "a")).toEqual({ problem: "unknownNode" });
    // Uma ligação por par: a → t já existe (diff); troca-se pela etiqueta.
    expect(pick("a", "t")).toEqual({ problem: "duplicate" });
    // t → a fecharia a → t → a.
    expect(pick("t", "a")).toEqual({ problem: "cycle" });
  });

  test("edgeKindChoices: troca só entre os compatíveis do par", () => {
    expect(edgeKindChoices(nodes, graph.edges, integrations, "e1")).toEqual(["diff", "context"]);
    expect(edgeKindChoices(nodes, graph.edges, integrations, "e2")).toEqual(["context", "file"]);
    const twin = [...graph.edges, { id: "e5", from: "a", to: "t", kind: "context" as const }];
    expect(edgeKindChoices(nodes, twin, integrations, "e1")).toEqual(["diff"]);
    expect(edgeKindChoices(nodes, graph.edges, integrations, "sumiu")).toEqual([]);
  });
});

test("emptyGraph leva nome e pasta padrão", () => {
  expect(emptyGraph("g", "Revisão", "/Users/x/rota-saude")).toEqual({
    id: "g",
    name: "Revisão",
    dir: "/Users/x/rota-saude",
    nodes: [],
    edges: [],
    variables: {},
  });
});

describe("pasta por nó", () => {
  test("nodeDir: a do nó, a padrão sem ela, nada na nota", () => {
    const own = { ...claude.kind, dir: "/srv/api" } as typeof claude.kind;
    expect(nodeDir(own, "/srv/padrao")).toBe("/srv/api");
    expect(nodeDir(claude.kind, "/srv/padrao")).toBe("/srv/padrao");
    expect(nodeDir(note.kind, "/srv/padrao")).toBeNull();
  });

  test("shortDir mostra as duas últimas partes", () => {
    expect(shortDir("/Users/x/projetos/api")).toBe("…/projetos/api");
    expect(shortDir("C:\\Users\\x\\api")).toBe("…/x/api");
    expect(shortDir("/srv/api")).toBe("/srv/api");
  });

  test("pickGraph abre o último usado, ou o mais recente", () => {
    const a = { ...graph, id: "a" };
    const b = { ...graph, id: "b" };
    expect(pickGraph([a, b], "b")).toBe(b);
    expect(pickGraph([a, b], "sumiu")).toBe(a);
    expect(pickGraph([a, b], null)).toBe(a);
    expect(pickGraph([], "b")).toBeNull();
  });

  test("missingImportDirs: só a pasta inexistente sem escolha segura o Confirmar", () => {
    const dirs = [
      { nodeId: "a", dir: "/aqui", exists: true },
      { nodeId: "t", dir: "/de/outra/maquina", exists: false },
      { nodeId: "i", dir: "C:\\x", exists: false },
    ];
    expect(missingImportDirs(dirs, {})).toEqual(["t", "i"]);
    expect(missingImportDirs(dirs, { t: "/escolhida" })).toEqual(["i"]);
    expect(missingImportDirs(dirs, { t: "/escolhida", i: "/outra" })).toEqual([]);
  });

  test("sem localStorage, a última pasta e o último grafo voltam vazios sem erro", () => {
    expect(() => rememberDir("/srv/api")).not.toThrow();
    expect(() => rememberGraph("g")).not.toThrow();
    if (typeof localStorage === "undefined") {
      expect(lastDir()).toBeNull();
      expect(lastGraphId()).toBeNull();
    } else {
      expect(lastDir()).toBe("/srv/api");
      expect(lastGraphId()).toBe("g");
    }
  });

  test("a chave antiga do Maestro vale até a nova ser gravada", () => {
    const g = globalThis as { localStorage?: unknown };
    const saved = Object.getOwnPropertyDescriptor(g, "localStorage");
    const data = new Map<string, string>([
      ["tyba.maestro.lastDir", "/antiga"],
      ["tyba.maestro.lastGraph", "g-velho"],
    ]);
    Object.defineProperty(g, "localStorage", {
      configurable: true,
      value: {
        getItem: (k: string) => data.get(k) ?? null,
        setItem: (k: string, v: string) => void data.set(k, v),
      },
    });
    try {
      expect(lastDir()).toBe("/antiga");
      expect(lastGraphId()).toBe("g-velho");
      rememberDir("/nova");
      rememberGraph("g-novo");
      expect(data.get("tyba.orbit.lastDir")).toBe("/nova");
      expect(data.get("tyba.maestro.lastDir")).toBe("/antiga");
      expect(lastDir()).toBe("/nova");
      expect(lastGraphId()).toBe("g-novo");
    } finally {
      if (saved) Object.defineProperty(g, "localStorage", saved);
      else delete g.localStorage;
    }
  });
});

describe("conversa e memória", () => {
  const a2: OrbitNode = { ...claude, id: "a2" };
  const all = [claude, a2, note];
  const problem = (edges: Graph["edges"], from: string, to: string, kind: EdgeKind) =>
    connectionProblem(all, edges, integrations, from, to, kind);

  test("conversa: agente ⇄ agente, uma por par, sem ciclo", () => {
    expect(problem([], "a", "a2", "talk")).toBeNull();
    expect(problem([], "a", "n", "talk")).toBe("notAccepted");
    const talk = [{ id: "t", from: "a", to: "a2", kind: "talk" as const }];
    expect(problem(talk, "a2", "a", "talk")).toBe("duplicate");
    // Conversa não leva valor: o contexto de volta não fecha ciclo.
    expect(problem(talk, "a2", "a", "context")).toBeNull();
  });

  test("memória: agente → nota", () => {
    expect(problem([], "a", "n", "memory")).toBeNull();
    expect(problem([], "a", "a2", "memory")).toBe("notAccepted");
    expect(problem([], "n", "a", "memory")).toBe("notProduced");
  });
});
