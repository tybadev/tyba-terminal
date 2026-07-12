import { describe, expect, test } from "bun:test";

import type { Workspace } from "./ipc";
import { findSessionLocation } from "./sessionLocation";

const workspace = (over: Partial<Workspace> = {}): Workspace =>
  ({
    id: "w1",
    name: "w",
    repo_root: null,
    color: null,
    group: null,
    kind: "user",
    active_tab: "t1",
    tabs: [
      {
        id: "t1",
        title: null,
        view: null,
        active_pane: "p1",
        root: { type: "leaf", id: "p1", session_id: "s-a" },
        created_at: "",
      },
    ],
    created_at: "",
    ...over,
  }) as Workspace;

describe("findSessionLocation", () => {
  test("acha a sessão numa folha simples", () => {
    expect(findSessionLocation([workspace()], "s-a")).toEqual({
      workspaceId: "w1",
      tabId: "t1",
    });
  });

  test("acha a sessão dentro de um split", () => {
    const split = workspace({
      id: "w2",
      tabs: [
        {
          id: "t2",
          title: null,
          view: null,
          active_pane: "p1",
          root: {
            type: "split",
            id: "s1",
            split: "v",
            ratio: 0.5,
            first: { type: "leaf", id: "p1", session_id: "s-a" },
            second: { type: "leaf", id: "p2", session_id: "s-b" },
          },
          created_at: "",
        },
      ],
    });
    expect(findSessionLocation([split], "s-b")).toEqual({
      workspaceId: "w2",
      tabId: "t2",
    });
  });

  test("procura em vários workspaces até achar", () => {
    const w1 = workspace({ id: "w1" });
    const w2 = workspace({
      id: "w2",
      tabs: [
        {
          id: "t2",
          title: null,
          view: null,
          active_pane: "p2",
          root: { type: "leaf", id: "p2", session_id: "s-b" },
          created_at: "",
        },
      ],
    });
    expect(findSessionLocation([w1, w2], "s-b")).toEqual({
      workspaceId: "w2",
      tabId: "t2",
    });
  });

  test("sessão inexistente devolve null", () => {
    expect(findSessionLocation([workspace()], "ghost")).toBeNull();
  });

  test("tab sem root não quebra a busca", () => {
    const w = workspace({
      tabs: [
        { id: "t1", title: null, view: "settings", active_pane: null, root: null, created_at: "" },
      ],
    });
    expect(findSessionLocation([w], "s-a")).toBeNull();
  });
});
