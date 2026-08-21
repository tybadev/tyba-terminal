import { describe, expect, test } from "bun:test";

import type { RepoSnapshot } from "./ipc";
import {
  DEFAULT_TOOLBAR,
  parseToolbarPref,
  sessionRepoDir,
  snapshotForDir,
  toolbarBranchChip,
} from "./repoSnapshots";

const snap = (root: string, branch = "main"): RepoSnapshot => ({
  root,
  branch,
  status: null,
  ahead: null,
  behind: null,
});

describe("snapshotForDir", () => {
  test("exact root matches", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/home/u/repo")?.root).toBe("/home/u/repo");
  });

  test("subdirectory matches its root", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/home/u/repo/src/deep")?.root).toBe(
      "/home/u/repo",
    );
  });

  test("sibling with shared prefix does not match", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/home/u/repo-other")).toBeUndefined();
  });

  test("nested roots pick the longest", () => {
    const s = {
      "/home/u/outer": snap("/home/u/outer"),
      "/home/u/outer/inner": snap("/home/u/outer/inner"),
    };
    expect(snapshotForDir(s, "/home/u/outer/inner/src")?.root).toBe(
      "/home/u/outer/inner",
    );
  });

  test("unrelated dir matches nothing", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/tmp/elsewhere")).toBeUndefined();
  });
});

describe("parseToolbarPref", () => {
  test("null and garbage fall back to default", () => {
    expect(parseToolbarPref(null)).toEqual(DEFAULT_TOOLBAR);
    expect(parseToolbarPref("not json")).toEqual(DEFAULT_TOOLBAR);
    expect(parseToolbarPref("42")).toEqual(DEFAULT_TOOLBAR);
  });

  test("newer version than supported falls back to default", () => {
    expect(parseToolbarPref('{"version": 2, "enabled": false}')).toEqual(
      DEFAULT_TOOLBAR,
    );
  });

  test("unknown chip ids are dropped and missing ones heal into default zones", () => {
    const pref = parseToolbarPref(
      '{"version":1,"enabled":true,"left":["diffCount","voiceInput","hack"],"right":["clock"],"hidden":[]}',
    );
    expect(pref.left).toEqual(["diffCount", "reviewDiff"]);
    expect(pref.right).toEqual(["clock", "cwd", "branch", "aheadBehind"]);
    expect(pref.hidden).toEqual([]);
  });

  test("missing chip arrays fall back to the default sides", () => {
    const pref = parseToolbarPref('{"version":1,"enabled":true}');
    expect(pref.left).toEqual(DEFAULT_TOOLBAR.left);
    expect(pref.right).toEqual(DEFAULT_TOOLBAR.right);
  });

  test("chip escondido pelo usuário continua escondido; ausente é re-adicionado", () => {
    const pref = parseToolbarPref(
      '{"version":1,"enabled":true,"left":[],"right":["clock"],"hidden":["diffCount","cwd","branch","aheadBehind","reviewDiff"]}',
    );
    expect(pref.left).toEqual([]);
    expect(pref.right).toEqual(["clock"]);
    expect(pref.hidden).toEqual([
      "diffCount",
      "cwd",
      "branch",
      "aheadBehind",
      "reviewDiff",
    ]);
    const healed = parseToolbarPref(
      '{"version":1,"enabled":true,"left":[],"right":["clock"],"hidden":[]}',
    );
    expect(healed.left).toEqual(["diffCount", "reviewDiff"]);
    expect(healed.right).toEqual(["clock", "cwd", "branch", "aheadBehind"]);
  });

  test("enabled false is preserved", () => {
    const pref = parseToolbarPref(
      '{"version":1,"enabled":false,"left":[],"right":[],"hidden":[]}',
    );
    expect(pref.enabled).toBe(false);
  });
});

