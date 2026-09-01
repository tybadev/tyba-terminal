import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "bun:test";

/**
 * Contrato de FIAÇÃO — "quem chama quem" — não de resultado.
 *
 * `refit()` é o único caminho que compara `cols`/`rows` contra o PTY e avisa
 * `resizeSession` (D4); a medida da faixa ao vivo tem de considerar a
 * última linha escrita, não só o cursor (D6). As duas são invariantes de
 * ligação dentro de `TerminalView.tsx`, sem correspondente observável num
 * teste puro sem montar um xterm real — mockar o xterm inteiro pra testar
 * isso seria mockar um colaborador interno, não uma borda.
 *
 * Por isso o teste lê o arquivo de verdade, como `terminalFont.test.ts` já
 * faz: uma cópia da regra aqui pinaria o dia em que o código real
 * regredisse por baixo dela.
 */

const HERE = import.meta.dir;
const SOURCE = readFileSync(
  join(HERE, "..", "components", "TerminalView.tsx"),
  "utf8",
);

/** O código sem comentários — senão `fit.fit()` mencionado numa doc conta. */
function stripComments(source: string): string {
  let out = "";
  let i = 0;
  while (i < source.length) {
    const ch = source[i];
    if (ch === "/" && source[i + 1] === "/") {
      const nl = source.indexOf("\n", i);
      i = nl === -1 ? source.length : nl;
      continue;
    }
    if (ch === "/" && source[i + 1] === "*") {
      const end = source.indexOf("*/", i + 2);
      i = end === -1 ? source.length : end + 2;
      continue;
    }
    if (ch === '"' || ch === "'" || ch === "`") {
      const quote = ch;
      out += ch;
      i++;
      while (i < source.length && source[i] !== quote) {
        if (source[i] === "\\" && i + 1 < source.length) {
          out += source[i] + source[i + 1];
          i += 2;
          continue;
        }
        out += source[i];
        i++;
      }
      if (i < source.length) {
        out += source[i];
        i++;
      }
      continue;
    }
    out += ch;
    i++;
  }
  return out;
}

const CODE = stripComments(SOURCE);

/**
 * O corpo de `const NOME = (...) => { ... }`, pelo balanceamento de chaves
 * — pula strings/template literals, então um `{` dentro de uma delas não
 * conta pro fechamento.
 */
function functionBody(code: string, name: string): string {
  const marker = new RegExp(`const\\s+${name}\\b\\s*=[^{]*\\{`);
  const found = code.match(marker);
  if (!found || found.index === undefined) {
    throw new Error(`função ${name} não encontrada em TerminalView.tsx`);
  }
  let depth = 1;
  let i = found.index + found[0].length;
  const start = i;
  while (i < code.length && depth > 0) {
    const ch = code[i];
    if (ch === '"' || ch === "'" || ch === "`") {
      const quote = ch;
      i++;
      while (i < code.length && code[i] !== quote) {
        if (code[i] === "\\") i++;
        i++;
      }
      i++;
      continue;
    }
    if (ch === "{") depth++;
    else if (ch === "}") depth--;
    i++;
  }
  return code.slice(start, i - 1);
}

describe("contrato de fiação — TerminalView.tsx", () => {
  // D4: os dois caminhos que hoje chamavam `fit.fit()` cru (abertura e
  // `[visible, rect]`) passaram a chamar `refit()`. Mutante que reverte
  // qualquer um deles de volta pro fit cru precisa reprovar aqui — os 811
  // testes de antes ficavam verdes com a mutação, porque nada testava a
  // LIGAÇÃO (só o `sameRect`/`usedRowsFromLastLine` isolados).
  it("fit.fit() só existe dentro do corpo de refit() — todo fit cru avisa o PTY", () => {
    const totalCalls = (CODE.match(/\bfit\.fit\(\)/g) ?? []).length;
    const refitBody = functionBody(CODE, "refit");
    const callsInsideRefit = (refitBody.match(/\bfit\.fit\(\)/g) ?? [])
      .length;
    expect(totalCalls).toBeGreaterThan(0);
    expect(callsInsideRefit).toBe(totalCalls);
  });

  // D6: mesma lacuna, pro outro lado — reverter `measureLive` de volta a só
  // `cursorY + 1` não quebrava nenhum teste, porque `usedRowsFromLastLine`
  // ficava correta e testada, só não CHAMADA.
  it("a medida da faixa ao vivo passa por usedRowsFromLastLine, não só pelo cursor", () => {
    const measureLiveBody = functionBody(CODE, "measureLive");
    expect(measureLiveBody).toMatch(/usedRowsFromLastLine\(/);
  });
});
