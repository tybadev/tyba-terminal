import { describe, expect, test } from "bun:test";

import {
  accepts,
  carriesData,
  connectionKinds,
  edgeHasValue,
  edgeKindChoices,
  newNode,
  produces,
  timelineSteps,
  type Integration,
  type OrbitNode,
  type Plan,
  type Run,
} from "./orbit";
import {
  applyPlans,
  checklistCount,
  freshPlans,
  isPlanNote,
  noteEditable,
  parseChecklist,
  planShown,
  planWriters,
} from "./orbitPlan";

// Fixtures sintéticas: nada aqui veio de sessão real.

describe("checklist da nota de plano", () => {
  test("lê os quatro estados e a linha criou sob o passo", () => {
    // O texto exato que o `plan_markdown` do core escreve.
    const md =
      "- [x] Subir a api\n  criou muriki-api (:3000)\n- [~] Abrir o login\n" +
      "- [ ] Escrever\n- [!] Caddy\n  criou caddy\n";
    expect(parseChecklist(md)).toEqual([
      { state: "done", text: "Subir a api", made: ["muriki-api (:3000)"] },
      { state: "doing", text: "Abrir o login", made: [] },
      { state: "todo", text: "Escrever", made: [] },
      { state: "failed", text: "Caddy", made: ["caddy"] },
    ]);
  });

  test("conta os prontos", () => {
    const steps = parseChecklist("- [x] a\n- [X] b\n- [~] c\n- [ ] d\n- [!] e")!;
    expect(checklistCount(steps)).toEqual({ done: 2, total: 5 });
  });

  test("linha em branco não quebra; \\r do fim some", () => {
    expect(parseChecklist("- [x] a\r\n\n- [ ] b\n")).toHaveLength(2);
  });

  test("texto que não é checklist renderiza normal", () => {
    expect(parseChecklist("")).toBeNull();
    expect(parseChecklist("- UTC sempre")).toBeNull();
    expect(parseChecklist("# Decisões\n- [x] a")).toBeNull();
    // Linha criou sem passo antes, ou sem recuo, não é checklist.
    expect(parseChecklist("  criou x\n- [ ] a")).toBeNull();
    expect(parseChecklist("- [ ] a\ncriou x")).toBeNull();
    // Passo sem texto ou com marca desconhecida.
    expect(parseChecklist("- [ ] ")).toBeNull();
    expect(parseChecklist("- [?] a")).toBeNull();
  });

  test("nota de plano é a que recebe ligação plano", () => {
    const edges = [
      { to: "n", kind: "plan" as const },
      { to: "m", kind: "context" as const },
    ];
    expect(isPlanNote("n", edges)).toBe(true);
    expect(isPlanNote("m", edges)).toBe(false);
  });

  test("nota de plano é só leitura; a comum se edita", () => {
    const edges = [
      { to: "n", kind: "plan" as const },
      { to: "m", kind: "context" as const },
    ];
    expect(noteEditable("n", edges)).toBe(false);
    expect(noteEditable("m", edges)).toBe(true);
    expect(noteEditable("solta", [])).toBe(true);
  });
});

describe("plano do run na tela", () => {
  const note = newNode("plano-1", "Plano", { type: "note", markdown: "- [ ] a\n" }, { x: 5, y: 9 });
  const plan = (updatedAt: number, markdown = "- [x] a\n"): Plan => ({
    orchestrator: "o",
    note: { ...note, kind: { type: "note", markdown } },
    edge: { id: "plano-1-plan", from: "o", to: "plano-1", kind: "plan" },
    steps: [{ text: "a", state: "done" }],
    updatedAt,
  });
  const flow = (n: OrbitNode) => ({ id: n.id, data: { node: n } });

  test("só o plano escrito depois da leitura e mais novo que o já posto", () => {
    expect(freshPlans([plan(10)], new Map(), 20)).toEqual([]);
    expect(freshPlans([plan(30)], new Map(), 20)).toHaveLength(1);
    expect(freshPlans([plan(30)], new Map([["plano-1", 30]]), 20)).toEqual([]);
  });

  test("nota que falta entra com a ligação; a que está só troca o texto", () => {
    const orq = newNode(
      "o",
      "Orquestrador",
      { type: "agent", runner: "claude", role: "", prompt: "", dir: "" },
      { x: 0, y: 0 },
    );
    const added = applyPlans([flow(orq)], [], [plan(1)]);
    expect(added.nodes.map((n) => n.id)).toEqual(["o", "plano-1"]);
    expect(added.edges).toMatchObject([{ id: "plano-1-plan", source: "o", target: "plano-1" }]);

    // O dono arrastou a nota: fica onde está, só o texto muda.
    const moved = { ...flow(note), position: { x: 400, y: 400 } };
    const again = applyPlans([flow(orq), moved], added.edges, [plan(2, "- [!] a\n")]);
    expect(again.nodes[1]).toMatchObject({ position: { x: 400, y: 400 } });
    expect(again.nodes[1].data.node.kind).toEqual({ type: "note", markdown: "- [!] a\n" });
    expect(again.edges).toHaveLength(1);

    // Orquestrador apagado da tela: a nota vem, a ligação não.
    expect(applyPlans([], [], [plan(3)]).edges).toEqual([]);
  });
});

