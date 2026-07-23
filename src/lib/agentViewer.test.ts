import { describe, expect, it } from "bun:test";
import type { PaneNode } from "./ipc";
import { leafSessions, paneSession } from "./ipc";
import { computeRects, findAncestorSplit } from "./panes";

const tree: PaneNode = {
  type: "split",
  id: "split-1",
  split: "v",
  ratio: 0.62,
  first: { type: "leaf", id: "term-1", session_id: "sess-a" },
  second: { type: "agentviewer", id: "viewer-1", session_id: "sess-a" },
};

describe("helpers de pane com AgentViewer", () => {
  it("paneSession resolve a sessão tanto do terminal quanto do viewer", () => {
    expect(paneSession(tree, "term-1")).toBe("sess-a");
    expect(paneSession(tree, "viewer-1")).toBe("sess-a");
    expect(paneSession(tree, "ausente")).toBeNull();
  });

  it("leafSessions ignora o AgentViewer e conta só terminais", () => {
    expect(leafSessions(tree)).toEqual(["sess-a"]);
  });

  it("computeRects separa terminais de agent viewers e mantém o divisor", () => {
    const layout = computeRects(tree);
    expect(layout.panes.map((p) => p.pane)).toEqual(["term-1"]);
    expect(layout.agentViewers.map((p) => p.pane)).toEqual(["viewer-1"]);
    expect(layout.dividers).toHaveLength(1);
    const term = layout.panes[0];
    const viewer = layout.agentViewers[0];
    expect(term.w).toBeCloseTo(62, 5);
    expect(viewer.w).toBeCloseTo(38, 5);
  });

  it("findAncestorSplit encontra o split que contém o AgentViewer", () => {
    const found = findAncestorSplit(tree, "viewer-1", "v");
    expect(found).toEqual({ id: "split-1", ratio: 0.62 });
  });
});
