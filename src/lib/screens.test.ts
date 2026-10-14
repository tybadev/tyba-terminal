import { describe, expect, test } from "bun:test";

import type { FilesScreenState, Workspace } from "./ipc";
import {
  activeFile,
  allScreenEditors,
  escClosesScreen,
  recentFiles,
  screenOf,
  sessionFiles,
  shouldPin,
  sideSlotView,
} from "./screens";

const A = "11111111-1111-1111-1111-111111111111";
const B = "22222222-2222-2222-2222-222222222222";
const ed = (s: string, rel: string) => `editor:${s}:${rel}`;

const files = (over: Partial<FilesScreenState> = {}): FilesScreenState => ({
  section: "files",
  tabs: [],
  active: null,
  recent: [],
  ...over,
});

describe("screenOf", () => {
  test("git, arquivos e docker são telas cheias", () => {
    expect(screenOf(`files:${A}`)).toEqual({ kind: "files", session: A });
    expect(screenOf(`docker:${A}`)).toEqual({ kind: "docker", session: A });
    expect(screenOf(`git:${A}`)).toEqual({ kind: "git", session: A });
  });
  test("o resto continua painel lateral", () => {
    expect(screenOf(`history:${A}`)).toBeNull();
    expect(screenOf("agent-queue")).toBeNull();
    expect(screenOf(null)).toBeNull();
    expect(sideSlotView(`history:${A}`)).toBe(`history:${A}`);
    expect(sideSlotView(`files:${A}`)).toBeNull();
    expect(sideSlotView(`docker:${A}`)).toBeNull();
  });
});

describe("abas internas da tela de Arquivos", () => {
  const state = files({
    tabs: [
      { view: ed(A, "src/a.ts"), preview: false },
      { view: ed(B, "b.rs"), preview: false },
      { view: ed(A, "README.md"), preview: true },
    ],
    active: ed(B, "b.rs"),
    recent: [ed(A, "README.md"), ed(B, "b.rs"), ed(A, "src/a.ts")],
  });

  test("só as da sessão da tela, na ordem do core", () => {
    expect(sessionFiles(state, A).map((f) => [f.rel, f.preview])).toEqual([
      ["src/a.ts", false],
      ["README.md", true],
    ]);
  });

  test("ativa de outra sessão cai na primeira desta", () => {
    expect(activeFile(state, A)).toBe(ed(A, "src/a.ts"));
    expect(activeFile(state, B)).toBe(ed(B, "b.rs"));
    expect(activeFile(files(), A)).toBeNull();
  });

  test("recentes da sessão como caminho relativo", () => {
    expect(recentFiles(state, A)).toEqual(["README.md", "src/a.ts"]);
  });
});

test("editores de todos os workspaces ficam montados, sem repetir", () => {
  const ws = (id: string, tabs: FilesScreenState["tabs"]) =>
    ({
      id,
      screens: { files: files({ tabs }), docker: { section: "containers", container: null, pane: "logs" } },
    }) as unknown as Workspace;
  const out = allScreenEditors([
    ws("w1", [{ view: ed(A, "a.ts"), preview: true }, { view: "lixo", preview: false }]),
    ws("w2", [{ view: ed(B, "b.ts"), preview: false }, { view: ed(A, "a.ts"), preview: false }]),
  ]);
  expect(out.map((e) => [e.workspace, e.rel])).toEqual([
    ["w1", "a.ts"],
    ["w2", "b.ts"],
  ]);
});

test("editar a pré-visualização a fixa", () => {
  expect(shouldPin(true, true)).toBe(true);
  expect(shouldPin(true, false)).toBe(false);
  expect(shouldPin(false, true)).toBe(false);
});

describe("Esc fecha a tela", () => {
  const ev = (tagName: string, extra: Partial<{ isContentEditable: boolean; defaultPrevented: boolean }> = {}) => ({
    key: "Escape",
    defaultPrevented: extra.defaultPrevented ?? false,
    target: { tagName, isContentEditable: extra.isContentEditable ?? false },
  });
  test("fora de campo fecha", () => {
    expect(escClosesScreen(ev("DIV"))).toBe(true);
    expect(escClosesScreen(ev("BUTTON"))).toBe(true);
  });
  test("em campo, editor ou terminal não fecha", () => {
    expect(escClosesScreen(ev("INPUT"))).toBe(false);
    // O xterm digita num TEXTAREA: Esc é do vim, não da tela.
    expect(escClosesScreen(ev("TEXTAREA"))).toBe(false);
    expect(escClosesScreen(ev("DIV", { isContentEditable: true }))).toBe(false);
    expect(escClosesScreen(ev("DIV", { defaultPrevented: true }))).toBe(false);
    expect(escClosesScreen({ ...ev("DIV"), key: "Enter" })).toBe(false);
  });
});
