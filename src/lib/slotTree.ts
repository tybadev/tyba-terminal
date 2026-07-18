import type { PaneId, PaneNode, SlotId, SlotNode, SplitKind } from "./ipc";

export const MIN_RATIO = 0.1;
export const MAX_RATIO = 0.9;

export function toPaneTree(node: SlotNode): PaneNode {
  if (node.type === "leaf") {
    return { type: "leaf", id: node.id, session_id: node.slot_id };
  }
  return {
    type: "split",
    id: node.id,
    split: node.split,
    ratio: node.ratio,
    first: toPaneTree(node.first),
    second: toPaneTree(node.second),
  };
}

export function slotIds(node: SlotNode): SlotId[] {
  if (node.type === "leaf") return [node.slot_id];
  return [...slotIds(node.first), ...slotIds(node.second)];
}

export function findSlotOfPane(node: SlotNode, pane: PaneId): SlotId | null {
  if (node.type === "leaf") return node.id === pane ? node.slot_id : null;
  return findSlotOfPane(node.first, pane) ?? findSlotOfPane(node.second, pane);
}

export function findPaneOfSlot(node: SlotNode, slot: SlotId): PaneId | null {
  if (node.type === "leaf") return node.slot_id === slot ? node.id : null;
  return findPaneOfSlot(node.first, slot) ?? findPaneOfSlot(node.second, slot);
}

export function setRatio(
  node: SlotNode,
  split: PaneId,
  ratio: number,
): SlotNode {
  if (node.type === "leaf") return node;
  const clamped = Math.min(MAX_RATIO, Math.max(MIN_RATIO, ratio));
  if (node.id === split) return { ...node, ratio: clamped };
  return {
    ...node,
    first: setRatio(node.first, split, ratio),
    second: setRatio(node.second, split, ratio),
  };
}

export function splitPane(
  node: SlotNode,
  pane: PaneId,
  kind: SplitKind,
  newPaneId: PaneId,
  newSlotId: SlotId,
  newSplitId: PaneId,
): SlotNode {
  if (node.type === "leaf") {
    if (node.id !== pane) return node;
    return {
      type: "split",
      id: newSplitId,
      split: kind,
      ratio: 0.5,
      first: node,
      second: { type: "leaf", id: newPaneId, slot_id: newSlotId },
    };
  }
  return {
    ...node,
    first: splitPane(node.first, pane, kind, newPaneId, newSlotId, newSplitId),
    second: splitPane(node.second, pane, kind, newPaneId, newSlotId, newSplitId),
  };
}

export function removePane(node: SlotNode, pane: PaneId): SlotNode | null {
  if (node.type === "leaf") return node.id === pane ? null : node;
  const first = removePane(node.first, pane);
  const second = removePane(node.second, pane);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

export function countPanes(node: SlotNode): number {
  return node.type === "leaf" ? 1 : countPanes(node.first) + countPanes(node.second);
}
