import { describe, expect, test } from "bun:test";

import {
  COMPOSE_OUTPUT_CAP,
  composeProjects,
  mergeComposeOutput,
  mergeComposeSnapshot,
  pickProject,
  portMappings,
  projectRun,
  serviceState,
  visibleProjects,
} from "./dockerCompose";
import type { ComposeRun, ContainerInfo } from "./ipc";

// Lista sintética: nomes e caminhos inventados.
const c = (over: Partial<ContainerInfo>): ContainerInfo => ({
  id: "a".repeat(64),
  name: "x",
  image: "img",
  state: "running",
  status: "Up 3 minutes",
  ports: "",
  compose_project: null,
  compose_working_dir: null,
  service: null,
  config_files: null,
  working_dir_status: null,
  compose_file_status: null,
  in_repo: false,
  ...over,
});

const list: ContainerInfo[] = [
  c({ id: "1", name: "loose" }),
  c({
    id: "2",
    name: "obs-grafana-1",
    service: "grafana",
    compose_project: "observability",
    compose_working_dir: "/home/dev/infra/obs",
    state: "exited",
    status: "Exited (0) 2 days ago",
  }),
  c({
    id: "3",
    name: "poc-worker-1",
    service: "worker",
    compose_project: "poc",
    compose_working_dir: "/home/dev/poc",
    config_files: "/home/dev/poc/compose.yml,/home/dev/poc/compose.override.yml",
    working_dir_status: "ok",
    compose_file_status: "ok",
    in_repo: true,
    state: "exited",
    status: "Exited (137) 2 minutes ago",
  }),
  c({
    id: "4",
    name: "poc-api-1",
    service: "api",
    compose_project: "poc",
    compose_working_dir: "/home/dev/poc",
    in_repo: true,
  }),
  c({
    id: "5",
    name: "app-web-1",
    service: "web",
    compose_project: "app",
    compose_working_dir: "/home/dev/app",
  }),
];

describe("composeProjects", () => {
  const projects = composeProjects(list);

  test("agrupa por projeto, avulsos de fora, repo primeiro e depois por nome", () => {
    expect(projects.map((p) => p.name)).toEqual(["poc", "app", "observability"]);
  });

  test("conta rodando, ordena serviços e lê o arquivo compose", () => {
    const poc = projects[0];
    expect(poc.inRepo).toBe(true);
    expect(poc.running).toBe(1);
    expect(poc.services.map((s) => s.service)).toEqual(["api", "worker"]);
    expect(poc.workingDir).toBe("/home/dev/poc");
    expect(poc.composeFile).toBe("compose.yml");
    expect(projects[1].composeFile).toBeNull();
  });

  test("ponto do projeto: todos, alguns, nenhum", () => {
    expect(projects.map(projectRun)).toEqual(["some", "all", "none"]);
  });

  test("sem compose, sem projeto", () => {
    expect(composeProjects([c({})])).toEqual([]);
  });
});

describe("visibleProjects e pickProject", () => {
  const projects = composeProjects(list);

  test("ocultar parados tira quem não tem nada rodando", () => {
    expect(visibleProjects(projects, true).map((p) => p.name)).toEqual(["poc", "app"]);
    expect(visibleProjects(projects, false)).toHaveLength(3);
  });

  test("escolhido fica; sumido cai no do repo, depois no primeiro", () => {
    expect(pickProject(projects, "app")).toBe("app");
    expect(pickProject(projects, "gone")).toBe("poc");
    expect(pickProject(projects, null)).toBe("poc");
    const others = projects.filter((p) => !p.inRepo);
    expect(pickProject(others, null)).toBe("app");
    expect(pickProject([], "app")).toBeNull();
  });
});

test("portMappings: host → container, sem repetir IPv4/IPv6", () => {
  expect(portMappings("0.0.0.0:3000->3000/tcp, :::3000->3000/tcp")).toEqual([":3000 → 3000"]);
  expect(portMappings("0.0.0.0:8080->80/tcp, 5432/tcp")).toEqual([":8080 → 80"]);
  expect(portMappings("")).toEqual([]);
});

describe("serviceState", () => {
  test("rodando, saudável, doente", () => {
    expect(serviceState({ state: "running", status: "Up 3 minutes" })).toEqual({
      tone: "green",
      key: "running",
      span: 3 * 60_000,
    });
    expect(serviceState({ state: "running", status: "Up 20 hours (healthy)" }).key).toBe(
      "healthy",
    );
    expect(serviceState({ state: "running", status: "Up 1 minute (unhealthy)" }).tone).toBe(
      "red",
    );
  });

  test("saída com erro é vermelha; saída limpa, apagada", () => {
    expect(serviceState({ state: "exited", status: "Exited (137) 2 minutes ago" })).toEqual({
      tone: "red",
      key: "exited",
      code: 137,
      span: 2 * 60_000,
    });
    expect(serviceState({ state: "exited", status: "Exited (0) 2 days ago" }).tone).toBe("faint");
    expect(serviceState({ state: "created", status: "Created" })).toEqual({
      tone: "faint",
      key: "exited",
      code: null,
      span: null,
    });
  });
});

