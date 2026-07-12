import { describe, expect, test } from "bun:test";

import { SPINNER_FRAMES, windowTitle } from "./windowTitle";

describe("windowTitle", () => {
  test("rodando anima os frames braille na frente do título", () => {
    const first = windowTitle({
      base: "tyba",
      running: true,
      attention: false,
      frame: 0,
      reducedMotion: false,
    });
    const second = windowTitle({
      base: "tyba",
      running: true,
      attention: false,
      frame: 1,
      reducedMotion: false,
    });
    expect(first).toBe(`${SPINNER_FRAMES[0]} tyba`);
    expect(second).toBe(`${SPINNER_FRAMES[1]} tyba`);
    expect(
      windowTitle({
        base: "tyba",
        running: true,
        attention: false,
        frame: SPINNER_FRAMES.length,
        reducedMotion: false,
      }),
    ).toBe(first);
  });

  test("reduced motion usa glifo estático", () => {
    expect(
      windowTitle({
        base: "tyba",
        running: true,
        attention: false,
        frame: 3,
        reducedMotion: true,
      }),
    ).toBe("✳ tyba");
  });

  test("atenção não vista prefixa ponto; rodando vence atenção", () => {
    expect(
      windowTitle({
        base: "tyba",
        running: false,
        attention: true,
        frame: 0,
        reducedMotion: false,
      }),
    ).toBe("● tyba");
    expect(
      windowTitle({
        base: "tyba",
        running: true,
        attention: true,
        frame: 0,
        reducedMotion: false,
      }),
    ).toBe(`${SPINNER_FRAMES[0]} tyba`);
  });

  test("ocioso sem atenção é só o título", () => {
    expect(
      windowTitle({
        base: "Tyba",
        running: false,
        attention: false,
        frame: 0,
        reducedMotion: false,
      }),
    ).toBe("Tyba");
  });
});
