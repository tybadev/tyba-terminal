import { describe, expect, test } from "bun:test";

import type { Tunnel } from "@/lib/ipc";
import { addedRiskyTunnels, describeTunnel, tunnelFlag } from "@/lib/tunnels";

const local = (port: number): Tunnel => ({
  kind: "local",
  listen_port: port,
  listen_host: null,
  target_host: "localhost",
  target_port: port,
});

const remote = (port: number): Tunnel => ({
  kind: "remote",
  listen_port: port,
  listen_host: null,
  target_host: "localhost",
  target_port: 3000,
});

const dynamic = (port: number): Tunnel => ({
  kind: "dynamic",
  listen_port: port,
  listen_host: null,
  target_host: null,
  target_port: null,
});

describe("describeTunnel", () => {
  test("bind implícito vira 127.0.0.1 na descrição, igual ao writer", () => {
    expect(describeTunnel(local(5432))).toBe("127.0.0.1:5432 → localhost:5432");
  });

  test("dynamic não tem alvo, vira SOCKS", () => {
    expect(describeTunnel(dynamic(1080))).toBe("127.0.0.1:1080 → SOCKS");
  });

  test("bind explícito do dono aparece", () => {
    expect(
      describeTunnel({ ...local(5432), listen_host: "0.0.0.0" }),
    ).toBe("0.0.0.0:5432 → localhost:5432");
  });
});

describe("tunnelFlag", () => {
  test("mapeia os três kinds para a flag do ssh", () => {
    expect(tunnelFlag(local(1))).toBe("-L");
    expect(tunnelFlag(remote(1))).toBe("-R");
    expect(tunnelFlag(dynamic(1))).toBe("-D");
  });
});

describe("addedRiskyTunnels", () => {
  test("-R novo é listado para o diálogo de consentimento", () => {
    expect(addedRiskyTunnels([], [remote(8000)])).toEqual([remote(8000)]);
  });

  test("-L nunca entra no diálogo", () => {
    expect(addedRiskyTunnels([], [local(5432)])).toEqual([]);
  });

  test("-R já salvo não reaparece: o dono já disse sim na criação", () => {
    expect(addedRiskyTunnels([remote(8000)], [remote(8000)])).toEqual([]);
  });

  test("TODOS os arriscados novos são listados — o banner descreve exatamente o que a confirmação aprova", () => {
    const listed = addedRiskyTunnels(
      [remote(8000)],
      [remote(8000), remote(9000), dynamic(1080), local(5432)],
    );
    expect(listed).toEqual([remote(9000), dynamic(1080)]);
  });

  test("bind implícito e explícito são a mesma exposição, como no core", () => {
    expect(
      addedRiskyTunnels(
        [remote(8000)],
        [{ ...remote(8000), listen_host: "127.0.0.1" }],
      ),
    ).toEqual([]);
  });
});
