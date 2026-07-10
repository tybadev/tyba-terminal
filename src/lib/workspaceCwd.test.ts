import { describe, expect, test } from "bun:test";

import type { SessionCwd, Workspace } from "./ipc";
import { resolveWorkspaceCwd, workspaceMatchDir } from "./workspaceCwd";

const cwd = (path: string, canonical = path): SessionCwd => ({
  cwd: path,
  canonical,
});

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
        active_pane: "p2",
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
    created_at: "",
    ...over,
  }) as Workspace;

describe("resolveWorkspaceCwd", () => {
  test("prefere o cwd da sessão do pane ativo", () => {
    const result = resolveWorkspaceCwd(workspace(), {
      "s-a": cwd("/repo/a"),
      "s-b": cwd("/repo/b"),
    });
    expect(result?.cwd).toBe("/repo/b");
  });

  test("cai na primeira folha com cwd quando o pane ativo não tem", () => {
    const result = resolveWorkspaceCwd(workspace(), { "s-a": cwd("/repo/a") });
    expect(result?.cwd).toBe("/repo/a");
  });

  test("sem nenhuma sessão com cwd devolve null", () => {
    expect(resolveWorkspaceCwd(workspace(), {})).toBeNull();
  });

  test("workspace sem tab devolve null", () => {
    expect(resolveWorkspaceCwd(workspace({ tabs: [] }), {})).toBeNull();
  });
});

describe("workspaceMatchDir — casa pelo físico, nunca pelo lógico", () => {
  test("usa o canonical do cwd da sessão", () => {
    const ws = workspace();
    const cwds = { "s-b": cwd("/tmp/proj", "/private/tmp/proj") };
    expect(workspaceMatchDir(ws, cwds)).toBe("/private/tmp/proj");
  });

  test("cai no repo_root quando ainda não há cwd de sessão", () => {
    const ws = workspace({ repo_root: "/private/tmp/proj" });
    expect(workspaceMatchDir(ws, {})).toBe("/private/tmp/proj");
  });

  test("sem cwd e sem repo_root devolve null", () => {
    expect(workspaceMatchDir(workspace(), {})).toBeNull();
  });
});

describe("display — mostra o que o usuário digitou", () => {
  test("o path exibido é o lógico, não o canonical", () => {
    const ws = workspace();
    const cwds = { "s-b": cwd("~/dev/proj", "/Volumes/Dev/proj") };
    expect(resolveWorkspaceCwd(ws, cwds)?.cwd).toBe("~/dev/proj");
    expect(workspaceMatchDir(ws, cwds)).toBe("/Volumes/Dev/proj");
  });
});
