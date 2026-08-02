import { describe, expect, it } from "bun:test";

import {
  inTextField,
  modeFor,
  pickedBlocks,
  selectBlock,
  type BlockSelection,
} from "./blockSelection";
import type { Block } from "./ipc";

const ORDER = [10, 20, 30, 40, 50];

const BLOCKS: Block[] = ORDER.map((id) => ({
  id,
  sessionId: "s1",
  command: `cmd${id}`,
  exitCode: 0,
  startedAtMs: 0,
  finishedAtMs: 0,
  lines: [],
  truncated: 0,
  cwd: null,
  altScreen: false,
}));

function pick(
  current: BlockSelection | null,
  id: number,
  mode: Parameters<typeof selectBlock>[3],
) {
  return selectBlock(current, ORDER, id, mode);
}

describe("modeFor", () => {
  it("reads shift as range and cmd/ctrl as toggle", () => {
    const plain = { shiftKey: false, metaKey: false, ctrlKey: false };
    expect(modeFor(plain)).toBe("replace");
    expect(modeFor({ ...plain, shiftKey: true })).toBe("range");
    expect(modeFor({ ...plain, metaKey: true })).toBe("toggle");
    expect(modeFor({ ...plain, ctrlKey: true })).toBe("toggle");
  });

  it("lets shift win over cmd", () => {
    expect(modeFor({ shiftKey: true, metaKey: true, ctrlKey: false })).toBe(
      "range",
    );
  });
});

describe("selectBlock", () => {
  it("marks a single block and makes it the anchor", () => {
    expect(pick(null, 30, "replace")).toEqual({ ids: [30], anchor: 30 });
  });

  it("clicking the only marked block again clears", () => {
    expect(pick({ ids: [30], anchor: 30 }, 30, "replace")).toBeNull();
  });

  it("clicking one of several collapses to it instead of clearing", () => {
    expect(pick({ ids: [10, 20, 30], anchor: 10 }, 30, "replace")).toEqual({
      ids: [30],
      anchor: 30,
    });
  });

  it("extends from the anchor, downward", () => {
    expect(pick({ ids: [20], anchor: 20 }, 40, "range")?.ids).toEqual([
      20, 30, 40,
    ]);
  });

  it("extends from the anchor, upward", () => {
    expect(pick({ ids: [40], anchor: 40 }, 20, "range")?.ids).toEqual([
      20, 30, 40,
    ]);
  });

  it("keeps the anchor so a second shift-click re-extends from it", () => {
    const first = pick({ ids: [20], anchor: 20 }, 50, "range");
    expect(pick(first, 30, "range")?.ids).toEqual([20, 30]);
  });

  it("marks only the clicked one when the anchor left the list", () => {
    // Retenção podou o bloco da âncora enquanto a seleção existia.
    expect(pick({ ids: [999], anchor: 999 }, 30, "range")?.ids).toEqual([30]);
  });

  it("range with nothing marked behaves like a plain click", () => {
    expect(pick(null, 30, "range")).toEqual({ ids: [30], anchor: 30 });
  });

  it("toggle adds and removes one at a time", () => {
    const added = pick({ ids: [10], anchor: 10 }, 40, "toggle");
    expect(added?.ids).toEqual([10, 40]);
    expect(pick(added, 10, "toggle")?.ids).toEqual([40]);
  });

  it("toggling the last one off clears", () => {
    expect(pick({ ids: [10], anchor: 10 }, 10, "toggle")).toBeNull();
  });
});

describe("inTextField", () => {
  const el = (tag: string, editable = false) =>
    ({
      tagName: tag,
      isContentEditable: editable,
    }) as unknown as Element;

  it("recognises the places where Esc belongs to whoever has the keyboard", () => {
    expect(inTextField(el("INPUT"))).toBe(true);
    expect(inTextField(el("TEXTAREA"))).toBe(true);
    expect(inTextField(el("DIV", true))).toBe(true);
  });

  it("lets Esc through elsewhere", () => {
    expect(inTextField(el("DIV"))).toBe(false);
    expect(inTextField(null)).toBe(false);
  });
});

describe("pickedBlocks", () => {
  const ids = (blocks: Block[]) => blocks.map((b) => b.id);

  it("returns the marked ones in list order, not click order", () => {
    expect(ids(pickedBlocks({ ids: [40, 10, 30], anchor: 40 }, BLOCKS))).toEqual(
      [10, 30, 40],
    );
  });

  it("drops ids that are no longer in the list", () => {
    // Retenção podou o bloco depois de marcado: some da cópia em vez de virar
    // um furo no meio dela.
    expect(ids(pickedBlocks({ ids: [10, 999], anchor: 10 }, BLOCKS))).toEqual([
      10,
    ]);
  });

  it("is empty without a selection", () => {
    expect(pickedBlocks(null, BLOCKS)).toEqual([]);
  });
});
