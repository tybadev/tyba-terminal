import { describe, expect, it } from "bun:test";

import { blockMarkdown, blockOutput, duration, failed } from "./blockText";
import type { Block } from "./ipc";

function block(over: Partial<Block> = {}): Block {
  return {
    id: 1,
    sessionId: "s1",
    command: "ls",
    exitCode: 0,
    startedAtMs: 0,
    finishedAtMs: 0,
    lines: [],
    truncated: 0,
    ...over,
  };
}

function textLines(...texts: string[]) {
  return texts.map((text) => ({ text, runs: [] }));
}

describe("failed", () => {
  it("treats a clean exit as success", () => {
    expect(failed(0)).toBe(false);
  });

  it("treats a real non-zero exit as failure", () => {
    expect(failed(2)).toBe(true);
  });

  it("does not paint Ctrl+C or SIGPIPE as failure", () => {
    // Vermelho que aparece toda vez que alguém aperta Ctrl+C vira vermelho que
    // ninguém olha.
    expect(failed(130)).toBe(false);
    expect(failed(141)).toBe(false);
  });

  it("says nothing when there is no exit code yet", () => {
    // Bloco interrompido por crash: `exit_code` nulo não é falha, é ignorância.
    expect(failed(null)).toBe(false);
  });
});

describe("duration", () => {
  it("stays quiet under a second", () => {
    expect(duration(block({ startedAtMs: 0, finishedAtMs: 900 }))).toBeNull();
  });

  it("shows seconds with one decimal", () => {
    expect(duration(block({ startedAtMs: 0, finishedAtMs: 4200 }))).toBe("4.2s");
  });

  it("switches to minutes past a minute", () => {
    expect(duration(block({ startedAtMs: 0, finishedAtMs: 125_000 }))).toBe(
      "2min",
    );
  });
});

describe("blockOutput", () => {
  it("joins the logical lines and nothing else", () => {
    expect(blockOutput(block({ lines: textLines("um", "dois") }))).toBe(
      "um\ndois",
    );
  });

  it("copies past the body limit of the card", () => {
    // A garantia da spec: a ação lê o modelo, não o render. O corpo desenha 200
    // linhas; a cópia entrega as 500 que o bloco tem.
    const many = textLines(...Array.from({ length: 500 }, (_, i) => `l${i}`));
    expect(blockOutput(block({ lines: many })).split("\n")).toHaveLength(500);
  });

  it("does not end with a newline", () => {
    // O destino comum é a linha de comando, onde quebra final é Enter.
    expect(blockOutput(block({ lines: textLines("um") }))).toBe("um");
  });

  it("is empty when the command printed nothing", () => {
    expect(blockOutput(block())).toBe("");
  });
});

describe("blockMarkdown", () => {
  it("puts the command and the output in one console fence", () => {
    const out = blockMarkdown(
      block({ command: "ls", lines: textLines("a", "b") }),
    );
    expect(out).toBe("```console\n$ ls\na\nb\n```");
  });

  it("grows the fence when the output carries backticks", () => {
    // `cat README.md` traz ``` na saída; cerca de três fecharia o bloco ali.
    const out = blockMarkdown(
      block({ command: "cat r.md", lines: textLines("```", "js", "```") }),
    );
    expect(out.startsWith("````console\n")).toBe(true);
    expect(out.endsWith("\n````")).toBe(true);
  });

  it("reports a real failure after the fence", () => {
    const out = blockMarkdown(
      block({ command: "ls /nope", exitCode: 2, lines: textLines("no such") }),
    );
    expect(out.split("\n").at(-1)).toBe("exit 2");
  });

  it("says nothing about exit when the command worked", () => {
    expect(blockMarkdown(block({ lines: textLines("ok") }))).not.toContain(
      "exit",
    );
  });

  it("admits truncation instead of passing a cut output as whole", () => {
    const out = blockMarkdown(block({ truncated: 12, lines: textLines("x") }));
    expect(out.split("\n").at(-1)).toBe("12 lines omitted");
  });

  it("joins the notes on a single line", () => {
    const out = blockMarkdown(
      block({
        exitCode: 1,
        truncated: 3,
        startedAtMs: 0,
        finishedAtMs: 2000,
        lines: textLines("x"),
      }),
    );
    expect(out.split("\n").at(-1)).toBe("exit 1 · 3 lines omitted · 2.0s");
  });

  it("drops the prompt line for a block with no command", () => {
    // Bloco background: saída que chegou sem comando ativo. Um `$ ` sozinho
    // inventaria um comando vazio que ninguém rodou.
    const out = blockMarkdown(block({ command: "", lines: textLines("solto") }));
    expect(out).toBe("```console\nsolto\n```");
  });
});
