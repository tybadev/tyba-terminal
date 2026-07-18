import { describe, expect, it } from "bun:test";
import type { SlotNode } from "./ipc";
import {
  MAX_RATIO,
  MIN_RATIO,
  countPanes,
  findPaneOfSlot,
  findSlotOfPane,
  removePane,
  setRatio,
  slotIds,
  splitPane,
  toPaneTree,
} from "./slotTree";

const leaf = (id: string, slot: string): SlotNode => ({
  type: "leaf",
  id,
  slot_id: slot,
});

const tree: SlotNode = {
  type: "split",
  id: "s1",
  split: "v",
  ratio: 0.6,
  first: leaf("p1", "slot-a"),
  second: {
    type: "split",
    id: "s2",
    split: "h",
    ratio: 0.4,
    first: leaf("p2", "slot-b"),
    second: leaf("p3", "slot-c"),
  },
};

describe("toPaneTree", () => {
  it("mapeia slot_id para session_id preservando a forma", () => {
    const pane = toPaneTree(tree);
    expect(pane).toEqual({
      type: "split",
      id: "s1",
      split: "v",
      ratio: 0.6,
      first: { type: "leaf", id: "p1", session_id: "slot-a" },
      second: {
        type: "split",
        id: "s2",
        split: "h",
        ratio: 0.4,
        first: { type: "leaf", id: "p2", session_id: "slot-b" },
        second: { type: "leaf", id: "p3", session_id: "slot-c" },
      },
    });
  });
});

describe("slotIds", () => {
  it("lista os slots na ordem da árvore", () => {
    expect(slotIds(tree)).toEqual(["slot-a", "slot-b", "slot-c"]);
  });
});

describe("findSlotOfPane / findPaneOfSlot", () => {
  it("faz a volta completa", () => {
    expect(findSlotOfPane(tree, "p2")).toBe("slot-b");
    expect(findPaneOfSlot(tree, "slot-b")).toBe("p2");
  });

  it("devolve null para id inexistente", () => {
    expect(findSlotOfPane(tree, "nope")).toBeNull();
    expect(findPaneOfSlot(tree, "nope")).toBeNull();
  });
});

describe("setRatio", () => {
  it("altera só o split alvo", () => {
    const next = setRatio(tree, "s2", 0.75);
    expect(next.type === "split" && next.ratio).toBe(0.6);
    const inner = next.type === "split" ? next.second : null;
    expect(inner?.type === "split" && inner.ratio).toBe(0.75);
  });

  it("prende o ratio na faixa aceita pelo core", () => {
    const tiny = setRatio(tree, "s1", 0.01);
    expect(tiny.type === "split" && tiny.ratio).toBe(MIN_RATIO);
    const huge = setRatio(tree, "s1", 0.99);
    expect(huge.type === "split" && huge.ratio).toBe(MAX_RATIO);
  });

  it("não muda nada quando o split não existe", () => {
    expect(setRatio(tree, "fantasma", 0.3)).toEqual(tree);
  });
});

describe("splitPane", () => {
  it("transforma a folha alvo num split meio a meio", () => {
    const next = splitPane(leaf("p1", "slot-a"), "p1", "h", "p9", "slot-z", "s9");
    expect(next).toEqual({
      type: "split",
      id: "s9",
      split: "h",
      ratio: 0.5,
      first: leaf("p1", "slot-a"),
      second: leaf("p9", "slot-z"),
    });
  });

  it("encontra a folha no fundo da árvore", () => {
    const next = splitPane(tree, "p3", "v", "p9", "slot-z", "s9");
    expect(countPanes(next)).toBe(4);
    expect(slotIds(next)).toEqual(["slot-a", "slot-b", "slot-c", "slot-z"]);
  });

  it("ignora pane que não existe", () => {
    expect(splitPane(tree, "fantasma", "v", "p9", "slot-z", "s9")).toEqual(tree);
  });
});

describe("removePane", () => {
  it("colapsa o split quando sobra um irmão só", () => {
    const next = removePane(tree, "p2");
    expect(next).toEqual({
      type: "split",
      id: "s1",
      split: "v",
      ratio: 0.6,
      first: leaf("p1", "slot-a"),
      second: leaf("p3", "slot-c"),
    });
  });

  it("devolve null ao remover o último pane", () => {
    expect(removePane(leaf("p1", "slot-a"), "p1")).toBeNull();
  });

  it("preserva a árvore quando o pane não existe", () => {
    expect(removePane(tree, "fantasma")).toEqual(tree);
  });
});
