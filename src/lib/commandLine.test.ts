import { describe, expect, it } from "bun:test";

import {
  clearsDraft,
  controlBytes,
  keyboardOwner,
  type OwnerInput,
} from "./commandLine";

const atPrompt: OwnerInput = {
  promptMode: true,
  kind: { type: "shell" },
  altScreen: false,
  command: { command: null, running: false, agent_match: false },
  integrated: true,
};

describe("keyboardOwner", () => {
  it("dá a linha ao TYBA quando o shell está no prompt", () => {
    expect(keyboardOwner(atPrompt)).toBe("tybaLine");
  });

  it("devolve o teclado ao terminal enquanto um comando roda", () => {
    // A regra que impede o usuário de digitar a senha do sudo numa caixa que
    // não vai a lugar nenhum.
    const running = {
      ...atPrompt,
      command: { command: "ssh prod", running: true, agent_match: false },
    };
    expect(keyboardOwner(running)).toBe("terminal");
  });

  it("devolve o teclado ao terminal em alt-screen", () => {
    expect(keyboardOwner({ ...atPrompt, altScreen: true })).toBe("terminal");
  });

  it("alt-screen vence mesmo com o shell ocioso", () => {
    // `vim` deixa o shell sem comando rodando do ponto de vista do OSC 133,
    // mas a tela é dele.
    const inVim = { ...atPrompt, altScreen: true, command: undefined };
    expect(keyboardOwner(inVim)).toBe("terminal");
  });

  it("nunca assume o prompt sem shell integration", () => {
    expect(keyboardOwner({ ...atPrompt, integrated: false })).toBe("terminal");
  });

  it("não toma a linha de sessão de agente", () => {
    const agent: OwnerInput = {
      ...atPrompt,
      kind: { type: "agent", runner: "claude_code" },
    };
    expect(keyboardOwner(agent)).toBe("terminal");
  });

  it("não toma a linha de sessão ssh", () => {
    const ssh: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
    };
    expect(keyboardOwner(ssh)).toBe("terminal");
  });

  it("respeita a válvula de escape do usuário", () => {
    expect(keyboardOwner({ ...atPrompt, promptMode: false })).toBe("terminal");
  });
});

describe("controlBytes", () => {
  it("traduz os sinais que a caixa nunca consome", () => {
    expect(controlBytes({ key: "c", ctrl: true, meta: false, alt: false })).toBe(
      "\x03",
    );
    expect(controlBytes({ key: "d", ctrl: true, meta: false, alt: false })).toBe(
      "\x04",
    );
    expect(controlBytes({ key: "z", ctrl: true, meta: false, alt: false })).toBe(
      "\x1a",
    );
  });

  it("ignora letra sem ctrl e chord com meta", () => {
    expect(controlBytes({ key: "c", ctrl: false, meta: false, alt: false })).toBeNull();
    expect(controlBytes({ key: "c", ctrl: true, meta: true, alt: false })).toBeNull();
  });

  it("ignora ctrl em tecla que não é sinal", () => {
    expect(controlBytes({ key: "a", ctrl: true, meta: false, alt: false })).toBeNull();
  });

  it("só o Ctrl+C limpa o rascunho", () => {
    expect(clearsDraft({ key: "c", ctrl: true, meta: false, alt: false })).toBe(true);
    expect(clearsDraft({ key: "d", ctrl: true, meta: false, alt: false })).toBe(false);
  });
});
