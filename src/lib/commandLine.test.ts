import { describe, expect, it } from "bun:test";

import {
  clearsDraft,
  controlBytes,
  ghostFor,
  keyboardOwner,
  lineState,
  lineToken,
  pathToken,
  replaceToken,
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

describe("lineState", () => {
  it("é editável só quando o shell está no prompt", () => {
    expect(lineState(atPrompt)).toBe("own");
  });

  it("diz por que não é editável, em vez de sumir", () => {
    // A caixa sumir e voltar a cada comando redimensionava o terminal duas
    // vezes por execução.
    expect(
      lineState({
        ...atPrompt,
        command: { command: "sleep 5", running: true, agent_match: false },
      }),
    ).toBe("running");
    expect(lineState({ ...atPrompt, altScreen: true })).toBe("app");
    expect(lineState({ ...atPrompt, promptMode: false })).toBe("waiting");
  });

  it("app de tela cheia vence comando rodando", () => {
    expect(
      lineState({
        ...atPrompt,
        altScreen: true,
        command: { command: "vim", running: true, agent_match: false },
      }),
    ).toBe("app");
  });
});

describe("ghostFor", () => {
  const hits = [
    { command: "cargo test --lib" },
    { command: "cargo clippy" },
    { command: "git status" },
  ];

  it("completa com a primeira sugestão que é prefixo", () => {
    expect(ghostFor("cargo t", hits)).toBe("est --lib");
  });

  it("pula sugestão que não começa com o digitado", () => {
    // "git status" está na lista, mas o ghost tem de casar o prefixo — senão o
    // cinza mentiria sobre o que a seta vai completar.
    expect(ghostFor("gi", hits)).toBe("t status");
    expect(ghostFor("zzz", hits)).toBe("");
  });

  it("não sugere nada com a linha vazia ou só espaço", () => {
    expect(ghostFor("", hits)).toBe("");
    expect(ghostFor("   ", hits)).toBe("");
  });

  it("não sugere quando o digitado já é o comando inteiro", () => {
    expect(ghostFor("cargo clippy", hits)).toBe("");
  });
});

describe("pathToken", () => {
  const at = (text: string) => pathToken(text, text.length);

  it("completa argumento como caminho", () => {
    expect(at("cd tyba")).toEqual({ start: 3, value: "tyba" });
    expect(at("cat src/lib/ip")).toEqual({ start: 4, value: "src/lib/ip" });
  });

  it("não completa a primeira palavra: ali é posição de comando", () => {
    // Num diretório com `teste/`, completar `te` para `teste/` quebraria quem
    // está digitando `test`.
    expect(at("te")).toBeNull();
    expect(at("git")).toBeNull();
  });

  it("completa a primeira palavra quando ela já se declara caminho", () => {
    expect(at("./scr")).toEqual({ start: 0, value: "./scr" });
    expect(at("../ou")).toEqual({ start: 0, value: "../ou" });
    expect(at("~/proj")).toEqual({ start: 0, value: "~/proj" });
    expect(at("/usr/lo")).toEqual({ start: 0, value: "/usr/lo" });
    expect(at("bin/tool")).toEqual({ start: 0, value: "bin/tool" });
  });

  it("não completa depois de espaço nem em linha vazia", () => {
    expect(at("cd ")).toBeNull();
    expect(at("")).toBeNull();
  });

  it("usa o token sob o cursor, não o fim da linha", () => {
    expect(pathToken("cp src dst", 5)).toEqual({ start: 3, value: "sr" });
  });
});

describe("replaceToken", () => {
  it("troca só o token e devolve o cursor depois dele", () => {
    const text = "cp src dst";
    const token = pathToken(text, 5)!;
    expect(replaceToken(text, token, "src/")).toEqual({
      text: "cp src/c dst",
      caret: 7,
    });
  });
});

describe("lineToken", () => {
  const at = (text: string) => lineToken(text, text.length);

  it("dá o token e o contexto que vem antes dele", () => {
    expect(at("git co")).toEqual({ start: 4, value: "co", prefix: "git " });
    expect(at("cargo test --l")).toEqual({
      start: 11,
      value: "--l",
      prefix: "cargo test ",
    });
  });

  it("aceita token vazio logo após o espaço", () => {
    // `git ` sem nada digitado ainda deve oferecer os subcomandos usados.
    expect(at("git ")).toEqual({ start: 4, value: "", prefix: "git " });
  });

  it("não completa a primeira palavra: ali não há contexto", () => {
    expect(at("gi")).toBeNull();
    expect(at("")).toBeNull();
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
