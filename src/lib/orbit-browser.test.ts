import { describe, expect, test } from "bun:test";

import {
  accepts,
  browserFed,
  browserRefusal,
  OPENED_CHIP_MS,
  openedChipLeft,
  connectionProblem,
  edgeDetail,
  edgeHasValue,
  emptySource,
  importItems,
  serviceLiveLabel,
  sourceDetail,
  withLiveContainer,
  isLocalUrl,
  localUrl,
  newNode,
  nodeDir,
  nodeStatusLabel,
  produces,
  serviceContainers,
  urlPort,
  type OrbitEdge,
  type NodeRun,
  type Run,
} from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const none = new Map();
const dev = newNode(
  "p",
  "Dev server",
  { type: "process", command: "bun run dev", output: "log", file: null, dir: "/tmp/web" },
  { x: 0, y: 0 },
);
const nav = newNode("b", "Navegador", { type: "browser", url: "" }, { x: 500, y: 0 });
const api = newNode(
  "s",
  "Serviço · api",
  { type: "service", source: { type: "container", containerId: "abc123def456", name: "api" } },
  { x: 0, y: 400 },
);
const claude = newNode(
  "a",
  "Claude",
  { type: "agent", runner: "claude", role: "", prompt: "investigar", dir: "/tmp/web" },
  { x: 500, y: 400 },
);

describe("isLocalUrl (espelha o core)", () => {
  test("endereços locais passam", () => {
    for (const ok of [
      "http://localhost:5173",
      "http://localhost:5173/agenda?x=1#y",
      "http://localhost",
      "https://localhost:8443/",
      "http://127.0.0.1",
      "http://127.0.0.1:3000",
      "http://[::1]:8080/",
      "HTTP://LOCALHOST:5173",
      "https://code.muriki.test",
      "https://api.muriki.test:8443/health",
      "http://app.localhost:5173/",
    ]) {
      expect(isLocalUrl(ok)).toBe(true);
    }
  });

  test("o resto é recusado", () => {
    for (const bad of [
      "http://192.168.0.10:5173",
      "https://test",
      "https://a..test",
      "https://printer.local",
      "https://evil.test.example.com",
      "http://example.com",
      "https://example.com/localhost",
      "javascript:alert(1)",
      "file:///etc/passwd",
      "data:text/html,oi",
      "localhost:5173",
      "http://localhost.example.com",
      "http://localhost@example.com",
      "http://example.com@localhost",
      "http://localhost\\@example.com",
      "http://0.0.0.0:5173",
      "http://localhost:0",
      "http://localhost:65536",
      "http://localhost:",
      "http://local host",
      "http://localhost\n.example.com",
      "",
    ]) {
      expect(isLocalUrl(bad)).toBe(false);
    }
  });

  test("url de porta e a porta da url", () => {
    expect(localUrl(5173)).toBe("http://localhost:5173");
    expect(isLocalUrl(localUrl(5173))).toBe(true);
    expect(urlPort("http://localhost:5173/agenda")).toBe(5173);
    expect(urlPort("http://[::1]:8080")).toBe(8080);
    expect(urlPort("http://localhost")).toBeNull();
    expect(urlPort(null)).toBeNull();
  });
});

describe("navegador e serviço no grafo", () => {
  test("produces / accepts", () => {
    expect(produces(nav.kind, none)).toEqual([]);
    expect(accepts(nav.kind, none)).toEqual(["url", "control"]);
    expect(produces(api.kind, none)).toEqual(["log", "url"]);
    expect(accepts(api.kind, none)).toEqual([]);
    expect(produces(dev.kind, none)).toEqual(["log", "url"]);
    expect(nodeDir(nav.kind, "/tmp")).toBeNull();
    expect(nodeDir(api.kind, "/tmp")).toBeNull();
  });

  test("ligações oferecidas", () => {
    const nodes = [dev, nav, api, claude];
    expect(connectionProblem(nodes, [], none, "p", "b", "url")).toBeNull();
    expect(connectionProblem(nodes, [], none, "s", "b", "url")).toBeNull();
    expect(connectionProblem(nodes, [], none, "s", "a", "log")).toBeNull();
    expect(connectionProblem(nodes, [], none, "s", "b", "log")).toBe("notAccepted");
    expect(connectionProblem(nodes, [], none, "b", "a", "url")).toBe("notProduced");
    expect(connectionProblem(nodes, [], none, "a", "s", "context")).toBe("notAccepted");
  });

  test("fonte vazia de cada tipo", () => {
    expect(emptySource("port")).toEqual({ type: "port", sessionId: null, port: 0 });
    expect(emptySource("tunnel")).toEqual({
      type: "tunnel",
      sessionId: "",
      tunnelId: "",
      hostId: "",
      listenPort: 0,
    });
    expect(emptySource("container")).toEqual({
      type: "container",
      containerId: "",
      name: "",
      project: "",
      service: "",
    });
  });

  test("contêineres a manter e navegador já ligado", () => {
    const edges: OrbitEdge[] = [{ id: "e", from: "p", to: "b", kind: "url" }];
    expect(serviceContainers([dev, nav, api, api])).toEqual(["abc123def456"]);
    expect(browserFed({ nodes: [dev, nav], edges }, "p")?.id).toBe("b");
    expect(browserFed({ nodes: [dev, nav], edges: [] }, "p")).toBeNull();
  });

  test("a prévia da importação mostra a url e o serviço", () => {
    const items = importItems({
      graph: { id: "g", name: "g", dir: "", nodes: [nav, api], edges: [], variables: {} },
      integrations: [],
    });
    expect(items.map((i) => i.type)).toEqual(["browser", "service"]);
  });
});

