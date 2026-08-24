import { describe, expect, test } from "bun:test";

import type { Block } from "./ipc";
import { sessionBlocksData } from "./sessionBlocks";

type Input = Parameters<typeof sessionBlocksData>[0];

/**
 * A comparação que o `React.memo` faz: `Object.is` campo a campo sobre as
 * props. É por ela que passa a decisão de re-renderizar ou não — e era ela que
 * falhava em todo quadro quando o painel era montado inline no `App`.
 */
function shallowEqual(a: object, b: object): boolean {
  const left = Object.keys(a) as Array<keyof typeof a>;
  const right = Object.keys(b);
  if (left.length !== right.length) return false;
  return left.every((key) => Object.is(a[key], b[key]));
}

/**
 * Um render do `App`, com o estado desta sessão parado.
 *
 * Cada chamada monta `session` e `pane` do zero de propósito: o que se quer
 * provar é que as props saem iguais POR VALOR, sem depender de o chamador
 * reaproveitar objeto nenhum.
 */
function renderProps(overrides: Partial<Input> = {}) {
  return sessionBlocksData({
    session: { id: "s1", created_at: "2026-08-21T10:00:00Z" },
    pane: { pane: "p1", x: 0, y: 0, w: 50, h: 100 },
    blocks: undefined,
    live: false,
    used: undefined,
    headerPx: undefined,
    fontSizePx: 13,
    lineHeightPx: 21.5,
    cellWidthPx: 8,
    cwd: undefined,
    active: true,
    command: undefined,
    marked: null,
    copyCombo: "⌘C",
    ...overrides,
  });
}

describe("sessionBlocksData", () => {
  test("dois renders com o mesmo estado produzem props shallow-iguais", () => {
    // É este o teto do defeito: props iguais ⇒ `memo` corta o render. Enquanto
    // `rect`, `opened` e `onActivate` nasciam no JSX do `App`, esta comparação
    // dava `false` nos ~60 renders por segundo, e a memoização da lista de
    // blocos custava sem devolver nada.
    expect(shallowEqual(renderProps(), renderProps())).toBe(true);
  });

  test("o formato antigo falhava em todo render", () => {
    // O controle negativo, para o teste acima não passar por acidente: era
    // assim que o call site montava as props.
    const inline = () => ({
      rect: { left: 0, top: 0, width: 50, height: 100 },
      opened: { cwd: null, atMs: null },
      onActivate: () => {},
    });
    expect(shallowEqual(inline(), inline())).toBe(false);
  });

  test("sessão sem bloco nenhum não inventa lista nova", () => {
    // `blocks[id] ?? []` é o mais fácil de reintroduzir: o literal parece
    // constante e não é.
    expect(renderProps().blocks).toBe(renderProps().blocks);
  });

  test("a lista de blocos passa por referência", () => {
    // Não basta ser igual: o `memo` do cartão compara identidade, e copiar a
    // lista aqui derrubaria a memoização de dentro da lista também.
    const list: Block[] = [];
    expect(renderProps({ blocks: list }).blocks).toBe(list);
  });

  test("painel que se move muda as props", () => {
    // A outra metade: memo que nunca corta é desperdício, memo que corta demais
    // deixa a lista desenhada no lugar antigo.
    expect(
      shallowEqual(
        renderProps(),
        renderProps({ pane: { pane: "p1", x: 50, y: 0, w: 50, h: 100 } }),
      ),
    ).toBe(false);
  });

  test("faixa ao vivo que abre ou encolhe muda as props", () => {
    expect(shallowEqual(renderProps(), renderProps({ live: true }))).toBe(false);
    expect(shallowEqual(renderProps(), renderProps({ used: 0.4 }))).toBe(false);
  });

  test("faixa sem medida vale como cheia, e header sem medida como zero", () => {
    // Os defaults saem do `App` para cá justamente para não virarem objeto novo
    // no JSX; conferir aqui garante que a mudança de casa não trocou o valor.
    expect(renderProps().used).toBe(1);
    expect(renderProps().headerPx).toBe(0);
  });

  test("sessão sem cwd conhecido ainda ganha o cartão-zero", () => {
    expect(
      renderProps({ cwd: { cwd: "~/dev", canonical: "/Users/x/dev" } })
        .openedCwd,
    ).toBe("~/dev");
    expect(renderProps().openedCwd).toBe(null);
  });

  test("data de abertura ilegível não vira NaN na tela", () => {
    expect(
      renderProps({ session: { id: "s1", created_at: "lixo" } }).openedAtMs,
    ).toBe(null);
  });
});
