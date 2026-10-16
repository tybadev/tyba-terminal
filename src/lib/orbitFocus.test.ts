import { expect, test } from "bun:test";

import { isFocusCloseKey } from "./orbitFocus";

const key = (patch: Partial<KeyboardEvent>) => ({
  key: ".",
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...patch,
});

test("⌘. fecha no mac; Ctrl+. fora dele", () => {
  expect(isFocusCloseKey(key({ metaKey: true }), true)).toBe(true);
  expect(isFocusCloseKey(key({ ctrlKey: true }), true)).toBe(false);
  expect(isFocusCloseKey(key({ ctrlKey: true }), false)).toBe(true);
  expect(isFocusCloseKey(key({ metaKey: true }), false)).toBe(false);
});

test("Esc, ponto pelado e combinações com mais teclas não fecham", () => {
  expect(isFocusCloseKey(key({ key: "Escape" }), true)).toBe(false);
  expect(isFocusCloseKey(key({}), true)).toBe(false);
  expect(isFocusCloseKey(key({ metaKey: true, shiftKey: true }), true)).toBe(false);
  expect(isFocusCloseKey(key({ metaKey: true, altKey: true }), true)).toBe(false);
});