describe("url antes do fim do processo", () => {
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
  const edge: OrbitEdge = { id: "e", from: "p", to: "b", kind: "url" };
  const run = (p: NodeRun): Run => ({
    id: "r",
    graphId: "g",
    startedAt: 0,
    finishedAt: null,
    status: "running",
    nodes: { p, b: nodeRun({ status: "waiting", sessionId: null }) },
  });

  test("sem porta, a ligação url espera; com porta, tem valor", () => {
    const waiting = run(nodeRun({}));
    expect(edgeHasValue(edge, waiting)).toBe(false);
    expect(edgeDetail("url", waiting.nodes.p)?.key).toBe("orbitEdgeLive");
    expect(nodeStatusLabel({ nodes: [dev, nav], edges: [edge] }, waiting, "b")?.key).toBe(
      "orbitWaitingFor",
    );

    const ready = run(nodeRun({ outputs: { url: "/out/url.txt" }, url: "http://localhost:5173" }));
    expect(edgeHasValue(edge, ready)).toBe(true);
    expect(edgeHasValue({ ...edge, kind: "log" }, ready)).toBe(false);
    expect(edgeDetail("url", ready.nodes.p)?.key).toBe("orbitEdgeReady");
    expect(edgeDetail("log", ready.nodes.p)?.key).toBe("orbitEdgeLive");
  });
});

describe("navegador aberto por agente", () => {
  test("file:// tem o motivo próprio; o resto, a regra local", () => {
    expect(browserRefusal("file:///Users/x/index.html")).toBe("orbitBrowserFileRefused");
    expect(browserRefusal(" FILE:///tmp/a.html")).toBe("orbitBrowserFileRefused");
    expect(browserRefusal("https://example.com")).toBe("orbitBrowserRefused");
  });

  test("o chip '<agente> abriu' dura alguns segundos", () => {
    const opened = { by: "Claude", at: 10_000 };
    expect(openedChipLeft(opened, 10_000)).toBe(OPENED_CHIP_MS);
    expect(openedChipLeft(opened, 10_000 + OPENED_CHIP_MS - 1)).toBe(1);
    expect(openedChipLeft(opened, 10_000 + OPENED_CHIP_MS)).toBe(0);
    expect(openedChipLeft(null, 0)).toBe(0);
    expect(openedChipLeft(undefined, 0)).toBe(0);
  });
});

describe("serviço vivo", () => {
  const live = (state: "up" | "down" | "stopped", containerId: string | null = null) => ({
    state,
    detail: null,
    canStart: state !== "up",
    containerId,
  });

  test("a cápsula diz no ar / caiu / parado", () => {
    expect(serviceLiveLabel(null)).toBeNull();
    expect(serviceLiveLabel(live("up"))).toEqual({ key: "orbitServiceUp", tone: "green" });
    expect(serviceLiveLabel(live("down"))).toEqual({ key: "orbitServiceDown", tone: "red" });
    expect(serviceLiveLabel(live("stopped"))).toEqual({ key: "orbitServiceStopped", tone: "faint" });
  });

  test("contêiner recriado: o nó passa a guardar o id de agora", () => {
    const source = { type: "container" as const, containerId: "old", name: "api", project: "m", service: "api" };
    expect(withLiveContainer(source, live("up", "new"))).toEqual({ ...source, containerId: "new" });
    expect(withLiveContainer(source, live("up", "old"))).toBeNull();
    expect(withLiveContainer(source, null)).toBeNull();
    expect(withLiveContainer({ type: "port", sessionId: null, port: 1 }, live("up", "x"))).toBeNull();
  });

  test("túnel mostra a porta local", () => {
    expect(sourceDetail({ type: "tunnel", sessionId: "s", tunnelId: "abcdefghij", listenPort: 5173 })).toBe(":5173");
    expect(sourceDetail({ type: "tunnel", sessionId: "s", tunnelId: "abcdefghij" })).toBe("abcdefgh");
  });
});
