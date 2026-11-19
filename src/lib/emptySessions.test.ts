import { describe, expect, test } from "bun:test";

import type { Host } from "./ipc";
import {
  INITIAL_CHOICE,
  agentsAllowed,
  effectiveWhat,
  hostAddress,
  launchPlan,
  launchSummary,
  pickFolder,
  pickHost,
  recentHosts,
  type LaunchChoice,
} from "./emptySessions";

const host = (id: string, last: string | null, extra: Partial<Host> = {}): Host => ({
  id,
  alias: id,
  hostname: `${id}.example.test`,
  port: null,
  username: null,
  identity_file: null,
  proxy_jump: null,
  group_id: null,
  color: null,
  notes: null,
  position: 0,
  tunnels: [],
  created_at: "2026-07-16T00:00:00Z",
  last_connected_at: last,
  ...extra,
});

const FOLDER = "/Users/fulano/projetos/loja";
const choice = (c: Partial<LaunchChoice>): LaunchChoice => ({ ...INITIAL_CHOICE, ...c });

describe("recentHosts", () => {
  test("ordena do mais recente, tira quem nunca conectou e corta em 3", () => {
    const hosts = [
      host("a", "2026-10-01T00:00:00Z"),
      host("never", null),
      host("b", "2026-11-01T00:00:00Z"),
      host("c", "2026-09-01T00:00:00Z"),
      host("d", "2026-10-15T00:00:00Z"),
    ];
    expect(recentHosts(hosts).map((h) => h.id)).toEqual(["b", "d", "a"]);
  });

  test("não muda a lista recebida", () => {
    const hosts = [host("a", "2026-01-01T00:00:00Z"), host("b", "2026-02-01T00:00:00Z")];
    recentHosts(hosts);
    expect(hosts.map((h) => h.id)).toEqual(["a", "b"]);
  });

  test("sem host usado, vazio", () => {
    expect(recentHosts([host("x", null)])).toEqual([]);
  });
});

describe("hostAddress", () => {
  test("só com o que o host tem", () => {
    expect(hostAddress(host("s", null))).toBe("s.example.test");
    expect(hostAddress(host("s", null, { username: "deploy", port: 2222 }))).toBe(
      "deploy@s.example.test:2222",
    );
  });
});

describe("resumo dinâmico", () => {
  const hosts = [host("staging", null, { username: "deploy" })];

  test("começa em shell neste computador", () => {
    expect(launchSummary(INITIAL_CHOICE, hosts)).toEqual({ what: "shell", place: "~" });
  });

  test("agente na pasta, com a home em ~", () => {
    expect(launchSummary(choice({ where: "folder", what: "claude", folder: FOLDER }), hosts)).toEqual({
      what: "claude",
      place: "~/projetos/loja",
    });
  });

  test("host mostra o endereço e o shell, mesmo com agente escolhido antes", () => {
    expect(launchSummary(choice({ where: "host", what: "codex", hostId: "staging" }), hosts)).toEqual({
      what: "shell",
      place: "deploy@staging.example.test",
    });
  });

  test("sem alvo, sem lugar", () => {
    expect(launchSummary(choice({ where: "folder" }), hosts).place).toBeNull();
    expect(launchSummary(choice({ where: "host", hostId: "sumiu" }), hosts).place).toBeNull();
  });
});

describe("agente desligado com host", () => {
  test("host SSH desliga agente e força shell", () => {
    const c = choice({ where: "host", what: "claude", hostId: "staging" });
    expect(agentsAllowed(c)).toBe(false);
    expect(effectiveWhat(c)).toBe("shell");
  });

  test("voltar para a máquina devolve o agente escolhido", () => {
    const c = choice({ where: "local", what: "claude", hostId: "staging" });
    expect(agentsAllowed(c)).toBe(true);
    expect(effectiveWhat(c)).toBe("claude");
  });

  test("o plano de host é ssh, nunca agente", () => {
    const h = host("staging", null);
    expect(launchPlan(choice({ where: "host", what: "codex", hostId: "staging" }), [h])).toEqual({
      kind: "ssh",
      host: h,
    });
  });
});

describe("launchPlan", () => {
  test("cada combinação na máquina", () => {
    expect(launchPlan(INITIAL_CHOICE, [])).toEqual({ kind: "shell", cwd: null });
    expect(launchPlan(choice({ what: "claude" }), [])).toEqual({
      kind: "agent",
      runner: "claude_code",
      cwd: null,
    });
    expect(launchPlan(choice({ where: "folder", what: "codex", folder: FOLDER }), [])).toEqual({
      kind: "agent",
      runner: "codex",
      cwd: FOLDER,
    });
    expect(launchPlan(choice({ where: "folder", folder: FOLDER }), [])).toEqual({
      kind: "shell",
      cwd: FOLDER,
    });
  });

  test("sem pasta ou sem host não abre", () => {
    expect(launchPlan(choice({ where: "folder" }), [])).toBeNull();
    expect(launchPlan(choice({ where: "host", hostId: "sumiu" }), [host("x", null)])).toBeNull();
  });
});

describe("clicar em recente escolhe o Onde", () => {
  test("pasta recente escolhe a pasta e mantém o Com quê", () => {
    const c = pickFolder(choice({ what: "codex" }), FOLDER);
    expect(c).toEqual({ where: "folder", what: "codex", folder: FOLDER, hostId: null });
  });

  test("host recente escolhe o host", () => {
    const c = pickHost(choice({ where: "folder", folder: FOLDER }), "staging");
    expect(c.where).toBe("host");
    expect(c.hostId).toBe("staging");
    // A pasta fica guardada para quando voltar a Pasta…
    expect(c.folder).toBe(FOLDER);
  });

  test("não muda a escolha recebida", () => {
    const before = choice({});
    pickHost(before, "staging");
    expect(before).toEqual(INITIAL_CHOICE);
  });
});
