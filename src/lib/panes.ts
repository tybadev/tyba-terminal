import type { PaneId, PaneNode, SessionId, SplitKind } from "./ipc";

export interface PaneRect {
  pane: PaneId;
  session: SessionId;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface DividerRect {
  split: PaneId;
  kind: SplitKind;
  at: number;
  start: number;
  length: number;
  crossStart: number;
  crossLength: number;
}

export interface PaneLayout {
  panes: PaneRect[];
  dividers: DividerRect[];
}

export function computeRects(root: PaneNode): PaneLayout {
  const panes: PaneRect[] = [];
  const dividers: DividerRect[] = [];

  const walk = (n: PaneNode, x: number, y: number, w: number, h: number) => {
    if (n.type === "leaf") {
      panes.push({ pane: n.id, session: n.session_id, x, y, w, h });
      return;
    }
    if (n.split === "v") {
      const first = w * n.ratio;
      walk(n.first, x, y, first, h);
      dividers.push({
        split: n.id,
        kind: "v",
        at: x + first,
        start: x,
        length: w,
        crossStart: y,
        crossLength: h,
      });
      walk(n.second, x + first, y, w - first, h);
    } else {
      const first = h * n.ratio;
      walk(n.first, x, y, w, first);
      dividers.push({
        split: n.id,
        kind: "h",
        at: y + first,
        start: y,
        length: h,
        crossStart: x,
        crossLength: w,
      });
      walk(n.second, x, y + first, w, h - first);
    }
  };

  walk(root, 0, 0, 100, 100);
  return { panes, dividers };
}

export function findAncestorSplit(
  root: PaneNode,
  pane: PaneId,
  kind: SplitKind,
): { id: PaneId; ratio: number } | null {
  let found: { id: PaneId; ratio: number } | null = null;

  const contains = (n: PaneNode): boolean => {
    if (n.type === "leaf") return n.id === pane;
    const inside = n.id === pane || contains(n.first) || contains(n.second);
    if (inside && n.split === kind && !found) {
      found = { id: n.id, ratio: n.ratio };
    }
    return inside;
  };

  contains(root);
  return found;
}
