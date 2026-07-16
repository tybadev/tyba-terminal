import { describe, expect, test } from "bun:test";

import type { Host } from "./ipc";
import { matchSshHost, parseSshCommand } from "./sshCommand";

const host = (over: Partial<Host>): Host => ({
  id: "id",
  alias: "web-01",
  hostname: "web-01.example.com",
  port: null,
  username: null,
  identity_file: null,
  proxy_jump: null,
  group_id: null,
  color: null,
  notes: null,
  position: 0,
  created_at: "2026-07-16T00:00:00Z",
  last_connected_at: null,
  ...over,
});

describe("parseSshCommand", () => {
  test("user@host", () => {
    expect(parseSshCommand("ssh root@2.25.198.136")).toEqual({
      user: "root",
      target: "2.25.198.136",
      port: null,
    });
  });

  test("alias sem user", () => {
    expect(parseSshCommand("ssh Hostinger-vps")).toEqual({
      user: null,
      target: "Hostinger-vps",
      port: null,
    });
  });

  test("porta por -p", () => {
    expect(parseSshCommand("ssh -p 2222 deploy@host")).toEqual({
      user: "deploy",
      target: "host",
      port: 2222,
    });
  });

  test("user por -l", () => {
    expect(parseSshCommand("ssh -l deploy host")).toEqual({
      user: "deploy",
      target: "host",
      port: null,
    });
  });

  test("flags com valor não viram alvo", () => {
    expect(parseSshCommand("ssh -i ~/.ssh/key -o StrictHostKeyChecking=no h")).toEqual(
      { user: null, target: "h", port: null },
    );
  });

  test("comando remoto não é sessão interativa", () => {
    expect(parseSshCommand("ssh host uptime")).toBeNull();
  });

  test("não é ssh", () => {
    expect(parseSshCommand("scp a host:/b")).toBeNull();
    expect(parseSshCommand("sshfs x")).toBeNull();
    expect(parseSshCommand("ssh")).toBeNull();
  });

  test("ipv6 e user com ponto", () => {
    expect(parseSshCommand("ssh john.doe@srv")).toEqual({
      user: "john.doe",
      target: "srv",
      port: null,
    });
  });
});

describe("matchSshHost", () => {
  const hosts = [
    host({ id: "a", alias: "Hostinger-vps", hostname: "2.25.198.136", username: "root" }),
    host({ id: "b", alias: "web-01", hostname: "web-01.example.com" }),
  ];

  test("casa por alias", () => {
    expect(matchSshHost("ssh Hostinger-vps", hosts)?.id).toBe("a");
  });

  test("casa por hostname + user", () => {
    expect(matchSshHost("ssh root@2.25.198.136", hosts)?.id).toBe("a");
  });

  test("user diferente não casa", () => {
    expect(matchSshHost("ssh outro@2.25.198.136", hosts)).toBeNull();
  });

  test("host desconhecido não casa", () => {
    expect(matchSshHost("ssh 10.0.0.1", hosts)).toBeNull();
  });

  test("comando nulo ou não-ssh", () => {
    expect(matchSshHost(null, hosts)).toBeNull();
    expect(matchSshHost("vim", hosts)).toBeNull();
  });
});
