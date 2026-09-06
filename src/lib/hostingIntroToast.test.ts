import { describe, expect, test } from "bun:test";

import {
  hostingIntroToastInput,
  shouldShowHostingIntroToast,
} from "./hostingIntroToast";

describe("shouldShowHostingIntroToast", () => {
  test("primeira transição para hospedado, nunca visto antes: mostra", () => {
    expect(shouldShowHostingIntroToast(false, true)).toBe(true);
  });

  test("já visto antes: nunca mostra de novo, mesmo hospedando agora", () => {
    expect(shouldShowHostingIntroToast(true, true)).toBe(false);
  });

  test("ninguém hospedado ainda: não mostra", () => {
    expect(shouldShowHostingIntroToast(false, false)).toBe(false);
  });

  test("já visto e ninguém hospedado: não mostra", () => {
    expect(shouldShowHostingIntroToast(true, false)).toBe(false);
  });
});

describe("hostingIntroToastInput", () => {
  test("é um toast informativo, sem ação nem sticky", () => {
    const input = hostingIntroToastInput((k) => k);
    expect(input.tone).toBe("info");
    expect(input.title).toBe("shimV2IntroToast");
    expect(input.action).toBeUndefined();
    expect(input.sticky).toBeUndefined();
  });
});
