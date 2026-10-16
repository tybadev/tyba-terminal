import { describe, expect, test } from "bun:test";

import {
  edgeKindChoices,
  newNode,
  pickConnection,
  type Integration,
  type NodeRun,
  type OrbitEdge,
  type Run,
} from "./orbit";
import { noteEditable } from "./orbitPlan";
import {
  browserControllers,
  clockTime,
  freshWrite,
  inboxSenders,
  lastNoteWritten,
  memoryAgents,
  noteMemoryLabel,
  roomAgents,
  talkQueued,
  undeliveredReason,
  writeOnBlur,
} from "./orbitTalk";

// Fixtures sintéticas: nada aqui veio de sessão real.

const none = new Map<string, Integration>();
const agent = (id: string) =>
  newNode(id, id, { type: "agent", runner: "claude", role: "", prompt: "", dir: "" }, { x: 0, y: 0 });
const note = (id: string) => newNode(id, id, { type: "note", markdown: "" }, { x: 0, y: 0 });

describe("tipos que a ligação oferece (conversa e memória)", () => {
  const nodes = [agent("dev"), agent("rev"), note("decisoes"), note("plano")];
  const planEdge: OrbitEdge = { id: "pl", from: "dev", to: "plano", kind: "plan" };

  test("agente → nota comum nasce memória; nota que já é plano, plano", () => {
    expect(pickConnection(nodes, [], none, "dev", "decisoes")).toEqual({ kind: "memory" });
    expect(pickConnection(nodes, [planEdge], none, "rev", "plano")).toEqual({ kind: "plan" });
  });

  test("a etiqueta de memória troca para plano e volta; nada além disso", () => {
    const edges: OrbitEdge[] = [{ id: "m", from: "dev", to: "decisoes", kind: "memory" }];
    expect(edgeKindChoices(nodes, edges, none, "m")).toEqual(["memory", "plan"]);
  });

  test("agente → agente oferece conversa junto do que leva valor", () => {
    const edges: OrbitEdge[] = [{ id: "c", from: "dev", to: "rev", kind: "context" }];
    expect(edgeKindChoices(nodes, edges, none, "c")).toContain("talk");
    // Nota não conversa.
    const toNote: OrbitEdge[] = [{ id: "m", from: "dev", to: "decisoes", kind: "memory" }];
    expect(edgeKindChoices(nodes, toNote, none, "m")).not.toContain("talk");
  });

  test("conversa vale nos dois sentidos: a de volta não oferece outra", () => {
    const edges: OrbitEdge[] = [
      { id: "t", from: "dev", to: "rev", kind: "talk" },
      { id: "c", from: "rev", to: "dev", kind: "context" },
    ];
    expect(edgeKindChoices(nodes, edges, none, "c")).not.toContain("talk");
    expect(edgeKindChoices(nodes, edges, none, "t")).toContain("talk");
  });
});

describe("fila da conversa e nota viva", () => {
  const nodeRun = (patch: Partial<NodeRun>): NodeRun => ({
    status: "running",
    sessionId: null,
    startedAt: 1,
    finishedAt: null,
    exitCode: null,
    outputs: {},
    reason: null,
    ...patch,
  });
  const run = (nodes: Record<string, NodeRun>): Run => ({
    id: "r",
    graphId: "g",
    startedAt: 1,
    finishedAt: null,
    status: "running",
    nodes,
  });

  test("a etiqueta conta a fila do par, nos dois sentidos, e só dele", () => {
    const r = run({
      rev: nodeRun({ inbox: 3, inboxFrom: { dev: 1, outro: 2 } }),
      dev: nodeRun({ inbox: 1, inboxFrom: { rev: 1 } }),
    });
    expect(talkQueued({ from: "dev", to: "rev" }, r)).toBe(2);
    expect(talkQueued({ from: "rev", to: "dev" }, r)).toBe(2);
    expect(talkQueued({ from: "dev", to: "outro" }, r)).toBe(0);
    expect(talkQueued({ from: "dev", to: "rev" }, null)).toBe(0);
    // Retrato antigo, sem `inboxFrom`.
    expect(talkQueued({ from: "dev", to: "rev" }, run({ rev: nodeRun({ inbox: 1 }) }))).toBe(0);
  });

  test("agentes da nota: os ligados por memória, cada um uma vez", () => {
    const edges: OrbitEdge[] = [
      { id: "1", from: "dev", to: "n", kind: "memory" },
      { id: "2", from: "rev", to: "n", kind: "memory" },
      { id: "3", from: "dev", to: "outra", kind: "memory" },
      { id: "4", from: "x", to: "n", kind: "plan" },
    ];
    expect(memoryAgents("n", edges)).toBe(2);
    expect(memoryAgents("solta", edges)).toBe(0);
  });

  test("cápsula: ao vivo só com o run rodando; com cadeado eles leem", () => {
    expect(noteMemoryLabel(0, false, true)).toBeNull();
    expect(noteMemoryLabel(2, false, true)).toEqual({
      key: "orbitNoteAgentsWriteLive",
      params: { count: 2 },
    });
    expect(noteMemoryLabel(2, false, false)?.key).toBe("orbitNoteAgentsWrite");
    expect(noteMemoryLabel(1, true, true)?.key).toBe("orbitNoteAgentsReadLive");
    expect(noteMemoryLabel(1, true, false)?.key).toBe("orbitNoteAgentsRead");
  });

  test("cadeado tranca só os agentes: o dono segue editando; plano não", () => {
    expect(noteEditable("n", [])).toBe(true);
    expect(noteEditable("n", [{ to: "n", kind: "memory" }])).toBe(true);
    expect(noteEditable("n", [{ to: "n", kind: "plan" }])).toBe(false);
  });

  test("escrita de agente com o dono digitando: entra ao sair só se ele não mudou nada", () => {
    // Ele não mexeu: o texto do agente entra.
    expect(writeOnBlur("a", "a", "a\n- do agente")).toBe("a\n- do agente");
    // Ele mexeu: o dele vence (o salvar dele vai ao core).
    expect(writeOnBlur("a", "a e mais", "a\n- do agente")).toBeNull();
    // Nada chegou, ou chegou o mesmo texto: nada a fazer.
    expect(writeOnBlur("a", "a", null)).toBeNull();
    expect(writeOnBlur("a", "a", "a")).toBeNull();
  });

  test("escrita de agente entra na tela uma vez, e só a de depois da abertura", () => {
    expect(freshWrite("n", 5, new Map(), 10)).toBe(false);
    expect(freshWrite("n", 20, new Map(), 10)).toBe(true);
    expect(freshWrite("n", 20, new Map([["n", 20]]), 10)).toBe(false);
    expect(freshWrite("n", 30, new Map([["n", 20]]), 10)).toBe(true);
  });

  test("hora de quem escreveu: HH:MM local", () => {
    expect(clockTime(new Date(2026, 9, 16, 14, 22, 59).getTime())).toBe("14:22");
    expect(clockTime(new Date(2026, 9, 16, 9, 5).getTime())).toBe("09:05");
  });
});

