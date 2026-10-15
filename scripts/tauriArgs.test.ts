import { describe, expect, test } from "bun:test";
import { DEV_CONFIG, withDevConfig } from "./tauriArgs";

describe("withDevConfig", () => {
  test("tauri dev ganha a config de dev", () => {
    expect(withDevConfig(["dev"])).toEqual(["dev", "--config", DEV_CONFIG]);
    expect(withDevConfig(["dev", "--verbose"])).toEqual([
      "dev",
      "--config",
      DEV_CONFIG,
      "--verbose",
    ]);
  });

  test("config explícita vence", () => {
    expect(withDevConfig(["dev", "-c", "x.json"])).toEqual(["dev", "-c", "x.json"]);
    expect(withDevConfig(["dev", "--config=x.json"])).toEqual(["dev", "--config=x.json"]);
  });

  test("build e o resto passam intactos", () => {
    expect(withDevConfig(["build", "--target", "x"])).toEqual(["build", "--target", "x"]);
    expect(withDevConfig([])).toEqual([]);
  });
});
