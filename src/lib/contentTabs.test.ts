import { describe, expect, it } from "bun:test";

import {
  containerFilesTarget,
  containerName,
  dirtyEditorsOf,
  editorView,
  eolOf,
  isEditorDirty,
  langGlyph,
  languageName,
  parseContentView,
  rememberContainerName,
  requestReveal,
  setEditorDirty,
  takeReveal,
} from "./contentTabs";

const SID = "6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b";
const OTHER = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

describe("parseContentView", () => {
  it("lê editor com caminho que tem dois-pontos", () => {
    expect(parseContentView(editorView(SID, "src/a:b.ts"))).toEqual({
      kind: "editor",
      session: SID,
      rel: "src/a:b.ts",
    });
  });

  it("lê logs de container", () => {
    expect(parseContentView("container-logs:abc123")).toEqual({
      kind: "container-logs",
      containerId: "abc123",
    });
  });

  it("lê diff da sessão", () => {
    expect(parseContentView(`session-diff:${SID}`)).toEqual({
      kind: "session-diff",
      session: SID,
    });
    expect(parseContentView("session-diff:curto")).toBeNull();
  });

  it("lê histórico do repo da sessão", () => {
    expect(parseContentView(`git-log:${SID}`)).toEqual({
      kind: "git-log",
      session: SID,
    });
    expect(parseContentView("git-log:curto")).toBeNull();
  });

  it("recusa views que não são de conteúdo ou estão quebradas", () => {
    expect(parseContentView("settings")).toBeNull();
    expect(parseContentView(null)).toBeNull();
    expect(parseContentView(`editor:${SID}`)).toBeNull();
    expect(parseContentView(`editor:${SID}:`)).toBeNull();
    expect(parseContentView("container-logs:")).toBeNull();
  });

  it("acha o container do painel lateral de arquivos", () => {
    expect(containerFilesTarget("container-files:abc")).toBe("abc");
    expect(containerFilesTarget("files:abc")).toBeNull();
    expect(containerFilesTarget(null)).toBeNull();
  });
});

describe("estado efêmero das abas", () => {
  it("revelar linha é consumido uma vez", () => {
    const view = editorView(SID, "a.ts");
    requestReveal(view, 8);
    expect(takeReveal(view)).toBe(8);
    expect(takeReveal(view)).toBeUndefined();
  });

  it("não salvo é por aba e filtra por sessão dona", () => {
    const a = editorView(SID, "a.ts");
    const b = editorView(OTHER, "b.ts");
    setEditorDirty(a, true);
    setEditorDirty(b, true);
    expect(isEditorDirty(a)).toBe(true);
    expect(dirtyEditorsOf([SID])).toEqual([a]);
    setEditorDirty(a, false);
    setEditorDirty(b, false);
    expect(dirtyEditorsOf([SID, OTHER])).toEqual([]);
  });

  it("nome do container cai para o id curto sem lembrança", () => {
    const id = "a1b2c3d4e5f6a1b2c3d4e5f6";
    expect(containerName(id)).toBe("a1b2c3d4e5f6");
    rememberContainerName(id, "api");
    expect(containerName(id)).toBe("api");
  });
});

describe("apresentação", () => {
  it("selo e nome de linguagem pela extensão", () => {
    expect(langGlyph("src/server.js").text).toBe("JS");
    expect(langGlyph("Main.TSX").text).toBe("TS");
    expect(langGlyph("Makefile").text).toBe("·");
    expect(languageName("src/server.js")).toBe("JavaScript");
    expect(languageName("x.zig")).toBe("ZIG");
    expect(languageName("Makefile")).toBe("Texto");
  });

  it("fim de linha", () => {
    expect(eolOf("a\nb")).toBe("LF");
    expect(eolOf("a\r\nb")).toBe("CRLF");
  });
});
