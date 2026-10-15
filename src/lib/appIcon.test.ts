import { describe, expect, it } from "bun:test";

import { APP_ICONS, APP_ICON_NAMES, isAppIconId, radioStep } from "./appIcon";

describe("isAppIconId", () => {
  it("aceita só os ids da lista", () => {
    for (const id of APP_ICONS) expect(isAppIconId(id)).toBe(true);
    for (const bad of ["", "Aurora", "neon", "../aurora", null, 3]) {
      expect(isAppIconId(bad)).toBe(false);
    }
  });

  it("toda variante tem nome", () => {
    for (const id of APP_ICONS) expect(APP_ICON_NAMES[id]).toBeTruthy();
  });
});

describe("radioStep", () => {
  it("anda com as setas e dá a volta", () => {
    expect(radioStep(0, "ArrowRight", 8)).toBe(1);
    expect(radioStep(7, "ArrowDown", 8)).toBe(0);
    expect(radioStep(0, "ArrowLeft", 8)).toBe(7);
    expect(radioStep(3, "ArrowUp", 8)).toBe(2);
  });

  it("Home e End vão às pontas", () => {
    expect(radioStep(4, "Home", 8)).toBe(0);
    expect(radioStep(4, "End", 8)).toBe(7);
  });

  it("ignora o resto", () => {
    expect(radioStep(4, "Enter", 8)).toBeNull();
    expect(radioStep(4, "a", 8)).toBeNull();
  });
});
