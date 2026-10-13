import { describe, expect, test } from "bun:test";

import type { FileDiff, SessionDiff } from "./ipc";
import {
  commitPlan,
  gitChangeRows,
  pruneChosen,
  toggleUntracked,
  upstreamLine,
} from "./gitPanel";
import { isSensitivePath } from "./sensitivePaths";

const file = (
  path: string,
  status: FileDiff["status"],
  insertions: number | null,
  deletions: number | null,
  old_path: string | null = null,
): FileDiff => ({ path, old_path, status, insertions, deletions });

const diff = (
  staged: FileDiff[],
  unstaged: FileDiff[],
  untracked: string[] = [],
): SessionDiff => ({
  root: "/repo",
  base_ref: "abc",
  commits: [],
  files: [],
  staged_files: staged,
  unstaged_files: unstaged,
  untracked,
  commits_truncated: false,
});

describe("gitChangeRows", () => {
  test("rastreados numa lista, untracked em outra; totais somam tudo", () => {
    const out = gitChangeRows(
      diff(
        [file("src/server.js", "added", 30, 0)],
        [
          file("index.js", "modified", 12, 3),
          file("old.js", "deleted", 0, 4),
          file("novo.txt", "added", 5, 0),
        ],
        ["novo.txt"],
      ),
    );
    expect(out.rows.map((r) => `${r.letter} ${r.path}`)).toEqual([
      "M index.js",
      "D old.js",
      "A src/server.js",
    ]);
    expect(out.untracked.map((r) => r.path)).toEqual(["novo.txt"]);
    expect(out.insertions).toBe(47);
    expect(out.deletions).toBe(7);
  });

  test("arquivo nas duas listas vira uma linha com +/− somados", () => {
    const out = gitChangeRows(
      diff([file("a.ts", "added", 5, 0)], [file("a.ts", "modified", 2, 1)]),
    );
    expect(out.rows).toEqual([
      {
        path: "a.ts",
        letter: "M",
        tone: "amber",
        insertions: 7,
        deletions: 1,
        sensitive: false,
      },
    ]);
  });

  test("marca o que parece segredo", () => {
    const out = gitChangeRows(diff([], [file(".env", "added", 1, 0)], [".env"]));
    expect(out.untracked[0].sensitive).toBe(true);
  });

  test("sem diff, vazio", () => {
    expect(gitChangeRows(null)).toEqual({
      rows: [],
      untracked: [],
      insertions: 0,
      deletions: 0,
    });
  });
});

describe("commitPlan", () => {
  test("com algo preparado, commita só o preparado", () => {
    expect(
      commitPlan(diff([file("a", "added", 1, 0)], [file("b", "modified", 1, 0)])),
    ).toEqual({ mode: "staged", stage: [] });
  });

  test("sem nada preparado: só rastreados (git add -u), com o caminho antigo do rename", () => {
    expect(
      commitPlan(
        diff(
          [],
          [
            file("b", "modified", 1, 0),
            file("gone", "deleted", 0, 3),
            file("new", "renamed", 0, 0, "old"),
            file("solto.txt", "added", 2, 0),
          ],
          ["solto.txt"],
        ),
      ),
    ).toEqual({ mode: "tracked", stage: ["b", "gone", "new", "old"] });
  });

  test("untracked só entra marcado — nos dois modos", () => {
    const d = diff([], [file("solto.txt", "added", 2, 0)], ["solto.txt"]);
    expect(commitPlan(d).mode).toBe("nothing");
    expect(commitPlan(d, new Set(["solto.txt"]))).toEqual({
      mode: "tracked",
      stage: ["solto.txt"],
    });
    const staged = diff(
      [file("a", "modified", 1, 0)],
      [file("solto.txt", "added", 2, 0)],
      ["solto.txt"],
    );
    expect(commitPlan(staged, new Set(["solto.txt"]))).toEqual({
      mode: "staged",
      stage: ["solto.txt"],
    });
  });

  test("escolha que não é untracked é ignorada", () => {
    const d = diff([], [file("b", "modified", 1, 0)]);
    expect(commitPlan(d, new Set(["b", "fantasma"])).stage).toEqual(["b"]);
  });

  test("limpo, nada a fazer", () => {
    expect(commitPlan(diff([], [])).mode).toBe("nothing");
    expect(commitPlan(null).mode).toBe("nothing");
  });
});

describe("toggleUntracked", () => {
  test("arquivo comum marca e desmarca livre", () => {
    const on = toggleUntracked(new Set(), "a.txt", false);
    expect(on).toEqual({ chosen: new Set(["a.txt"]), needsConfirm: false });
    expect(toggleUntracked(on.chosen, "a.txt", false).chosen.size).toBe(0);
  });

  test("segredo pede confirmação e só marca confirmado", () => {
    const asked = toggleUntracked(new Set(), "config/.env.local", false);
    expect(asked).toEqual({ chosen: new Set(), needsConfirm: true });
    const done = toggleUntracked(new Set(), "config/.env.local", true);
    expect(done.chosen.has("config/.env.local")).toBe(true);
  });

  test("desmarcar segredo nunca pede confirmação", () => {
    const off = toggleUntracked(new Set(["id_rsa"]), "id_rsa", false);
    expect(off).toEqual({ chosen: new Set(), needsConfirm: false });
  });
});

describe("pruneChosen", () => {
  test("tira o que não é mais untracked", () => {
    const d = diff([], [], ["a"]);
    expect(pruneChosen(new Set(["a", "b"]), d)).toEqual(new Set(["a"]));
    expect(pruneChosen(new Set(["a"]), null).size).toBe(0);
  });
});

describe("isSensitivePath", () => {
  test("padrões de segredo", () => {
    for (const p of [
      ".env",
      ".env.local",
      "app/.ENV.production",
      "certs/server.pem",
      "tls.key",
      "id_rsa",
      "id_rsa.pub",
      "aws-credentials.json",
      "cert.p12",
      ".npmrc",
      ".pypirc",
    ]) {
      expect(isSensitivePath(p)).toBe(true);
    }
  });

  test("arquivo comum não", () => {
    for (const p of ["src/env.ts", "keyboard.ts", "README.md", "environment.md", "monkey.js"]) {
      expect(isSensitivePath(p)).toBe(false);
    }
  });
});

describe("upstreamLine", () => {
  test("cada combinação de à frente/atrás", () => {
    expect(upstreamLine(null, null).key).toBe("gitPanelNoUpstream");
    expect(upstreamLine(0, 0).key).toBe("gitPanelUpToDate");
    expect(upstreamLine(2, 0)).toEqual({ key: "gitPanelAhead", ahead: 2, behind: 0 });
    expect(upstreamLine(0, 3).key).toBe("gitPanelBehind");
    expect(upstreamLine(1, 1).key).toBe("gitPanelAheadBehind");
  });
});
