import { describe, expect, test } from "bun:test";

import type { ApprovalRequest } from "./ipc";
import { newNode, type NodeRun, type Run } from "./orbit";
import {
  agentDetailRows,
  agentName,
  approvalLine,
  closesFocus,
  diffSummary,
  failureFooter,
  headerActions,
  sessionLog,
  showsEndLog,
  nodeApproval,
  ringState,
  visibleCardApprovalIds,
  waitedFor,
  waitingQueue,
  waitingYouOf,
} from "./orbitCard";

// Fixtures sintéticas: nada aqui veio de sessão real.
const nodeRun = (patch: Partial<NodeRun>): NodeRun => ({
  status: "running",
  sessionId: "s1",
  startedAt: 1,
  finishedAt: null,
  exitCode: null,
  outputs: {},
  reason: null,
  ...patch,
});

const request = (id: number, session: string, at: number): ApprovalRequest => ({
  id,
  session_id: session,
  command: "rm -rf dist",
  cwd: null,
  risk: "red",
  risk_reason: null,
  context: null,
  requested_at_ms: at,
});

describe("ringState", () => {
  test("cada estado do nó vira um anel", () => {
    expect(ringState(undefined, false)).toBe("idle");
    expect(ringState(nodeRun({ status: "running" }), false)).toBe("working");
    expect(ringState(nodeRun({ status: "running", url: "http://localhost:3000" }), false)).toBe(
      "live",
    );
    expect(ringState(nodeRun({ status: "waiting" }), false)).toBe("waiting");
    expect(ringState(nodeRun({ status: "failed" }), false)).toBe("failed");
    expect(ringState(nodeRun({ status: "done" }), false)).toBe("done");
    expect(ringState(nodeRun({ status: "stopped" }), false)).toBe("idle");
    expect(ringState(nodeRun({ status: "skipped" }), false)).toBe("idle");
  });

  test("precisar do dono vence o resto", () => {
    expect(ringState(nodeRun({ status: "running" }), true)).toBe("owner");
    expect(ringState(undefined, true)).toBe("owner");
  });

  test("agente parado numa execução assentada fica pronto, não gira", () => {
    const live = nodeRun({ status: "running", orchestrating: true });
    expect(ringState(live, false)).toBe("working");
    expect(ringState(live, false, true)).toBe("done");
    // No primeiro turno (ainda não segura o run) e com pedido do dono, não.
    expect(ringState(nodeRun({ status: "running" }), false, true)).toBe("working");
    expect(ringState(live, true, true)).toBe("owner");
  });
});

describe("nodeApproval", () => {
  const list = [
    request(7, "s2", 50),
    request(5, "s1", 90),
    request(3, "s1", 40),
    request(4, "s1", 40),
  ];

  test("o mais antigo da sessão do nó, desempate pelo id", () => {
    expect(nodeApproval("s1", list)?.id).toBe(3);
    expect(nodeApproval("s2", list)?.id).toBe(7);
  });

  test("sem sessão ou sem pedido, nenhum", () => {
    expect(nodeApproval(null, list)).toBeNull();
    expect(nodeApproval("s9", list)).toBeNull();
  });
});

describe("visibleCardApprovalIds: o toast só sai com a cápsula à vista", () => {
  const approvals = new Map([
    ["a", request(1, "s1", 1)],
    ["b", request(2, "s2", 1)],
  ]);
  const at = (x: number, y: number) => ({ x, y, w: 400, h: 300 });
  const pane = { w: 1000, h: 700 };
  const view = { x: 0, y: 0, zoom: 1 };
  const ids = (p: Partial<Parameters<typeof visibleCardApprovalIds>[0]>) =>
    [
      ...visibleCardApprovalIds({
        cards: [
          { id: "a", rect: at(20, 20) },
          { id: "b", rect: at(1200, 20) },
        ],
        approvals,
        view,
        pane,
        foreground: true,
        ...p,
      }),
    ].sort();

  test("cápsula na tela esconde o toast; a do nó fora da tela não", () => {
    expect(ids({})).toEqual([1]);
  });

  test("o pan traz o nó para a tela", () => {
    expect(ids({ view: { x: -900, y: 0, zoom: 1 } })).toEqual([2]);
  });

  test("cápsula cortada na borda: o toast fica", () => {
    expect(ids({ view: { x: 0, y: -30, zoom: 1 } })).toEqual([]);
  });

  test("tela de Serviços por cima (canvas fora do primeiro plano): nenhuma", () => {
    expect(ids({ foreground: false })).toEqual([]);
  });

  test("zoom longe demais para ler a cápsula: nenhuma", () => {
    expect(ids({ view: { x: 0, y: 0, zoom: 0.3 } })).toEqual([]);
    // A 0,5 o nó de x 1200 também cabe na tela.
    expect(ids({ view: { x: 0, y: 0, zoom: 0.5 } })).toEqual([1, 2]);
  });

  test("área ainda sem medida: nenhuma", () => {
    expect(ids({ pane: { w: 0, h: 0 } })).toEqual([]);
  });
});

