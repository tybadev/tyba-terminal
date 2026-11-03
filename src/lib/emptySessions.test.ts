import { describe, expect, test } from "bun:test";

import type { Host } from "./ipc";
import { primaryAction, recentHosts } from "./emptySessions";

const host = (id: string, last: string | null): Host => ({
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
});

describe("primaryAction", () => {
  const bindings = { newSession: "meta+n", newTab: "meta+t" };

  test("sem workspace é nova sessão com o atalho de nova sessão", () => {
    expect(primaryAction(false, bindings)).toEqual({
      kind: "newSession",
      combo: "meta+n",
    });
  });

  test("com workspace é nova tab com o atalho de nova tab", () => {
    expect(primaryAction(true, bindings)).toEqual({
      kind: "newTab",
      combo: "meta+t",
    });
  });

  test("segue o binding do usuário", () => {
    expect(
      primaryAction(false, { newSession: "ctrl+shift+n", newTab: "meta+t" })
        .combo,
    ).toBe("ctrl+shift+n");
  });
});

describe("recentHosts", () => {
  test("ordena do mais recente, tira quem nunca conectou e corta em 4", () => {
    const hosts = [
      host("a", "2026-10-01T00:00:00Z"),
      host("never", null),
      host("b", "2026-11-01T00:00:00Z"),
      host("c", "2026-09-01T00:00:00Z"),
      host("d", "2026-10-15T00:00:00Z"),
      host("e", "2026-08-01T00:00:00Z"),
    ];
    expect(recentHosts(hosts).map((h) => h.id)).toEqual(["b", "d", "a", "c"]);
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