describe("conversa e memória nos cards e na barra", () => {
  const nodes = [
    { id: "dev", title: "Claude · dev" },
    { id: "rev", title: "Codex · revisor" },
    { id: "nav", title: "Navegador · code" },
    { id: "dec", title: "decisoes.md" },
    { id: "out", title: "outra.md" },
  ];
  const edges: OrbitEdge[] = [
    { id: "c", from: "dev", to: "nav", kind: "control" },
    { id: "t", from: "dev", to: "rev", kind: "talk" },
    { id: "m1", from: "dev", to: "dec", kind: "memory" },
    { id: "m2", from: "dev", to: "out", kind: "memory" },
    { id: "m3", from: "rev", to: "dec", kind: "memory" },
  ];
  const nr = (extra: Partial<NodeRun>): NodeRun => ({
    status: "running",
    sessionId: null,
    startedAt: 1,
    finishedAt: null,
    exitCode: null,
    outputs: {},
    reason: null,
    ...extra,
  });

  test("navegador: controlado pelo agente ligado por control, só enquanto a ligação existe", () => {
    expect(browserControllers("nav", nodes, edges)).toEqual(["Claude · dev"]);
    expect(browserControllers("nav", nodes, edges.filter((e) => e.kind !== "control"))).toEqual([]);
    // Ligação de outro tipo não controla.
    expect(browserControllers("rev", nodes, edges)).toEqual([]);
  });

  test("agente: a nota mais recente em que ele escreveu, nunca a de outro", () => {
    const run = {
      nodes: {
        dec: nr({ written: { by: "Claude · dev", at: 10, markdown: "" } }),
        out: nr({ written: { by: "Claude · dev", at: 20, markdown: "" } }),
      },
    };
    expect(lastNoteWritten("dev", nodes, edges, run)).toBe("outra.md");
    // A última escrita de decisoes.md é do dev, não do revisor.
    expect(lastNoteWritten("rev", nodes, edges, run)).toBeNull();
    expect(lastNoteWritten("dev", nodes, edges, null)).toBeNull();
    // Sem a ligação de memória, não conta.
    expect(lastNoteWritten("dev", nodes, [], run)).toBeNull();
  });

  test("destino: quem tem mensagem na fila, pelo título", () => {
    expect(inboxSenders(nr({ inboxFrom: { dev: 1, rev: 0 } }), nodes)).toEqual(["Claude · dev"]);
    expect(inboxSenders(nr({}), nodes)).toEqual([]);
    expect(inboxSenders(undefined, nodes)).toEqual([]);
  });

  test("não entregue: o motivo curto, sem o prefixo do core", () => {
    expect(undeliveredReason("não entregue: sessão fechou")).toBe("sessão fechou");
    expect(undeliveredReason("não entregue: a\nb")).toBe("a");
    const long = undeliveredReason(`não entregue: ${"x".repeat(100)}`);
    expect(long.length).toBe(60);
    expect(long.endsWith("…")).toBe(true);
  });

  test("sala: agentes com alguma ligação de conversa", () => {
    expect(roomAgents(edges)).toBe(2);
    expect(roomAgents([...edges, { id: "t2", from: "rev", to: "x", kind: "talk" }])).toBe(3);
    expect(roomAgents(edges.filter((e) => e.kind !== "talk"))).toBe(0);
  });
});
