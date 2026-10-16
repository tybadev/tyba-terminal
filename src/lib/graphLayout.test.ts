import { describe, expect, test } from "bun:test";

import { freeSpot, nodeRect, outsideView, rightOf, type Rect } from "./graphLayout";

const size = { w: 200, h: 100 };

describe("freeSpot", () => {
  test("lugar vazio: fica onde se pediu", () => {
    expect(freeSpot({ x: 10.4, y: 20.6 }, size, [])).toEqual({ x: 10, y: 21 });
  });

  test("desce passando por baixo de quem está no caminho", () => {
    const occupied: Rect[] = [{ x: 0, y: 0, w: 300, h: 300 }];
    const at = freeSpot({ x: 50, y: 50 }, size, occupied, "down", 40);
    expect(at).toEqual({ x: 50, y: 340 });
  });

  test("desvia de vários na mesma coluna", () => {
    const occupied: Rect[] = [
      { x: 0, y: 0, w: 300, h: 100 },
      { x: 0, y: 140, w: 300, h: 100 },
    ];
    expect(freeSpot({ x: 0, y: 0 }, size, occupied, "down", 40)).toEqual({ x: 0, y: 280 });
  });

  test("right: anda para a direita (bloco de subagentes embaixo do pai)", () => {
    const occupied: Rect[] = [{ x: 0, y: 400, w: 380, h: 300 }];
    const at = freeSpot({ x: 0, y: 420 }, size, occupied, "right", 40);
    expect(at).toEqual({ x: 420, y: 420 });
  });

  test("nunca cobre nenhum retângulo ocupado", () => {
    const occupied: Rect[] = [];
    for (let i = 0; i < 20; i++) {
      const at = freeSpot({ x: 0, y: 0 }, size, occupied);
      for (const r of occupied) {
        const apart =
          at.x >= r.x + r.w || r.x >= at.x + size.w || at.y >= r.y + r.h || r.y >= at.y + size.h;
        expect(apart).toBe(true);
      }
      occupied.push({ ...at, ...size });
    }
  });
});

describe("nodeRect e rightOf", () => {
  test("o tamanho medido vale sobre o do modelo", () => {
    expect(
      nodeRect({ position: { x: 1, y: 2 }, width: 30, data: { node: { size: { w: 9, h: 8 } } } }),
    ).toEqual({ x: 1, y: 2, w: 30, h: 8 });
  });

  test("vizinho nasce à direita, alinhado pelo topo", () => {
    expect(rightOf({ x: 10, y: 20, w: 100, h: 50 }, 40)).toEqual({ x: 150, y: 20 });
  });
});

describe("outsideView (minimapa liga sozinho)", () => {
  const view = { x: 0, y: 0, zoom: 1 };
  const area = { w: 1000, h: 800 };

  test("tudo à vista: não", () => {
    expect(outsideView([{ x: 10, y: 10, w: 200, h: 100 }], view, area)).toBe(false);
  });

  test("um nó passa da borda: sim", () => {
    expect(outsideView([{ x: 900, y: 10, w: 200, h: 100 }], view, area)).toBe(true);
  });

  test("o zoom muda a área visível", () => {
    const far = [{ x: 1500, y: 10, w: 200, h: 100 }];
    expect(outsideView(far, view, area)).toBe(true);
    expect(outsideView(far, { x: 0, y: 0, zoom: 0.5 }, area)).toBe(false);
  });

  test("área ainda sem medida: não", () => {
    expect(outsideView([{ x: 5000, y: 0, w: 1, h: 1 }], view, { w: 0, h: 0 })).toBe(false);
  });
});
