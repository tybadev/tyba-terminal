import { describe, expect, test } from "bun:test";

import type { LspContentChange } from "./ipc";
import { createChangeBatcher } from "./lspChangeBatch";

function fakeTimers() {
  let next = 1;
  const pending = new Map<number, () => void>();
  return {
    setTimeout: (cb: () => void, _ms: number) => {
      const h = next++;
      pending.set(h, cb);
      return h;
    },
    clearTimeout: (h: number) => {
      pending.delete(h);
    },
    runAll: () => {
      const cbs = [...pending.values()];
      pending.clear();
      for (const cb of cbs) cb();
    },
    get size() {
      return pending.size;
    },
  };
}

// Aplica mudanças LSP (linha/coluna 0-based) em sequência — o que o server faz.
function apply(text: string, changes: LspContentChange[]): string {
  let doc = text;
  for (const c of changes) {
    if (!c.range) {
      doc = c.text;
      continue;
    }
    const offset = (line: number, ch: number) => {
      const lines = doc.split("\n");
      let o = 0;
      for (let i = 0; i < line; i++) o += lines[i].length + 1;
      return o + ch;
    };
    const from = offset(c.range.start.line, c.range.start.character);
    const to = offset(c.range.end.line, c.range.end.character);
    doc = doc.slice(0, from) + c.text + doc.slice(to);
  }
  return doc;
}

const insert = (line: number, character: number, text: string): LspContentChange => ({
  range: { start: { line, character }, end: { line, character } },
  text,
});

describe("createChangeBatcher", () => {
  test("uma rajada de teclas vira um único envio", () => {
    const timers = fakeTimers();
    const sent: LspContentChange[][] = [];
    const b = createChangeBatcher((c) => sent.push(c), 120, timers);
    const word = "const answer = 42;";
    for (let i = 0; i < word.length; i++) b.push([insert(0, i, word[i])]);
    expect(sent.length).toBe(0);
    expect(timers.size).toBe(1);
    timers.runAll();
    expect(sent.length).toBe(1);
    expect(sent[0].length).toBe(word.length);
    expect(apply("", sent[0])).toBe(word);
  });

  test("o lote dá o mesmo texto que mandar tecla a tecla", () => {
    const steps: LspContentChange[][] = [
      [insert(0, 0, "fn x() {}\n")],
      [insert(0, 3, "yz")],
      [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 2 } }, text: "pub fn" }],
      // transação com duas mudanças, já em ordem de aplicação sequencial
      [insert(1, 0, "// b"), insert(0, 0, "// a\n")],
    ];
    const oneByOne = steps.reduce((doc, s) => apply(doc, s), "");
    const timers = fakeTimers();
    const sent: LspContentChange[][] = [];
    const b = createChangeBatcher((c) => sent.push(c), 120, timers);
    for (const s of steps) b.push(s);
    b.flush();
    expect(sent.length).toBe(1);
    expect(apply("", sent[0])).toBe(oneByOne);
  });

  test("flush manda na hora e cancela o timer; sem pendente não envia", () => {
    const timers = fakeTimers();
    const sent: LspContentChange[][] = [];
    const b = createChangeBatcher((c) => sent.push(c), 120, timers);
    b.flush();
    expect(sent.length).toBe(0);
    b.push([insert(0, 0, "a")]);
    b.flush();
    expect(sent.length).toBe(1);
    expect(timers.size).toBe(0);
    timers.runAll();
    expect(sent.length).toBe(1);
  });

  test("clear descarta o pendente (documento reaberto)", () => {
    const timers = fakeTimers();
    const sent: LspContentChange[][] = [];
    const b = createChangeBatcher((c) => sent.push(c), 120, timers);
    b.push([insert(0, 0, "a")]);
    b.clear();
    timers.runAll();
    b.flush();
    expect(sent.length).toBe(0);
  });
});
