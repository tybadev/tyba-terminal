import { describe, expect, test } from "bun:test";

import type { PortEntry, ServiceOrigin } from "./ipc";
import {
  chipText,
  filterPorts,
  forceWaitSeconds,
  originText,
  portTone,
  portUrl,
  refusalKey,
  workspacePorts,
} from "./services";

const t = (key: string) => key;

function entry(port: number, origin: ServiceOrigin): PortEntry {
  return {
    port,
    addresses: ["*"],
    pid: port,
    start_ms: 1,
    comm: "node",
    command: "node x",
    origin,
    cpu_percent: null,
    rss_bytes: null,
    pgid: null,
    refusal: null,
  };
}

describe("filterPorts", () => {
  const ports = [
    entry(3000, { kind: "session", session_id: "s1" }),
    entry(5432, { kind: "docker" }),
    entry(5173, { kind: "outside" }),
  ];

  test("só das minhas sessões deixa só o que descende de um shell do TYBA", () => {
    expect(filterPorts(ports, true).map((p) => p.port)).toEqual([3000]);
  });

  test("desligado mostra tudo", () => {
    expect(filterPorts(ports, false)).toHaveLength(3);
  });
});

describe("originText", () => {
  test("sessão usa o título dela; sumiu, cai no genérico", () => {
    const origin: ServiceOrigin = { kind: "session", session_id: "s1" };
    expect(originText(origin, t, () => "poc-git")).toBe("poc-git");
    expect(originText(origin, t, () => null)).toBe("servicesOriginSession");
  });

  test("cada origem tem o seu texto", () => {
    expect(originText({ kind: "docker" }, t, () => null)).toBe("Docker");
    expect(originText({ kind: "outside" }, t, () => null)).toBe("servicesOriginOutside");
    expect(originText({ kind: "system" }, t, () => null)).toBe("servicesOriginSystem");
    expect(originText({ kind: "other_user" }, t, () => null)).toBe("servicesOriginOtherUser");
    expect(originText({ kind: "tyba" }, t, () => null)).toBe("TYBA");
  });
});

describe("portTone", () => {
  test("verde nas sessões e no Docker, âmbar fora do TYBA, apagado no resto", () => {
    expect(portTone({ kind: "session", session_id: "s" })).toBe("green");
    expect(portTone({ kind: "docker" })).toBe("green");
    expect(portTone({ kind: "outside" })).toBe("amber");
    expect(portTone({ kind: "system" })).toBe("faint");
    expect(portTone({ kind: "other_user" })).toBe("faint");
  });
});

describe("chipText", () => {
  test("até duas portas e o resto em +N", () => {
    expect(chipText([])).toBeNull();
    expect(chipText([3000])).toBe(":3000");
    expect(chipText([3000, 5432])).toBe(":3000 · :5432");
    expect(chipText([3000, 5432, 6379, 8080])).toBe(":3000 · :5432 +2");
  });
});

describe("workspacePorts", () => {
  test("junta as sessões do workspace sem repetir", () => {
    expect(workspacePorts({ a: [5432, 3000], b: [3000, 80], c: [9] }, ["a", "b"])).toEqual([
      80, 3000, 5432,
    ]);
    expect(workspacePorts({}, ["a"])).toEqual([]);
  });
});

describe("forceWaitSeconds", () => {
  test("conta até o SIGKILL ficar disponível", () => {
    expect(forceWaitSeconds(1000, 5000, 1000)).toBe(5);
    expect(forceWaitSeconds(1000, 5000, 4100)).toBe(2);
    expect(forceWaitSeconds(1000, 5000, 6000)).toBe(0);
    expect(forceWaitSeconds(1000, 5000, 9000)).toBe(0);
  });
});

test("url e chaves de recusa", () => {
  expect(portUrl(3000)).toBe("http://localhost:3000");
  expect(refusalKey("session_shell")).toBe("servicesRefuseSessionShell");
  expect(refusalKey("docker")).toBe("servicesRefuseDocker");
});