describe("de qual execução é o plano", () => {
  const note = newNode("plano-1", "Plano", { type: "note", markdown: "" }, { x: 0, y: 0 });
  const plan = (markdown: string): Plan => ({
    orchestrator: "o",
    note: { ...note, kind: { type: "note", markdown } },
    edge: { id: "plano-1-plan", from: "o", to: "plano-1", kind: "plan" },
    steps: markdown ? [{ text: "a", state: "done" }] : [],
    updatedAt: 1,
  });
  const run = (status: Run["status"], plans: Plan[]) => ({ status, plans });
  const base = { number: 4, live: true, writer: 4, created: false };

  test("o mais novo que escreveu dá o número", () => {
    const runs = [
      { id: "r5", plans: [] },
      { id: "r4", plans: [plan("- [x] a\n")] },
      { id: "r3", plans: [plan("- [ ] velho\n")] },
    ];
    const n: Record<string, number> = { r5: 5, r4: 4, r3: 3 };
    expect(planWriters(runs, (id) => n[id])).toEqual(new Map([["plano-1", 4]]));
  });

  test("execução rodando: atual, com o número dela", () => {
    expect(planShown("plano-1", "x", { ...base, run: run("running", [plan("- [x] a\n")]) })).toEqual({
      markdown: "- [x] a\n",
      number: 4,
      previous: false,
      waiting: false,
      none: false,
    });
  });

  test("no ▶ o plano pendente espera o agente escrever", () => {
    const shown = planShown("plano-1", "", { ...base, run: run("running", [plan("")]) });
    expect(shown.waiting).toBe(true);
    expect(shown.previous).toBe(false);
  });

  test("execução terminada ou parada: apagada, da execução anterior", () => {
    for (const status of ["done", "stopped", "failed"] as const) {
      const shown = planShown("plano-1", "x", { ...base, run: run(status, [plan("- [x] a\n")]) });
      expect(shown.previous).toBe(true);
      expect(shown.number).toBe(4);
    }
    // Terminou sem o agente escrever.
    expect(planShown("plano-1", "", { ...base, run: run("done", [plan("")]) }).none).toBe(true);
  });

  test("Execuções → #N mostra o plano daquela execução, não o do grafo", () => {
    const shown = planShown("plano-1", "- [ ] novo\n", {
      run: run("done", [plan("- [x] velho\n")]),
      number: 2,
      live: false,
      writer: 5,
      created: false,
    });
    expect(shown.markdown).toBe("- [x] velho\n");
    expect(shown.number).toBe(2);
    expect(shown.previous).toBe(true);
  });

  test("execução à vista sem plano para a nota: o texto do grafo, apagado, com quem escreveu", () => {
    expect(planShown("plano-1", "- [x] a\n", { ...base, run: run("running", []), writer: 3 })).toEqual({
      markdown: "- [x] a\n",
      number: 3,
      previous: true,
      waiting: false,
      none: false,
    });
    // Ninguém escreveu ainda: nem número, nem "anterior".
    const fresh = planShown("plano-1", "", { ...base, run: null, writer: undefined });
    expect(fresh.number).toBeNull();
    expect(fresh.previous).toBe(false);
  });

  test("nota criada por agente do run é daquela execução", () => {
    const shown = planShown("plano-1", "- [~] a\n", { ...base, run: run("running", []), created: true });
    expect(shown.number).toBe(4);
    expect(shown.previous).toBe(false);
  });
});

