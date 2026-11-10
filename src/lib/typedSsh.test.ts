import { describe, expect, test } from "bun:test";

import { emptyHostForm } from "./hostForm";
import type { SessionCommand, TypedSshDetected } from "./ipc";
import { dayMonth, hostFormFromDetected, liveSshNote, targetLabel } from "./typedSsh";

describe("targetLabel", () => {
  test("user@host:porta, como o core", () => {
    expect(targetLabel({ user: "root", host: "203.0.113.10", port: 22 })).toBe("root@203.0.113.10:22");
    expect(targetLabel({ user: null, host: "web.example.test", port: 2222 })).toBe("web.example.test:2222");
  });
  test("IPv6 entre colchetes", () => {
    expect(targetLabel({ user: "me", host: "2001:db8::1", port: 2200 })).toBe("me@[2001:db8::1]:2200");
  });
});

describe("dayMonth", () => {
  test("dd/mm na hora local", () => {
    expect(dayMonth(new Date(2026, 10, 9, 14, 2).getTime())).toBe("09/11");
    expect(dayMonth(new Date(2026, 0, 28).getTime())).toBe("28/01");
  });
});

const detected = (over: Partial<TypedSshDetected> = {}): TypedSshDetected => ({
  session_id: "s1",
  block_started_at_ms: 100,
  destination: { host: "203.0.113.10", user: "root", port: null, identity_file: null, proxy_jump: null },
  host_id: null,
  target: { user: "root", host: "203.0.113.10", port: 22 },
  open: false,
  ...over,
});

const running = (started: number | null, isRunning = true): SessionCommand =>
  ({
    command: "ssh root@203.0.113.10",
    running: isRunning,
    agent_match: false,
    continuation: false,
    started_at_ms: started,
  }) as SessionCommand;

describe("liveSshNote", () => {
  test("no bloco que gerou o aviso", () => {
    const note = detected();
    expect(liveSshNote(note, running(100))).toBe(note);
  });
  test("some quando o comando acaba ou é outro", () => {
    expect(liveSshNote(detected(), running(100, false))).toBeNull();
    expect(liveSshNote(detected(), running(200))).toBeNull();
    expect(liveSshNote(detected(), undefined)).toBeNull();
    expect(liveSshNote(undefined, running(100))).toBeNull();
  });
  test("sem started_at no core, vale enquanto roda", () => {
    const note = detected({ block_started_at_ms: null });
    expect(liveSshNote(note, running(5))).toBe(note);
  });
});

describe("hostFormFromDetected", () => {
  test("pré-preenche o cadastro com o destino", () => {
    expect(hostFormFromDetected(detected())).toEqual({
      ...emptyHostForm(),
      alias: "203.0.113.10",
      hostname: "203.0.113.10",
      username: "root",
    });
  });
  test("porta, chave e salto quando digitados", () => {
    const form = hostFormFromDetected(
      detected({
        destination: { host: "app", user: null, port: 2222, identity_file: "~/.ssh/k", proxy_jump: "bastion" },
        target: { user: null, host: "app", port: 2222 },
      }),
    );
    expect(form).toMatchObject({
      alias: "app",
      hostname: "app",
      port: "2222",
      username: "",
      auth_method: "file",
      identity_file: "~/.ssh/k",
      proxy_jump: "bastion",
    });
  });
});
