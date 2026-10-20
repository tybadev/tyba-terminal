import { describe, expect, test } from "bun:test";

import type { ContainerInfo } from "./ipc";
import {
  compactSpan,
  containerCounts,
  exitCode,
  exitReason,
  exitedAgoMs,
  formatBytes,
  formatCpu,
  groupContainers,
  healthOf,
  loadContainers,
  matchesQuery,
  parseDockerSpan,
  portList,
  shortImage,
  uptimeMs,
} from "./dockerPanel";

describe("exitReason", () => {
  test("OOM ganha do código", () => {
    expect(exitReason({ oom_killed: true, exit_code: 137 }, 137)).toEqual({ kind: "oom" });
  });
  test("sem OOM, o código do inspect; sem inspect, o do ps", () => {
    expect(exitReason({ oom_killed: false, exit_code: 1 }, null)).toEqual({
      kind: "code",
      code: 1,
    });
    expect(exitReason(undefined, 137)).toEqual({ kind: "code", code: 137 });
    expect(exitReason(undefined, 0)).toEqual({ kind: "clean" });
    expect(exitReason(undefined, null)).toBeNull();
  });
});

describe("formatCpu / formatBytes", () => {
  test("cpu", () => {
    expect(formatCpu(4.12, "en")).toBe("4.1%");
    expect(formatCpu(0.34, "pt-BR")).toBe("0,3%");
    expect(formatCpu(23.6, "en")).toBe("24%");
    expect(formatCpu(null)).toBeNull();
  });
  test("memória", () => {
    expect(formatBytes(182 * 1024 * 1024, "en")).toBe("182 MB");
    expect(formatBytes(1.5 * 1024 ** 3, "pt-BR")).toBe("1,5 GB");
    expect(formatBytes(512, "en")).toBe("512 B");
    expect(formatBytes(undefined)).toBeNull();
  });
});

const container = (state: string): ContainerInfo => ({
  id: state,
  name: state,
  image: "node:22",
  state,
  status: "",
  ports: "",
  compose_project: null,
  compose_working_dir: null,
  service: null,
  config_files: null,
  working_dir_status: null,
  compose_file_status: null,
  in_repo: false,
});

const make = (over: Partial<ContainerInfo>): ContainerInfo => ({
  ...container("running"),
  ...over,
});

describe("shortImage", () => {
  test("tira o digest, mantém repo:tag; full para o tooltip", () => {
    const ref = "redis:7-alpine@sha256:ff02b58f971e7d7d156a12";
    expect(shortImage(ref)).toEqual({ short: "redis:7-alpine", full: ref });
    expect(shortImage("ghcr.io/org/api:1.2").short).toBe("ghcr.io/org/api:1.2");
    expect(shortImage("postgres").short).toBe("postgres");
  });
  test("imagem só por id vira os 12 primeiros", () => {
    expect(shortImage("sha256:0123456789abcdef0123").short).toBe("sha256:0123456789ab");
  });
  test("portas viram lista para chips", () => {
    expect(portList("0.0.0.0:3000->3000/tcp, :::3000->3000/tcp, 0.0.0.0:80->80/tcp")).toEqual([
      "3000",
      "80",
    ]);
    expect(portList("6379/tcp")).toEqual([]);
  });
});

describe("durações do docker ps", () => {
  test("trechos em inglês do docker viram ms", () => {
    expect(parseDockerSpan("26 hours")).toBe(26 * 3600_000);
    expect(parseDockerSpan("About an hour")).toBe(3600_000);
    expect(parseDockerSpan("a minute")).toBe(60_000);
    expect(parseDockerSpan("Less than a second")).toBe(0);
    expect(parseDockerSpan("6 weeks")).toBe(42 * 86_400_000);
    expect(parseDockerSpan("xyz")).toBeNull();
  });
  test("Up/Exited", () => {
    expect(uptimeMs("Up 26 hours (healthy)")).toBe(26 * 3600_000);
    expect(uptimeMs("Up 3 minutes")).toBe(180_000);
    expect(uptimeMs("Exited (0) 2 days ago")).toBeNull();
    expect(exitedAgoMs("Exited (255) 6 days ago")).toBe(6 * 86_400_000);
    expect(exitedAgoMs("Created")).toBeNull();
  });
  test("compacto: Up 26 hours → 26 h", () => {
    expect(compactSpan(uptimeMs("Up 26 hours"))).toBe("26 h");
    expect(compactSpan(45_000)).toBe("45 s");
    expect(compactSpan(180_000)).toBe("3 min");
    expect(compactSpan(3 * 86_400_000)).toBe("3 d");
    expect(compactSpan(null)).toBeNull();
  });
  test("saúde", () => {
    expect(healthOf("Up 2 hours (healthy)")).toBe("healthy");
    expect(healthOf("Up 2 hours (unhealthy)")).toBe("unhealthy");
    expect(healthOf("Up 2 seconds (health: starting)")).toBe("starting");
    expect(healthOf("Up 2 hours")).toBeNull();
  });
});