describe("sessionRepoDir", () => {
  test("worktree ganha do cwd — é o que o core resolve primeiro", () => {
    // O caso do defeito: a barra lia o repo principal e o checkout caía no
    // worktree do agente.
    expect(
      sessionRepoDir({
        worktree: "/home/u/.tyba/wt/task",
        alive: true,
        cwd: "/home/u/repo",
      }),
    ).toBe("/home/u/.tyba/wt/task");
  });

  test("worktree vale mesmo com a sessão encerrada", () => {
    // `session_repo_context` sai pelo worktree antes de olhar o processo.
    expect(
      sessionRepoDir({
        worktree: "/home/u/.tyba/wt/task",
        alive: false,
        cwd: null,
      }),
    ).toBe("/home/u/.tyba/wt/task");
  });

  test("sem worktree, o cwd da sessão viva", () => {
    expect(
      sessionRepoDir({ worktree: null, alive: true, cwd: "/home/u/repo/src" }),
    ).toBe("/home/u/repo/src");
  });

  test("sessão morta sem worktree não tem repositório", () => {
    // O core devolveria "a sessão não tem um processo ativo": abrir o seletor
    // aqui seria abrir só para mostrar erro.
    expect(
      sessionRepoDir({ worktree: null, alive: false, cwd: "/home/u/repo" }),
    ).toBe(null);
  });

  test("sessão viva sem cwd conhecido também não", () => {
    expect(sessionRepoDir({ worktree: null, alive: true, cwd: null })).toBe(
      null,
    );
  });
});

describe("toolbarBranchChip", () => {
  const WT = "/home/u/.tyba/wt/task";
  const REPO = "/home/u/repo";
  const live = {
    id: "s1",
    worktree: WT,
    alive: true,
    cwd: WT,
  };

  test("worktree com snapshot: rótulo do próprio repo e checkout na sessão", () => {
    expect(
      toolbarBranchChip({
        session: live,
        workspaceDir: REPO,
        snapshots: { [WT]: snap(WT, "task/x"), [REPO]: snap(REPO, "main") },
      }),
    ).toEqual({ state: "known", label: "task/x", sessionId: "s1" });
  });

  test("worktree sem snapshot vira 'não sei', não vira chip ausente", () => {
    // O caso comum: `watched_repo_roots` só registra o cwd de sessão em
    // `Running`, e o worktree continua existindo depois que a sessão morre.
    // Sumir o chip nesse cenário lê como defeito — o usuário não tem como
    // saber que foi de propósito.
    expect(
      toolbarBranchChip({
        session: { ...live, alive: false, cwd: null },
        workspaceDir: REPO,
        snapshots: { [REPO]: snap(REPO, "main") },
      }),
    ).toEqual({ state: "unknown" });
  });

  test("o 'não sei' nunca empresta o rótulo do workspace", () => {
    // A regressão que este chip já teve: dizer `main` do repo principal
    // enquanto o checkout cairia dentro do worktree do agente.
    const chip = toolbarBranchChip({
      session: { ...live, alive: false, cwd: null },
      workspaceDir: REPO,
      snapshots: { [REPO]: snap(REPO, "main") },
    });
    expect(chip).not.toEqual({
      state: "known",
      label: "main",
      sessionId: null,
    });
  });

  test("sessão viva fora de repositório git não ganha chip nenhum", () => {
    // Sem worktree, um dir sem snapshot é indistinguível de "não é git": o
    // chip ausente ali é a resposta certa, e inventar "não sei" seria ruído.
    expect(
      toolbarBranchChip({
        session: { id: "s2", worktree: null, alive: true, cwd: "/tmp/scratch" },
        workspaceDir: null,
        snapshots: { [REPO]: snap(REPO) },
      }),
    ).toBeNull();
  });

  test("sem sessão, o rótulo é o do workspace e o chip não age", () => {
    expect(
      toolbarBranchChip({
        session: null,
        workspaceDir: "/home/u/repo/src",
        snapshots: { [REPO]: snap(REPO, "main") },
      }),
    ).toEqual({ state: "known", label: "main", sessionId: null });
  });

  test("sessão encerrada sem worktree cai no workspace", () => {
    // Aqui não há repositório PRÓPRIO para contradizer o do workspace.
    expect(
      toolbarBranchChip({
        session: { id: "s3", worktree: null, alive: false, cwd: REPO },
        workspaceDir: REPO,
        snapshots: { [REPO]: snap(REPO, "main") },
      }),
    ).toEqual({ state: "known", label: "main", sessionId: null });
  });

  test("sem sessão e sem snapshot do workspace, nada", () => {
    expect(
      toolbarBranchChip({
        session: null,
        workspaceDir: "/home/u/outro",
        snapshots: { [REPO]: snap(REPO) },
      }),
    ).toBeNull();
  });
});