describe("cards no quadro OrbitCardsD", () => {
  const agent = (runner: "claude" | "codex", role: string) =>
    ({ type: "agent", runner, role, prompt: "", dir: "/p" }) as const;
  const process = { type: "process", command: "bun test", output: "log", file: null, dir: "/p" } as const;

  test("grade do ⋯ do agente: agente | pasta / papel | diff / contexto | tempo", () => {
    const rows = agentDetailRows({
      runner: "claude",
      role: "desenvolvedor",
      dir: "muriki-web",
      task: "escrever os testes",
      telemetry: { model: "Opus 5.5", contextPct: 12.6, linesAdded: 42, linesRemoved: 7 },
    });
    expect(rows.slice(0, 6)).toEqual([
      { key: "orbitDetailAgent", value: "Claude · Opus 5.5" },
      { key: "orbitDetailDir", value: "muriki-web" },
      { key: "orbitRole", value: "desenvolvedor" },
      { key: "orbitDetailDiff", value: "+42 −7" },
      { key: "orbitDetailContext", value: "13%" },
      { key: "orbitDetailTime", value: null },
    ]);
  });

  test("sem o que o core reporta, o agente sai sozinho e diff/contexto vazios", () => {
    const rows = agentDetailRows({
      runner: "codex",
      role: "",
      dir: "",
      task: "",
      telemetry: undefined,
    });
    expect(rows.map((r) => r.value).slice(0, 5)).toEqual(["Codex", "", "", "", ""]);
  });

  test("nome do modelo: o id do Claude perde o prefixo", () => {
    expect(agentName("claude", "claude-opus-5-5")).toBe("Claude · opus-5-5");
    expect(agentName("claude", null)).toBe("Claude");
    expect(diffSummary({ linesAdded: 3, linesRemoved: null })).toBe("+3 −0");
  });

  test("esperando você: 'Codex · revisor · pede há 40s'", () => {
    expect(approvalLine(agent("codex", "revisor"), 1_000, 41_000)).toEqual({
      who: "Codex · revisor",
      ago: "40s",
    });
    expect(approvalLine(agent("claude", " "), 0, 0).who).toBe("Claude");
  });

  test("processo que falhou: código de saída e 'Ver log inteiro'", () => {
    const failed = nodeRun({ status: "failed", exitCode: 1 });
    expect(failureFooter(process, failed)).toEqual({ exitCode: 1, reason: "", log: true });
    expect(failureFooter(agent("claude", ""), failed)?.log).toBe(false);
    expect(failureFooter(process, nodeRun({ status: "done" }))).toBeNull();
  });
});

describe("showsEndLog", () => {
  const process = {
    type: "process",
    command: "python3 -m http.server 8765",
    output: "log",
    file: null,
    dir: "",
  } as const;
  test("processo que rodou e acabou mostra o log; o resto não", () => {
    expect(showsEndLog(process, nodeRun({ status: "failed", exitCode: 1 }))).toBe(true);
    expect(showsEndLog(process, nodeRun({ status: "done", exitCode: 0 }))).toBe(true);
    expect(showsEndLog(process, nodeRun({ status: "stopped" }))).toBe(true);
    expect(showsEndLog(process, nodeRun({ status: "failed", sessionId: null }))).toBe(false);
    expect(showsEndLog(process, nodeRun({ status: "running" }))).toBe(false);
    expect(showsEndLog(process, nodeRun({ status: "skipped", sessionId: null }))).toBe(false);
    expect(showsEndLog(process, undefined)).toBe(false);
  });
});

