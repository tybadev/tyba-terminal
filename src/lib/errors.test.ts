import { describe, expect, test } from "bun:test";

import { translateError } from "./errors";

const DICT: Record<string, string> = {
  "error.merge.base_dirty":
    "A branch base ({{branch}}) tem trabalho não-commitado.",
  "error.push.protected_branch": "Push para {{branch}} é recusado.",
  "error.unknown": "Erro inesperado: {{detail}}",
};

function fakeT(key: string, options?: Record<string, unknown>): string {
  const template = DICT[key];
  if (template === undefined) return key;
  return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => {
    const value = options?.[name];
    return value === undefined ? `{{${name}}}` : String(value);
  });
}

describe("translateError", () => {
  test("maps an {code, params} object to its interpolated key", () => {
    const err = { code: "merge.base_dirty", params: { branch: "main" } };
    expect(translateError(err, fakeT)).toBe(
      "A branch base (main) tem trabalho não-commitado.",
    );
  });

  test("interpolates every param", () => {
    const err = { code: "push.protected_branch", params: { branch: "master" } };
    expect(translateError(err, fakeT)).toBe("Push para master é recusado.");
  });

  test("passes a plain string through untouched", () => {
    expect(translateError("git push falhou: timeout", fakeT)).toBe(
      "git push falhou: timeout",
    );
  });

  test("falls back to error.unknown with detail for an unknown code", () => {
    const err = { code: "merge.void", params: { detail: "boom" } };
    expect(translateError(err, fakeT)).toBe("Erro inesperado: boom");
  });

  test("falls back to error.unknown with the code when there is no detail", () => {
    const err = { code: "merge.void", params: {} };
    expect(translateError(err, fakeT)).toBe("Erro inesperado: merge.void");
  });

  test("parses a JSON-string error coming from the Tauri invoke", () => {
    const raw = JSON.stringify({
      code: "merge.base_dirty",
      params: { branch: "develop" },
    });
    expect(translateError(raw, fakeT)).toBe(
      "A branch base (develop) tem trabalho não-commitado.",
    );
  });

  test("keeps a JSON-shaped string that is not an AppError as-is", () => {
    expect(translateError('{"foo":"bar"}', fakeT)).toBe('{"foo":"bar"}');
  });

  test("uses the message of an Error instance", () => {
    expect(translateError(new Error("network down"), fakeT)).toBe(
      "network down",
    );
  });
});
