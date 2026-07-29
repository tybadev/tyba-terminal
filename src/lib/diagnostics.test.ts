import { describe, expect, it } from "bun:test";

import { formatDiagnostics, platformLabel, shortCommitDate } from "./diagnostics";
import type { BuildInfo } from "./ipc";

const info: BuildInfo = {
  version: "0.3.0",
  commit: "710d7f1",
  commit_date: "2026-07-28T12:00:00-03:00",
  os: "macos",
  arch: "arm64",
  webview: "WebKit 620.1.16",
};

describe("platformLabel", () => {
  it("usa o nome amigável do SO com a arch crua", () => {
    expect(platformLabel(info)).toBe("macOS · arm64");
    expect(platformLabel({ ...info, os: "linux", arch: "x86_64" })).toBe(
      "Linux · x86_64",
    );
  });

  it("não inventa nome pra SO desconhecido", () => {
    expect(platformLabel({ ...info, os: "freebsd" })).toBe("freebsd · arm64");
  });

  it("cai no traço quando não sabe nada", () => {
    expect(platformLabel({ ...info, os: "", arch: "" })).toBe("—");
  });
});

describe("shortCommitDate", () => {
  it("formata no locale pedido", () => {
    expect(shortCommitDate(info.commit_date, "pt-BR")).toBe("28/07/2026");
  });

  it("devolve vazio pra data ausente ou inválida", () => {
    expect(shortCommitDate("", "pt-BR")).toBe("");
    expect(shortCommitDate("nem data", "pt-BR")).toBe("");
  });
});

describe("formatDiagnostics", () => {
  it("monta o bloco completo", () => {
    expect(formatDiagnostics(info, "pt-BR")).toBe(
      [
        "TYBA 0.3.0",
        "commit 710d7f1 (28/07/2026)",
        "macOS · arm64",
        "webview WebKit 620.1.16",
        "locale pt-BR",
      ].join("\n"),
    );
  });

  it("omite commit e webview quando o build não tem git", () => {
    const bare: BuildInfo = {
      ...info,
      commit: "",
      commit_date: "",
      webview: "",
    };
    expect(formatDiagnostics(bare, "en")).toBe(
      ["TYBA 0.3.0", "macOS · arm64", "locale en"].join("\n"),
    );
  });

  it("mantém o commit quando só a data falta", () => {
    expect(formatDiagnostics({ ...info, commit_date: "" }, "pt-BR")).toContain(
      "commit 710d7f1\n",
    );
  });

  it("não vaza path, conta nem repo", () => {
    const out = formatDiagnostics(info, "pt-BR");
    expect(out).not.toContain("/Users/");
    expect(out).not.toContain("~");
    expect(out).not.toContain("tyba-terminal");
    expect(out.split("\n")).toHaveLength(5);
  });
});