describe("headerActions", () => {
  const none = { stop: false, restart: false, rerun: false };
  test("processo rodando: abre, reinicia e para", () => {
    const a = headerActions("process", {
      session: true,
      controls: { stop: true, restart: true, rerun: false },
      terminalOpen: false,
    });
    expect(a.open).toEqual({ enabled: true, hint: "orbitOpenHint" });
    expect(a.again).toEqual({ enabled: true, rerun: false, hint: "orbitRestart" });
    expect(a.stop).toEqual({ enabled: true, hint: "orbitStopNode" });
  });
  test("processo que terminou: rodar de novo, abrir e parar apagados com o motivo", () => {
    const a = headerActions("process", {
      session: false,
      controls: { stop: false, restart: false, rerun: true },
      terminalOpen: false,
    });
    expect(a.open).toEqual({ enabled: false, hint: "orbitOpenNoSession" });
    expect(a.again).toEqual({ enabled: true, rerun: true, hint: "orbitRerun" });
    expect(a.stop).toEqual({ enabled: false, hint: "orbitStopNothing" });
  });
  test("execução fechada: reiniciar apagado pede o ▶", () => {
    const a = headerActions("agent", { session: false, controls: none, terminalOpen: false });
    expect(a.again).toEqual({ enabled: false, rerun: false, hint: "orbitRestartNeedsRun" });
  });
  test("terminal: abre e o ■ fecha o shell; não reinicia", () => {
    const open = headerActions("terminal", { session: true, controls: none, terminalOpen: true });
    expect(open.open?.enabled).toBe(true);
    expect(open.again).toBeNull();
    expect(open.stop).toEqual({ enabled: true, hint: "orbitTerminalCloseHint" });
    const closed = headerActions("terminal", { session: false, controls: none, terminalOpen: false });
    expect(closed.stop).toEqual({ enabled: false, hint: "orbitTerminalClosed" });
  });
  test("nota não tem ações; navegador só recarrega", () => {
    const note = headerActions("note", { session: false, controls: none, terminalOpen: false });
    expect(note).toEqual({ open: null, again: null, stop: null });
    const browser = headerActions("browser", {
      session: false,
      controls: { stop: false, restart: true, rerun: false },
      terminalOpen: false,
    });
    expect(browser.open).toBeNull();
    expect(browser.stop).toBeNull();
    expect(browser.again?.enabled).toBe(true);
  });
});

describe("sessionLog", () => {
  test("processo: criado pelo agente, subiu com o comando, porta, você reiniciou, saiu", () => {
    const lines = sessionLog(
      [
        { at: 1, what: { type: "created", by: "claude" } },
        { at: 2, what: { type: "started", command: "bun --hot server.ts" } },
        { at: 3, what: { type: "url", url: "http://localhost:3000/" } },
        { at: 4, what: { type: "restarted", by: "you", rerun: false } },
        { at: 5, what: { type: "exited", ok: false, code: 3 } },
        { at: 6, what: { type: "stopped", by: { agent: "claude" } } },
      ],
      "process",
    );
    expect(lines.map((l) => [l.tone, l.key, l.params, l.you])).toEqual([
      ["violet", "orbitLog_created", { by: "claude" }, false],
      ["green", "orbitLog_startedCommand", { command: "bun --hot server.ts" }, false],
      ["green", "orbitLog_port", { port: 3000 }, false],
      ["blue", "orbitLog_restartedYou", {}, true],
      ["red", "orbitLog_failedCode", { code: 3 }, false],
      ["faint", "orbitLog_stoppedAgent", { agent: "claude" }, false],
    ]);
    expect(lines.map((l) => l.at)).toEqual([1, 2, 3, 4, 5, 6]);
  });
  test("terminal: aberto, claude, criou, o shell saiu, fechado por você", () => {
    const lines = sessionLog(
      [
        { at: 1, what: { type: "started", command: null } },
        { at: 2, what: { type: "agentStarted", agent: "claude" } },
        { at: 3, what: { type: "createdNode", title: "api" } },
        { at: 4, what: { type: "exited", ok: true, code: null } },
        { at: 5, what: { type: "closed", by: "you" } },
      ],
      "terminal",
    );
    expect(lines.map((l) => l.key)).toEqual([
      "orbitLog_opened",
      "orbitLog_agentStarted",
      "orbitLog_createdNode",
      "orbitLog_shellExited",
      "orbitLog_closedYou",
    ]);
  });
  test("criado apagado pelo dono: você apagou", () => {
    const lines = sessionLog(
      [
        { at: 1, what: { type: "created", by: "o" } },
        { at: 2, what: { type: "discarded", by: "you" } },
      ],
      "process",
    );
    expect(lines.map((l) => [l.key, l.you])).toEqual([
      ["orbitLog_created", false],
      ["orbitLog_discardedYou", true],
    ]);
  });
  test("sem eventos (run gravado antes do log): vazio", () => {
    expect(sessionLog(undefined, "agent")).toEqual([]);
  });
  test("rodou de novo pelo agente; terminou sem código; url sem porta", () => {
    const lines = sessionLog(
      [
        { at: 1, what: { type: "restarted", by: { agent: "planner" }, rerun: true } },
        { at: 2, what: { type: "exited", ok: true, code: null } },
        { at: 3, what: { type: "url", url: "file:///x" } },
        { at: 4, what: { type: "started", command: null } },
      ],
      "agent",
    );
    expect(lines.map((l) => [l.key, l.params])).toEqual([
      ["orbitLog_rerunAgent", { agent: "planner" }],
      ["orbitLog_done", {}],
      ["orbitLog_url", { url: "file:///x" }],
      ["orbitLog_started", {}],
    ]);
  });
});

