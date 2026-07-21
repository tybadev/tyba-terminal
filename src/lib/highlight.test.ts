import { describe, expect, it } from "bun:test";

import { langForFence, langForFile } from "./highlight";

describe("langForFile", () => {
  it("resolves by extension", () => {
    expect(langForFile("main.ts")).toBe("typescript");
    expect(langForFile("lib.rs")).toBe("rust");
    expect(langForFile("config.toml")).toBe("toml");
    expect(langForFile("app.py")).toBe("python");
    expect(langForFile("Main.java")).toBe("java");
    expect(langForFile("run.sh")).toBe("shellscript");
  });

  it("resolves well-known filenames without an extension", () => {
    expect(langForFile("Dockerfile")).toBe("dockerfile");
    expect(langForFile("Dockerfile.dev")).toBe("dockerfile");
    expect(langForFile("Makefile")).toBe("make");
    expect(langForFile("docker-compose.yml")).toBe("yaml");
  });

  it("returns null for unknown files", () => {
    expect(langForFile("mystery")).toBeNull();
    expect(langForFile("data.bin")).toBeNull();
  });
});

describe("langForFence", () => {
  it("maps fence aliases to shiki grammar ids", () => {
    expect(langForFence("ts")).toBe("typescript");
    expect(langForFence("sh")).toBe("shellscript");
    expect(langForFence("dockerfile")).toBe("dockerfile");
    expect(langForFence("rust")).toBe("rust");
    expect(langForFence("YAML")).toBe("yaml");
  });

  it("returns null for an unknown fence language", () => {
    expect(langForFence("brainfuck")).toBeNull();
  });
});
