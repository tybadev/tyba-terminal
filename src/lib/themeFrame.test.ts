import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "bun:test";

/**
 * A moldura tem o mesmo peso em toda pele.
 *
 * Este teste lê `styles.css` em vez de um fixture porque o arquivo é GERADO
 * (`scripts/port_styles.py`, a partir do `tyba-design-system`): um fixture
 * copiado seria uma terceira cópia dos tokens, e passaria verde enquanto o
 * arquivo real regredisse.
 *
 * O que ele guarda é a decisão de 2026-08-25 (cofre,
 * `decisions/2026-08-25-uma-linha-so-derivada-da-polaridade`): enquanto cada
 * esquema escolhia a própria borda, o peso da linha ia de 16 a 39 entre as
 * peles sem ninguém ter decidido isso — e passava despercebido porque o
 * `tyba-dark` e o `light`, os dois em que a interface foi desenhada, eram
 * justamente os dois em que o valor já estava certo. Um tema novo reabre a
 * costura sem que ninguém veja, que é exatamente o que já aconteceu com o
 * fundo do xterm (ver `terminalTheme.test.ts`).
 */

const CSS = readFileSync(join(import.meta.dir, "..", "styles.css"), "utf8");

type Rgb = [number, number, number];

/** Blocos de tema do arquivo: `:root`, `[data-theme$='light']` e cada esquema. */
function blocks(css: string): Map<string, string> {
  const found = new Map<string, string>();
  const re = /^(:root|\[data-theme[^\]]*\])\s*\{([\s\S]*?)^\}/gm;
  for (const m of css.matchAll(re)) found.set(m[1], m[2]);
  return found;
}

function declared(body: string, name: string): string | null {
  const m = body.match(new RegExp(`^\\s*--tyba-${name}:\\s*([^;]+);`, "m"));
  return m ? m[1].trim() : null;
}

function hex(value: string): Rgb {
  const raw = value.replace("#", "");
  const size = raw.length === 3 ? 1 : 2;
  const at = (i: number) => {
    const part = raw.slice(i * size, (i + 1) * size);
    return parseInt(size === 1 ? part + part : part, 16);
  };
  return [at(0), at(1), at(2)];
}

/** `rgba(r, g, b, a)` — a única forma que a linha da moldura pode ter. */
function rgba(value: string): { rgb: Rgb; alpha: number } {
  const m = value.match(
    /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/,
  );
  if (!m) throw new Error(`não é rgba(): ${value}`);
  return {
    rgb: [Number(m[1]), Number(m[2]), Number(m[3])],
    alpha: m[4] === undefined ? 1 : Number(m[4]),
  };
}

function over(fg: Rgb, alpha: number, bg: Rgb): Rgb {
  return fg.map((c, i) => Math.round(alpha * c + (1 - alpha) * bg[i])) as Rgb;
}

/** Distância média por canal — a mesma medida usada para decidir o alpha. */
function distance(a: Rgb, b: Rgb): number {
  return (
    a.reduce((sum, c, i) => sum + Math.abs(c - b[i]), 0) / 3
  );
}

const parsed = blocks(CSS);
const root = parsed.get(":root");
const lightBase = parsed.get("[data-theme$='light']");

/** Cada esquema herda do `:root` e, se claro, também do bloco `$='light'`. */
function resolve(body: string, name: string): string {
  const own = declared(body, name);
  if (own) return own;
  const isLight = /color-scheme:\s*light/.test(body);
  const inherited = isLight ? declared(lightBase!, name) : null;
  return inherited ?? declared(root!, name)!;
}

const SCHEMES = [...parsed.entries()]
  .filter(([selector]) => /^\[data-theme='/.test(selector))
  .map(([selector, body]) => ({
    id: selector.replace(/^\[data-theme='|'\]$/g, ""),
    body,
  }));

/** As superfícies sobre as quais a linha precisa se sustentar. */
const SURFACES = ["bg", "surface", "sunken", "raised"] as const;

/**
 * Faixa aceita. O piso é o ponto em que a linha ainda existe (a derivada do
 * texto punha o `solarized-dark` em 8, e ali ela some); o teto é o ponto em
 * que ela vira moldura em vez de separação — o `monokai-light` marcava 39.
 */
const FLOOR = 12;
const CEILING = 20;

describe("moldura", () => {
  it("o arquivo tem os blocos que o teste supõe", () => {
    expect(root).toBeDefined();
    expect(lightBase).toBeDefined();
    expect(SCHEMES.length).toBeGreaterThanOrEqual(16);
  });

  it("a linha estrutural é rgba de polaridade, não derivada do texto", () => {
    // `color-mix(text …)` herda a intenção de contraste do esquema, que não é
    // a da moldura. É o que este teste existe para impedir de voltar.
    for (const [selector, body] of parsed) {
      const line = declared(body, "divider-line");
      if (!line) continue;
      expect(`${selector}: ${line}`).toMatch(/rgba?\(/);
      expect(line).not.toContain("color-mix");
    }
  });

  it("nenhum esquema declara a própria borda", () => {
    // A moldura é identidade, não cor: ela herda da polaridade. Um esquema que
    // volte a declarar borda volta a ter um peso só seu.
    const offenders = SCHEMES.filter(
      ({ body }) => declared(body, "border") || declared(body, "border-strong"),
    ).map(({ id }) => id);
    expect(offenders).toEqual([]);
  });

  for (const base of [":root", "[data-theme$='light']"] as const) {
    it(`a linha se sustenta em toda superfície de ${base}`, () => {
      const body = parsed.get(base)!;
      const line = rgba(declared(body, "divider-line")!);
      for (const surface of SURFACES) {
        const bg = hex(resolve(body, surface));
        const d = distance(over(line.rgb, line.alpha, bg), bg);
        expect({ base, surface, d }).toMatchObject({ base, surface });
        expect(d).toBeGreaterThanOrEqual(FLOOR);
        expect(d).toBeLessThanOrEqual(CEILING);
      }
    });
  }

  for (const { id, body } of SCHEMES) {
    it(`a linha se sustenta em toda superfície de ${id}`, () => {
      const line = rgba(resolve(body, "divider-line"));
      for (const surface of SURFACES) {
        const bg = hex(resolve(body, surface));
        const d = distance(over(line.rgb, line.alpha, bg), bg);
        expect({ id, surface, d }).toMatchObject({ id, surface });
        expect(d).toBeGreaterThanOrEqual(FLOOR);
        expect(d).toBeLessThanOrEqual(CEILING);
      }
    });
  }
});