describe("closesFocus", () => {
  test("apagar fecha a vista aberta de qualquer nó", () => {
    expect(closesFocus("delete", "agent")).toBe(true);
    expect(closesFocus("delete", "terminal")).toBe(true);
  });
  test("parar fecha só no Terminal (o shell acabou)", () => {
    expect(closesFocus("stop", "terminal")).toBe(true);
    expect(closesFocus("stop", "process")).toBe(false);
    expect(closesFocus("stop", "agent")).toBe(false);
  });
});

describe("esperando você (quadro OrbitEsperando)", () => {
  const permission = (since: number, hint: string | null = null) => ({
    reason: "permission" as const,
    since,
    hint,
  });

  test("o nó com o pedido aberto espera (também fechado com a sessão viva); anel âmbar, abaixo do gate", () => {
    const waiting = nodeRun({ waitingYou: permission(10, "bun test src/api") });
    expect(waitingYouOf(waiting)).toEqual(permission(10, "bun test src/api"));
    expect(ringState(waiting, false)).toBe("you");
    // Permitir/Negar do gate vem antes (cápsula inteira âmbar).
    expect(ringState(waiting, true)).toBe("owner");
    // Agente que só terminou o turno, processo que caiu, retrato velho: não.
    expect(waitingYouOf(nodeRun({}))).toBeNull();
    expect(waitingYouOf(nodeRun({ status: "failed" }))).toBeNull();
    expect(waitingYouOf(nodeRun({ status: "stopped", waitingYou: permission(1) }))).toBeNull();
    expect(waitingYouOf(nodeRun({ status: "skipped", waitingYou: permission(1) }))).toBeNull();
    expect(waitingYouOf(nodeRun({ status: "waiting", waitingYou: permission(1) }))).toBeNull();
    expect(waitingYouOf(undefined)).toBeNull();
    // Nó já fechado com a sessão viva pedindo: espera (o core só põe nesse caso).
    expect(waitingYouOf(nodeRun({ status: "done", waitingYou: permission(1) }))).toEqual(permission(1));
    expect(ringState(nodeRun({ status: "failed", waitingYou: permission(1) }), false)).toBe("you");
  });

  test("tempo: cabeçalho \"40 s\" / \"2 min\", linha da execução \"40s\" / \"2m\"", () => {
    expect(waitedFor(40_400)).toBe("40 s");
    expect(waitedFor(125_000)).toBe("2 min");
    expect(waitedFor(3 * 3_600_000 + 5)).toBe("3 h");
    expect(waitedFor(40_400, true)).toBe("40s");
    expect(waitedFor(125_000, true)).toBe("2m");
    expect(waitedFor(-5, true)).toBe("0s");
  });

  test("a fila segue a ordem do core e só mostra quem ainda espera no run aberto", () => {
    const agent = (id: string, title: string) =>
      newNode(id, title, { type: "agent", runner: "claude", role: "", prompt: "", dir: "" }, { x: 0, y: 0 });
    const nodes = [agent("rev", "rev · revisor"), agent("front", "front · codex")];
    const run: Run = {
      id: "r",
      graphId: "g",
      startedAt: 0,
      finishedAt: null,
      status: "running",
      nodes: {
        rev: nodeRun({ waitingYou: permission(1, "bun test src/api") }),
        front: nodeRun({ waitingYou: permission(5, " ") }),
        // Saiu da fila no retrato de agora (voltou a trabalhar).
        gone: nodeRun({}),
      },
      waitingYou: ["rev", "front", "gone"],
    };
    expect(waitingQueue(run, nodes)).toEqual([
      { id: "rev", title: "rev · revisor", reason: "permission", hint: "bun test src/api", since: 1 },
      { id: "front", title: "front · codex", reason: "permission", hint: null, since: 5 },
    ]);
    // Execução fechada (histórico) ou sem fila: nada.
    expect(waitingQueue({ ...run, status: "stopped" }, nodes)).toEqual([]);
    expect(waitingQueue({ ...run, waitingYou: undefined }, nodes)).toEqual([]);
    expect(waitingQueue(null, nodes)).toEqual([]);
  });
});
