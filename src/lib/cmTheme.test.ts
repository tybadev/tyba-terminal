import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "bun:test";

import { cmPalette, type SyntaxPalette } from "./cmTheme";

const ROLES: (keyof SyntaxPalette)[] = [
  "comment",
  "keyword",
  "control",
  "string",
  "function",
  "number",
  "type",
  "variable",
  "tag",
  "invalid",
];

describe("cmPalette", () => {
  it("cobre todos os papéis de token nos dois temas com cores válidas", () => {
    for (const dark of [true, false]) {
      const palette = cmPalette(dark);
      for (const role of ROLES) {
        expect(palette[role]).toMatch(/^#[0-9a-fA-F]{6,8}$/);
      }
    }
  });

  it("dark usa o mono-dark e light usa o vitesse — paletas distintas", () => {
    expect(cmPalette(true).keyword).toBe("#C792EA");
    expect(cmPalette(false).keyword).toBe("#4d9375");
    expect(cmPalette(true).string).not.toBe(cmPalette(false).string);
  });
});

/**
 * A entrelinha do editor não pode nascer no CSS injetado.
 *
 * O CodeMirror mede a altura da linha UMA vez, na montagem, e o CSS de
 * `EditorView.theme()` é injetado por JavaScript — quando ele perde a corrida
 * para a medição, o oráculo de altura fica com a entrelinha padrão (14px) e o
 * DOM renderiza 18px. Ele nunca revisa: `resize`, `requestMeasure()` e
 * `setState` foram medidos na sonda de 26/08/2026 e nenhum refaz a conta.
 *
 * O estrago aparece como DOIS bugs e é um só: o gutter deriva 4px por linha
 * (144px em 36 linhas, com os números empilhando aos pares) e o conteúdo some,
 * porque o viewport é calculado com metade da geometria.
 *
 * A condição existe desde `5fb4164` (21/07/2026), o commit que criou o editor
 * — é corrida, então ficou um ano latente aparecendo só às vezes. Este teste
 * troca a corrida por uma regra: a entrelinha mora numa folha de estilo real,
 * que já está aplicada antes de o React montar.
 */
describe("entrelinha do editor", () => {
  const dir = import.meta.dir;
  const TEMA = readFileSync(join(dir, "cmTheme.ts"), "utf8");
  const CSS = readFileSync(join(dir, "..", "styles.css"), "utf8");

  it("não é declarada no tema que o CodeMirror injeta", () => {
    // `styles.css` é gerado (`scripts/port_styles.py`); a regra vive lá.
    expect(TEMA).not.toContain("lineHeight");
  });

  it("é declarada na folha de estilo, para `.cm-content` e `.cm-gutters`", () => {
    // Os dois: o conteúdo é o que o CodeMirror mede, e o gutter é quem
    // desalinha quando a medição erra.
    expect(CSS).toMatch(/\.cm-content,\s*\n\.cm-gutters\s*\{[^}]*line-height:\s*1\.5/);
  });
});
