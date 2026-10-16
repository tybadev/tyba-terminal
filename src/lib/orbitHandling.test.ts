import { describe, expect, test } from "bun:test";

import type { NodeKind, NodeRun, NodeStatus, Run } from "./orbit";
import { deletePlan, deletedNotice, sizeLabel, snapToGuides, SNAP_PX } from "./orbitHandling";

describe("snapToGuides", () => {
  const other = { x: 0, y: 0, w: 400, h: 200 };

  test("longe de todos: fica onde está, sem guia", () => {
    const r = snapToGuides({ x: 500, y: 300, w: 400, h: 200 }, [other]);
    expect(r).toEqual({ x: 500, y: 300, guides: [] });
  });

  test("borda esquerda a 4px da do outro gruda nela, com a guia de um card ao outro", () => {
    const r = snapToGuides({ x: 4, y: 300, w: 300, h: 100 }, [other]);
    expect(r.x).toBe(0);
    expect(r.y).toBe(300);
    expect(r.guides).toEqual([{ axis: "x", at: 0, from: 0, to: 400 }]);
  });

  test("passou dos 6px: não gruda", () => {
    const r = snapToGuides({ x: SNAP_PX + 1, y: 300, w: 300, h: 100 }, [other]);
    expect(r.x).toBe(SNAP_PX + 1);
    expect(r.guides).toEqual([]);
  });

  test("centro com centro e topo com topo, nos dois eixos", () => {
    // Centro do outro em x=200; o card de 100 de largura com centro em 203.
    const r = snapToGuides({ x: 153, y: -5, w: 100, h: 80 }, [{ ...other, x: 0 }]);
    expect(r.x).toBe(150);
    expect(r.y).toBe(0);
    expect(r.guides).toContainEqual({ axis: "x", at: 200, from: 0, to: 200 });
    expect(r.guides).toContainEqual({ axis: "y", at: 0, from: 0, to: 400 });
  });

  test("o mais perto vence entre vários", () => {
    const r = snapToGuides({ x: 0, y: 503, w: 100, h: 100 }, [
      { x: 300, y: 498, w: 100, h: 100 },
      { x: 600, y: 502, w: 100, h: 100 },
    ]);
    expect(r.y).toBe(502);
    expect(r.guides.every((g) => g.axis === "y" && g.from === 0 && g.to === 700)).toBe(true);
  });

  test("o limite é o da tela: com zoom, quem chama divide", () => {
    const r = snapToGuides({ x: 10, y: 300, w: 300, h: 100 }, [other], SNAP_PX / 0.5);
    expect(r.x).toBe(0);
  });
});

test("sizeLabel arredonda e usa ×", () => {
  expect(sizeLabel(430.4, 229.6)).toBe("430 × 230");
});

describe("deletePlan", () => {
  const process: NodeKind = {
    type: "process",
    command: "bun dev",
    output: "log",
    file: null,
    dir: "/w",
  };
  const agent: NodeKind = { type: "agent", runner: "claude", role: "", prompt: "", dir: "/w" };
  const note: NodeKind = { type: "note", markdown: "" };
  const nodeRun = (status: NodeStatus, sessionId: string | null = null): NodeRun => ({
    status,
    sessionId,
    startedAt: null,
    finishedAt: null,
    exitCode: null,
    outputs: {},
    reason: null,
  });
  const run = (
    status: Run["status"],
    n: NodeRun,
    liveAgents: string[] = [],
  ): Pick<Run, "status" | "nodes" | "liveAgents"> => ({ status, nodes: { a: n }, liveAgents });

  test("sem execução: apaga na hora", () => {
    expect(deletePlan(note, null, "a", true)).toEqual({ ask: false, stop: false });
  });

  test("processo rodando pergunta e para", () => {
    expect(deletePlan(process, run("running", nodeRun("running", "s1")), "a", true)).toEqual({
      ask: true,
      stop: true,
    });
  });

  test("serviço resolvendo (rodando, sem sessão) também pergunta", () => {
    const service = {
      type: "service",
      source: { type: "port", sessionId: null, port: 80 },
    } as NodeKind;
    expect(deletePlan(service, run("running", nodeRun("running")), "a", true).ask).toBe(true);
  });

  test("agente pronto com a sessão viva pergunta, mesmo com o run fechado", () => {
    const r = run("done", nodeRun("done", "s1"), ["s1"]);
    expect(deletePlan(agent, r, "a", true)).toEqual({ ask: true, stop: true });
    // Run do histórico (não é o mais novo): o terminal vivo não é dele.
    expect(deletePlan(agent, r, "a", false).ask).toBe(false);
  });

  test("pronto, falhou ou parado: apaga na hora", () => {
    for (const s of ["done", "failed", "stopped", "skipped"] as const) {
      expect(deletePlan(process, run("running", nodeRun(s, "s1")), "a", true)).toEqual({
        ask: false,
        stop: false,
      });
    }
  });

  test("esperando no run aberto: não pergunta, mas para (não sobe sem card)", () => {
    expect(deletePlan(process, run("running", nodeRun("waiting")), "a", true)).toEqual({
      ask: false,
      stop: true,
    });
    expect(deletePlan(process, run("stopped", nodeRun("waiting")), "a", true).stop).toBe(false);
  });
});

describe("deletedNotice", () => {
  test("um nó: tipo e nome", () => {
    expect(deletedNotice([{ title: "Achados", kind: { type: "note", markdown: "" } }])).toEqual({
      key: "orbitDeleted_note",
      params: { title: "Achados" },
    });
  });

  test("vários: só quantos", () => {
    const n = { title: "x", kind: { type: "note", markdown: "" } as NodeKind };
    expect(deletedNotice([n, n, n])).toEqual({ key: "graphNodesDeleted", params: { count: 3 } });
  });
});
