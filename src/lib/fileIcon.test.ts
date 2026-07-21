import { describe, expect, it } from "bun:test";
import {
  File,
  FileCode,
  FileIni,
  FileLock,
  FileRs,
  FileTs,
} from "@phosphor-icons/react";

import { fileIcon } from "./fileIcon";

describe("fileIcon", () => {
  it("maps by extension", () => {
    expect(fileIcon("main.ts")).toBe(FileTs);
    expect(fileIcon("lib.rs")).toBe(FileRs);
  });

  it("maps well-known filenames by name", () => {
    expect(fileIcon("Cargo.lock")).toBe(FileLock);
    expect(fileIcon("Dockerfile")).toBe(FileCode);
    expect(fileIcon("docker-compose.yml")).toBe(FileCode);
    expect(fileIcon("Makefile")).toBe(FileCode);
  });

  it("routes dotfiles to the config icon", () => {
    expect(fileIcon(".env")).toBe(FileIni);
    expect(fileIcon(".gitignore")).toBe(FileIni);
  });

  it("falls back to a generic file", () => {
    expect(fileIcon("mystery")).toBe(File);
    expect(fileIcon("data.weirdext")).toBe(File);
  });
});
