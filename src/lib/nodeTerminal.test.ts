import { expect, test } from "bun:test";

import {
  canvasWheel,
  nodeFontSize,
  nodeGrid,
  nodeLiveClip,
  nodeTerminalClass,
  nodeZoomStyle,
  type Cell,
} from "./nodeTerminal";

/**
 * A célula que o renderer WebGL do xterm 5.5 desenha (WebglRenderer
 * `_updateDimensions`): largura `floor(charW × dpr) / dpr`, altura
 * `floor(ceil(charH × dpr) × lineHeight) / dpr`. O glifo da JetBrains Mono
 * mede ~0,6 × a fonte de largura e ~1,32 × de altura.
 */
const webglCell =
  (dpr: number) =>
  (fontSize: number): Cell => ({
    w: Math.floor(fontSize * 0.6 * dpr) / dpr,
    h: Math.floor(Math.ceil(fontSize * 1.32 * dpr) * 1.35) / dpr,
  });

const BASE = 13;
const SCROLLBAR = 15;
/** O nó: 520×300 px de layout, que o transform do canvas não muda. */
const NODE = { w: 520, h: 300 };

test("cols/rows do nó são as do fit em 100% e não mudam com o zoom", () => {
  for (const dpr of [1, 2]) {
    const cell = webglCell(dpr);
    const grid = nodeGrid(NODE, cell(BASE), SCROLLBAR);
    // As contas do FitAddon em 100%: (largura − barra) ÷ célula, altura ÷ célula.
    expect(grid).toEqual({
      cols: Math.floor((NODE.w - SCROLLBAR) / cell(BASE).w),
      rows: Math.floor(NODE.h / cell(BASE).h),
    });
    // Em qualquer zoom a grade é a mesma: o tamanho do PTY não depende dele.
    for (const zoom of [0.2, 0.37, 0.6, 1, 1.25, 1.5, 2]) {
      const style = nodeZoomStyle(zoom);
      expect(style.width).toBe(`${zoom * 100}%`);
      expect(nodeGrid(NODE, cell(BASE), SCROLLBAR)).toEqual(grid);
    }
  }
});

test("o invólucro desfaz a escala do canvas: escala total 1 (mouse na célula certa)", () => {
  for (const zoom of [0.2, 0.6, 1, 1.5, 2]) {
    const style = nodeZoomStyle(zoom);
    const scale = Number(/scale\(([^)]+)\)/.exec(style.transform)?.[1]);
    expect(scale * zoom).toBeCloseTo(1, 12);
    expect(style.transformOrigin).toBe("0 0");
  }
});

test("fonte = base × zoom; a grade fixa nunca passa da caixa (borda direita inteira)", () => {
  const dpr = 2;
  const cell = webglCell(dpr);
  const grid = nodeGrid(NODE, cell(BASE), SCROLLBAR);
  // Em 100% a fonte é a base, sem recuo.
  const at1 = nodeFontSize(BASE, 1, grid, { w: NODE.w - SCROLLBAR, h: NODE.h }, cell);
  expect(at1).toBe(BASE);
  for (const zoom of [0.2, 0.37, 0.6, 0.75, 1.25, 1.5, 1.8, 2]) {
    // A caixa na tela: o nó × zoom (a barra de rolagem não escala).
    const avail = { w: NODE.w * zoom - SCROLLBAR, h: NODE.h * zoom };
    const size = nodeFontSize(BASE, zoom, grid, avail, cell);
    expect(size).toBeLessThanOrEqual(BASE * zoom);
    // Perto de base × zoom: recua só o que o arredondamento da célula exige.
    expect(size).toBeGreaterThan(BASE * zoom * 0.85);
    const c = cell(size);
    expect(grid.cols * c.w).toBeLessThanOrEqual(avail.w + 0.5);
    expect(grid.rows * c.h).toBeLessThanOrEqual(avail.h + 0.5);
  }
});

test("a roda é do terminal só com o nó selecionado; senão é do canvas", () => {
  expect(nodeTerminalClass(true).split(" ")).toContain("nowheel");
  expect(nodeTerminalClass(false).split(" ")).not.toContain("nowheel");
  for (const selected of [true, false]) {
    const cls = nodeTerminalClass(selected).split(" ");
    expect(cls).toContain("nodrag");
    expect(cls).toContain("nokey");
  }
});

test("sem o nó selecionado a roda vai ao canvas, inteira (pan, pinça, ⌘ + roda)", () => {
  const wheel = {
    deltaX: 3,
    deltaY: -40,
    deltaZ: 0,
    deltaMode: 0,
    clientX: 210,
    clientY: 330,
    screenX: 1210,
    screenY: 1330,
    ctrlKey: true,
    metaKey: false,
    shiftKey: false,
    altKey: false,
  };
  // Selecionado: a roda é do terminal (scrollback), nada vai ao canvas.
  expect(canvasWheel(wheel, true)).toBeNull();
  // Não selecionado: a cópia sobe até o canvas com tudo que o pan/zoom lê.
  expect(canvasWheel(wheel, false)).toEqual({ ...wheel, bubbles: true, cancelable: true });
});

test("faixa ao vivo no nó: desce e corta pela fração escondida, sem mudar o tamanho", () => {
  expect(nodeLiveClip(0)).toEqual({});
  expect(nodeLiveClip(-0.2)).toEqual({});
  expect(nodeLiveClip(0.25)).toEqual({ top: "25%", clipPath: "inset(0 0 25% 0)" });
  const clip = nodeLiveClip(0.5);
  expect("height" in clip || "width" in clip).toBe(false);
});
