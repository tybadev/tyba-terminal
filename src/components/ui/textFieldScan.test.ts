import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Regra 29, a varredura. `textFieldDefaults.test.tsx` prova que `ui/input` e
// `ui/textarea` NASCEM sem correção; este prova que ninguém escapa da regra
// escrevendo a tag crua — que foi exatamente como o composer de agente
// (`RichInput`) e mais dezesseis campos ficaram anos com o autocorretor do
// macOS ligado, capitalizando palavra e enfiando espaço no que se digitava.
//
// A exigência NÃO é "desligado": é DECLARADO. Campo técnico desliga; campo de
// prosa (corpo de PR, comentário de review, descrição de snippet) religa
// explicitamente. O que este teste proíbe é o silêncio, porque o silêncio
// herda o padrão do WebKit — e o padrão do WebKit corrige.
const REQUIRED = ["autoCorrect", "spellCheck"];

/** Tipos que não têm texto a corrigir — não precisam declarar nada. */
const NOT_TEXT = /type="(checkbox|radio|range|file|color|number)"/;

const FIELD = /<(textarea|input)\b(.*?)\/>/gs;

function tsxFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...tsxFiles(full));
      continue;
    }
    if (entry.name.endsWith(".tsx") && !entry.name.includes(".test.")) {
      out.push(full);
    }
  }
  return out;
}

describe("nenhum campo de texto cru fica calado sobre correção", () => {
  test("todo <input>/<textarea> fora de ui/ declara autoCorrect e spellCheck", () => {
    const mudos: string[] = [];
    for (const file of tsxFiles("src")) {
      // `ui/` é a fonte do padrão, não um consumidor dele.
      if (file.includes(join("components", "ui"))) continue;
      const src = readFileSync(file, "utf8");
      for (const match of src.matchAll(FIELD)) {
        const attrs = match[2] ?? "";
        if (NOT_TEXT.test(attrs)) continue;
        if (REQUIRED.every((attr) => attrs.includes(attr))) continue;
        const line = src.slice(0, match.index).split("\n").length;
        mudos.push(`${file}:${line} <${match[1]}>`);
      }
    }
    expect(mudos).toEqual([]);
  });

  // O teste acima só vale se a varredura de fato ENXERGA os campos: um regex
  // que parasse de casar passaria a aprovar tudo, calado.
  test("a varredura encontra os campos crus que existem hoje", () => {
    const total = tsxFiles("src")
      .filter((f) => !f.includes(join("components", "ui")))
      .reduce((n, f) => n + [...readFileSync(f, "utf8").matchAll(FIELD)].length, 0);
    expect(total).toBeGreaterThan(15);
  });
});