describe("groupContainers", () => {
  const list = [
    make({ id: "1", name: "zeta", compose_project: "outro" }),
    make({ id: "2", name: "api", compose_project: "muriki", in_repo: true }),
    make({ id: "3", name: "db", compose_project: "muriki", in_repo: true }),
    make({
      id: "4",
      name: "worker",
      compose_project: "muriki",
      in_repo: true,
      state: "exited",
      status: "Exited (137) 2 hours ago",
    }),
    make({
      id: "5",
      name: "velho",
      compose_project: "muriki",
      in_repo: true,
      state: "exited",
      status: "Exited (0) 6 weeks ago",
    }),
    make({
      id: "6",
      name: "recente",
      compose_project: "muriki",
      in_repo: true,
      state: "exited",
      status: "Exited (0) 5 minutes ago",
    }),
    make({ id: "7", name: "solto", state: "exited", status: "Exited (1) 1 day ago" }),
    make({ id: "8", name: "alfa", compose_project: "aaa" }),
  ];

  test("repo primeiro, depois alfabético, Avulsos por último", () => {
    expect(groupContainers(list).map((g) => g.project)).toEqual([
      "muriki",
      "aaa",
      "outro",
      null,
    ]);
  });

  test("rodando por nome; parados recentes primeiro; antigos à parte", () => {
    const [repo] = groupContainers(list);
    expect(repo.running.map((c) => c.name)).toEqual(["api", "db"]);
    expect(repo.stopped.map((c) => c.name)).toEqual(["recente", "worker"]);
    expect(repo.old.map((c) => c.name)).toEqual(["velho"]);
  });

  test("busca filtra por nome, serviço, imagem ou projeto", () => {
    expect(groupContainers(list, "solto").map((g) => g.project)).toEqual([null]);
    expect(groupContainers(list, "OUTRO")[0].running[0].name).toBe("zeta");
    expect(groupContainers(list, "nada")).toEqual([]);
    expect(matchesQuery(make({ image: "redis:7" }), "redis")).toBe(true);
  });
});

describe("dockerPanel", () => {
  test("conta rodando e parados", () => {
    expect(
      containerCounts([container("running"), container("running"), container("exited")]),
    ).toEqual({ running: 2, stopped: 1 });
    expect(containerCounts([])).toEqual({ running: 0, stopped: 0 });
  });

  test("código de saída do status do docker ps", () => {
    expect(exitCode("Exited (137) 2 minutes ago")).toBe(137);
    expect(exitCode("Exited (0) 3 hours ago")).toBe(0);
    expect(exitCode("Up 3 minutes")).toBeNull();
    expect(exitCode("Created")).toBeNull();
  });

});

describe("loadContainers", () => {
  test("o ps que responde basta: o version não roda", async () => {
    const steps: string[] = [];
    let versions = 0;
    const out = await loadContainers(
      async () => ["a", "b"],
      async () => {
        versions += 1;
        return true;
      },
      (cmd) => steps.push(cmd),
    );
    expect(out).toEqual({ available: true, list: ["a", "b"] });
    expect(versions).toBe(0);
    expect(steps).toEqual(["ps -a"]);
  });

  test("ps falhou e o version também: sem docker, lista vazia", async () => {
    const steps: string[] = [];
    const out = await loadContainers(
      async () => {
        throw new Error("timeout");
      },
      async () => false,
      (cmd) => steps.push(cmd),
    );
    expect(out).toEqual({ available: false, list: [] });
    expect(steps).toEqual(["ps -a", "version"]);
  });

  test("ps falhou com o docker de pé: o erro do ps sobe", async () => {
    await expect(
      loadContainers(
        async () => {
          throw new Error("permission denied");
        },
        async () => true,
        () => {},
      ),
    ).rejects.toThrow("permission denied");
  });

  test("version que também lança conta como indisponível", async () => {
    const out = await loadContainers(
      async () => {
        throw new Error("x");
      },
      async () => {
        throw new Error("y");
      },
      () => {},
    );
    expect(out).toEqual({ available: false, list: [] });
  });
});