describe("mergeComposeOutput", () => {
  const run = (output: string[], total = output.length, id = 1): ComposeRun => ({
    run: id,
    working_dir: "/home/dev/poc",
    op: "up",
    last_op: "up",
    error: null,
    output,
    total,
  });

  test("lote na posição certa entra; repetido não duplica", () => {
    const runs = { poc: run(["a", "b"]) };
    const next = mergeComposeOutput(runs, { project: "poc", run: 1, op: "up", start: 2, lines: ["c"] });
    expect(next.poc.output).toEqual(["a", "b", "c"]);
    expect(next.poc.total).toBe(3);
    // O retrato já tinha "b" e "c": só "d" é novo.
    const again = mergeComposeOutput(next, {
      project: "poc",
      run: 1,
      op: "up",
      start: 1,
      lines: ["b", "c", "d"],
    });
    expect(again.poc.output).toEqual(["a", "b", "c", "d"]);
    expect(
      mergeComposeOutput(again, { project: "poc", run: 1, op: "up", start: 0, lines: ["a"] }),
    ).toBe(again);
  });

  test("buraco ou projeto desconhecido esperam o retrato", () => {
    const runs = { poc: run(["a"]) };
    expect(mergeComposeOutput(runs, { project: "poc", run: 1, op: "up", start: 5, lines: ["x"] })).toBe(
      runs,
    );
    expect(
      mergeComposeOutput(runs, { project: "outro", run: 1, op: "up", start: 3, lines: ["x"] }),
    ).toBe(runs);
    // Lote de execução antiga não entra.
    expect(
      mergeComposeOutput({ poc: run(["a"], 1, 2) }, { project: "poc", run: 1, op: "up", start: 1, lines: ["x"] }).poc
        .output,
    ).toEqual(["a"]);
  });

  test("guarda só as últimas linhas", () => {
    const full = Array.from({ length: COMPOSE_OUTPUT_CAP }, (_, i) => String(i));
    const next = mergeComposeOutput({ poc: run(full) }, {
      project: "poc",
      run: 1,
      op: "up",
      start: COMPOSE_OUTPUT_CAP,
      lines: ["fim"],
    });
    expect(next.poc.output).toHaveLength(COMPOSE_OUTPUT_CAP);
    expect(next.poc.output.at(-1)).toBe("fim");
    expect(next.poc.output[0]).toBe("1");
  });
});

describe("execução nova e retrato atrasado", () => {
  const run = (id: number, output: string[], op: ComposeRun["op"] = null): ComposeRun => ({
    run: id,
    working_dir: "/home/dev/poc",
    op,
    last_op: "up",
    error: null,
    output,
    total: output.length,
  });

  test("execução nova não se mistura à anterior", () => {
    const old = { poc: run(1, ["velha 1", "velha 2", "velha 3"]) };
    const next = mergeComposeOutput(old, {
      project: "poc",
      run: 2,
      op: "restart",
      start: 0,
      lines: ["nova 1"],
    });
    expect(next.poc.output).toEqual(["nova 1"]);
    expect(next.poc.op).toBe("restart");
    expect(next.poc.run).toBe(2);
    // Primeira execução do projeto também aparece já no primeiro lote.
    expect(
      mergeComposeOutput({}, { project: "api", run: 3, op: "up", start: 0, lines: ["x"] }).api.output,
    ).toEqual(["x"]);
  });

  test("retrato lido antes de eventos não apaga linhas nem volta execução", () => {
    const tela = { poc: run(2, ["a", "b", "c"], "up") };
    const atrasado = { poc: run(2, ["a"], "up") };
    expect(mergeComposeSnapshot(tela, atrasado).poc.output).toEqual(["a", "b", "c"]);
    const antigo = { poc: run(1, ["velha"]) };
    expect(mergeComposeSnapshot(tela, antigo).poc.run).toBe(2);
    // Retrato à frente manda.
    const novo = { poc: { ...run(2, ["a", "b", "c", "d"]), error: "falhou" } };
    expect(mergeComposeSnapshot(tela, novo).poc).toEqual(novo.poc);
  });
});

test("projeto sem container mas com compose registrado continua na coluna", () => {
  const runs = {
    sumido: {
      run: 1,
      working_dir: "/home/dev/sumido",
      op: null,
      last_op: "down" as const,
      error: null,
      output: ["Removed"],
      total: 1,
    },
  };
  const projects = composeProjects(list, runs);
  const ghost = projects.find((p) => p.name === "sumido");
  expect(ghost?.services).toEqual([]);
  expect(ghost?.workingDir).toBe("/home/dev/sumido");
  expect(visibleProjects(projects, true).map((p) => p.name)).toContain("sumido");
});
