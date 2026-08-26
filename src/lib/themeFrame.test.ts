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
  for (const m of css.matchAll(re)) {
    // Sobrescrever em silêncio seria pior que falhar: `styles.css` é gerado
    // fora deste repo, e um segundo `:root` no topo (uma variante de fonte,
    // um override de reduced-motion) faria `resolve()` passar a herdar do
    // bloco errado — a suíte mediria outra paleta e continuaria verde.
    if (found.has(m[1])) throw new Error(`seletor repetido: ${m[1]}`);
    found.set(m[1], m[2]);
  }
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

  it("os quatro divisores são inset e apontam para a mesma linha", () => {
    // `inset` é o mecanismo em que o refactor inteiro se apoia, e até aqui
    // nada o guardava: revertê-lo para a sombra externa passava nos 23 testes.
    // Por fora a divisa invade o vizinho e some quando o vizinho tem pilha
    // própria — foi assim que a borda direita da sidebar não renderizou um
    // único pixel — e compõe sobre o fundo DELE, o que dá duas cores para uma
    // linha só.
    const esperado: Record<string, string> = {
      "divider-b": "inset 0 -1px 0 0 var(--tyba-divider-line)",
      "divider-t": "inset 0 1px 0 0 var(--tyba-divider-line)",
      "divider-r": "inset -1px 0 0 0 var(--tyba-divider-line)",
      "divider-l": "inset 1px 0 0 0 var(--tyba-divider-line)",
    };
    for (const [nome, valor] of Object.entries(esperado)) {
      expect(`${nome}: ${declared(root!, nome)}`).toBe(`${nome}: ${valor}`);
    }
  });

  it("cada divisor tem uma classe que o consome", () => {
    // `--tyba-divider-l` nasceu sem nenhuma asserção. Um token declarado e
    // nunca consumido é dívida silenciosa: some numa limpeza e ninguém vê.
    for (const lado of ["b", "t", "r", "l"]) {
      expect(CSS).toContain(`.tyba-divide-${lado} {`);
      expect(CSS).toContain(`box-shadow: var(--tyba-divider-${lado});`);
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
    it(`o contorno de controle se sustenta em ${base}`, () => {
      // `--tyba-border` e `--tyba-divider-line` hoje carregam o MESMO literal
      // nas duas polaridades, e nada os mantém iguais — são dois tokens
      // editados à mão que por ora concordam. Só um deles tinha banda.
      // `--tyba-border` pinta as tabelas do StatsView, os cartões `.tyba-lit`,
      // as regras do `.files-markdown` e a borda de topo da CommandLine;
      // `--tyba-border-strong` dá o polegar da scrollbar e a barra de citação.
      const body = parsed.get(base)!;
      const border = rgba(declared(body, "border")!);
      const strong = rgba(declared(body, "border-strong")!);
      for (const surface of SURFACES) {
        const bg = hex(resolve(body, surface));
        const d = distance(over(border.rgb, border.alpha, bg), bg);
        expect(d).toBeGreaterThanOrEqual(FLOOR);
        expect(d).toBeLessThanOrEqual(CEILING);
        // O forte é forte de propósito: foco e hover têm de se destacar do
        // contorno em repouso, não empatar com ele.
        const ds = distance(over(strong.rgb, strong.alpha, bg), bg);
        expect(ds).toBeGreaterThan(CEILING);
      }
    });
  }

  for (const base of [":root", "[data-theme$='light']"] as const) {
    it(`a linha se sustenta em toda superfície de ${base}`, () => {
      const body = parsed.get(base)!;
      const line = rgba(declared(body, "divider-line")!);
      for (const surface of SURFACES) {
        const bg = hex(resolve(body, surface));
        const d = distance(over(line.rgb, line.alpha, bg), bg);
        expect(d).toBeGreaterThanOrEqual(FLOOR);
        expect(d).toBeLessThanOrEqual(CEILING);
      }
    });
  }

  // Aqui viveu um teste de FRONTEIRA — a linha medida contra a superfície do
  // vizinho, e não só contra a sua. Ele nasceu de um achado real: no
  // `github-light` a linha da sidebar rende (234,234,235) contra um terminal
  // em (235,238,242), então ali ela não separa nada; quem separa é o degrau.
  //
  // Foi removido porque não podia falhar sozinho. A afirmação era
  // `max(|X−linha|, |linha−Y|) ≥ piso`, e o primeiro termo é o contraste da
  // linha contra a PRÓPRIA superfície — exatamente o que o teste acima já
  // exige. Provado empiricamente com um tema de meio-tom: quem acusou foi o
  // teste de cima, e o de fronteira passou verde junto.
  //
  // Um teste que não pode falhar mede o próprio harness. O achado do
  // `github-light` continua verdadeiro e está no cofre; ele só não vira
  // asserção enquanto for consistência de mecanismo, e não de legibilidade.

  /**
   * A luz do cromo tem sempre um ingrediente vivo.
   *
   * `edge` morre no claro (branco sobre branco) e `cast` morre no preto
   * absoluto do BLACKOUT. Eles convivem justamente por isso — mas nada
   * impede um tema futuro de cair no vão onde os dois somem.
   */
  for (const [base, label] of [
    [":root", "escuro"],
    ["[data-theme$='light']", "claro"],
  ] as const) {
    it(`a luz do cromo tem um ingrediente vivo no ${label} padrão`, () => {
      const body = parsed.get(base)!;
      const edge = rgba(declared(body, "lift-edge")!);
      const cast = rgba(declared(body, "lift-cast")!);
      const surface = hex(resolve(body, "surface"));
      const sunken = hex(resolve(body, "sunken"));
      // `edge` é desenhado sobre a própria superfície; `cast` sobre a de trás.
      const viaEdge = distance(over(edge.rgb, edge.alpha, surface), surface);
      const viaCast = distance(over(cast.rgb, cast.alpha, sunken), sunken);
      expect(Math.max(viaEdge, viaCast)).toBeGreaterThanOrEqual(FLOOR);
    });
  }

  /**
   * No escuro, a sombra do cromo é profundidade — nunca uma faixa.
   *
   * O `cast` nasceu em `rgba(0,0,0,0.85)` sob a premissa de que ele "some sobre
   * preto absoluto". A premissa é verdadeira, e foi conferida no único esquema
   * onde ela vale: o BLACKOUT do `:root`, com `bg` em `#050505`. Os outros doze
   * escuros têm fundo levantado — o `monokai-machine` chega a `#2f2f2f` — e ali
   * a sombra de 16px da `.tyba-lift-r` caía sobre o terminal como uma FAIXA de
   * ~10px. Foi o que apareceu como uma camada entre a sidebar e o shell.
   *
   * O teto é o PISO da linha, e a escolha tem motivo: a linha é 1px, a sombra é
   * uma dezena deles. Uma banda que chegue ao ponto em que uma linha já
   * separaria sozinha deixou de ser profundidade e virou superfície. Ficando
   * abaixo do piso ela nunca compete com a divisa — no escuro quem separa
   * continua sendo o `edge`.
   *
   * `EDGE_FACTOR` é MODELO, não medição: uma borda desfocada rende perto de
   * metade do alpha no ponto exato da costura. Serve para ordenar os esquemas
   * entre si e para prender a regressão; não é um valor amostrado da tela.
   */
  const EDGE_FACTOR = 0.5;

  for (const { id, body } of SCHEMES.filter(
    ({ body }) => !/color-scheme:\s*light/.test(body),
  )) {
    it(`a sombra do cromo não vira faixa em ${id}`, () => {
      const cast = rgba(resolve(body, "lift-cast"));
      // O `bg` é o que recebe a sombra: a `.tyba-lift-r` da sidebar projeta
      // para dentro da área de conteúdo, não sobre outra superfície de cromo.
      const bg = hex(resolve(body, "bg"));
      const seam = distance(over(cast.rgb, cast.alpha * EDGE_FACTOR, bg), bg);
      expect(seam).toBeLessThan(FLOOR);
    });
  }

  for (const { id, body } of SCHEMES) {
    it(`a linha se sustenta em toda superfície de ${id}`, () => {
      const line = rgba(resolve(body, "divider-line"));
      for (const surface of SURFACES) {
        const bg = hex(resolve(body, surface));
        const d = distance(over(line.rgb, line.alpha, bg), bg);
        expect(d).toBeGreaterThanOrEqual(FLOOR);
        expect(d).toBeLessThanOrEqual(CEILING);
      }
    });
  }
});