describe("tipos de ligação plano e controla", () => {
  const none = new Map<string, Integration>();
  const agent = newNode(
    "o",
    "Orquestrador",
    { type: "agent", runner: "claude", role: "", prompt: "", dir: "" },
    { x: 0, y: 0 },
  );
  const proc = newNode(
    "p",
    "api",
    { type: "process", command: "bun dev", output: "log", file: null, dir: "" },
    { x: 0, y: 0 },
  );
  const note = newNode("n", "Plano", { type: "note", markdown: "" }, { x: 0, y: 0 });
  const nav = newNode("b", "Login", { type: "browser", url: "" }, { x: 0, y: 0 });
  const svc = newNode(
    "s",
    "API",
    { type: "service", source: { type: "port", sessionId: null, port: 3000 } },
    { x: 0, y: 0 },
  );
  const kinds = (a: OrbitNode, b: OrbitNode) => connectionKinds(a.kind, b.kind, none);

  test("controla só agente → navegador; plano só agente → nota", () => {
    expect(kinds(agent, nav)).toEqual(["control"]);
    expect(kinds(agent, note)).toEqual(["memory", "plan"]);
    expect(kinds(proc, nav)).toEqual(["url"]);
    expect(kinds(svc, nav)).toEqual(["url"]);
    expect(kinds(note, nav)).toEqual([]);
    // Ninguém mais aceita os dois.
    for (const target of [agent, proc, svc]) {
      expect(accepts(target.kind, none)).not.toContain("plan");
      expect(accepts(target.kind, none)).not.toContain("control");
    }
    for (const source of [proc, note, nav, svc]) {
      expect(produces(source.kind, none)).not.toContain("plan");
      expect(produces(source.kind, none)).not.toContain("control");
    }
  });

  test("integração por gatilho não aceita plano nem controla", () => {
    const hook: Integration = {
      id: "h",
      name: "h",
      script: "",
      trigger: { type: "onInput", kind: "control" },
      output: "log",
      file: null,
    };
    const ints = new Map([[hook.id, hook]]);
    expect(accepts({ type: "integration", integrationId: "h", dir: "" }, ints)).toEqual([]);
    const manual = new Map([["h", { ...hook, trigger: { type: "manual" as const } }]]);
    expect(accepts({ type: "integration", integrationId: "h", dir: "" }, manual)).not.toContain(
      "plan",
    );
  });

  test("a etiqueta só troca entre os válidos do par; a do plano não troca", () => {
    const nodes = [agent, proc, note, nav];
    const edges = [
      { id: "c", from: "o", to: "b", kind: "control" as const },
      { id: "u", from: "p", to: "b", kind: "url" as const },
      { id: "pl", from: "o", to: "n", kind: "plan" as const },
    ];
    expect(edgeKindChoices(nodes, edges, none, "c")).toEqual(["control"]);
    expect(edgeKindChoices(nodes, edges, none, "u")).toEqual(["url"]);
    expect(edgeKindChoices(nodes, edges, none, "pl")).toEqual([]);
  });

  test("não levam valor: não seguram o destino nem contam como passo", () => {
    expect(carriesData("plan")).toBe(false);
    expect(carriesData("control")).toBe(false);
    expect(carriesData("url")).toBe(true);
    const run: Run = {
      id: "r",
      graphId: "g",
      startedAt: 0,
      finishedAt: null,
      status: "running",
      nodes: {
        o: {
          status: "running",
          sessionId: null,
          startedAt: 0,
          finishedAt: null,
          exitCode: null,
          outputs: {},
          reason: null,
        },
      },
    };
    expect(edgeHasValue({ from: "o", kind: "control" }, run)).toBe(true);
    expect(edgeHasValue({ from: "o", kind: "context" }, run)).toBe(false);
    const steps = timelineSteps({
      nodes: [agent, nav],
      edges: [{ id: "c", from: "o", to: "b", kind: "control" }],
    });
    expect(steps).toEqual([["o", "b"]]);
  });
});
