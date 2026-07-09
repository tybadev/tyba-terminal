import { describe, expect, test } from "bun:test";

import {
  flattenPaste,
  isMultilinePaste,
  isSafeExternalUrl,
} from "./clipboard";

describe("isSafeExternalUrl", () => {
  test("accepts http and https", () => {
    expect(isSafeExternalUrl("http://example.com")).toBe(true);
    expect(isSafeExternalUrl("https://example.com/path?x=1")).toBe(true);
  });

  test("rejects file protocol", () => {
    expect(isSafeExternalUrl("file:///etc/passwd")).toBe(false);
  });

  test("rejects javascript protocol", () => {
    expect(isSafeExternalUrl("javascript:alert(1)")).toBe(false);
  });

  test("rejects data protocol", () => {
    expect(isSafeExternalUrl("data:text/html,<script>alert(1)</script>")).toBe(
      false,
    );
  });

  test("rejects vbscript protocol", () => {
    expect(isSafeExternalUrl("vbscript:msgbox(1)")).toBe(false);
  });

  test("rejects empty string", () => {
    expect(isSafeExternalUrl("")).toBe(false);
  });

  test("rejects non-url string", () => {
    expect(isSafeExternalUrl("not a url")).toBe(false);
  });

  test("rejects https with empty host", () => {
    expect(isSafeExternalUrl("https://")).toBe(false);
    expect(isSafeExternalUrl("https://:80")).toBe(false);
  });
});

describe("isMultilinePaste", () => {
  test("detects newline", () => {
    expect(isMultilinePaste("a\nb")).toBe(true);
  });

  test("detects carriage return", () => {
    expect(isMultilinePaste("a\rb")).toBe(true);
  });

  test("detects crlf", () => {
    expect(isMultilinePaste("a\r\nb")).toBe(true);
  });

  test("false for single line", () => {
    expect(isMultilinePaste("single line")).toBe(false);
  });
});

describe("flattenPaste", () => {
  test("flattens lf", () => {
    expect(flattenPaste("a\nb")).toBe("a b");
  });

  test("flattens crlf without double space", () => {
    expect(flattenPaste("a\r\nb")).toBe("a b");
  });

  test("flattens cr", () => {
    expect(flattenPaste("a\rb")).toBe("a b");
  });

  test("collapses repeated resulting spaces", () => {
    expect(flattenPaste("a\n\nb")).toBe("a b");
    expect(flattenPaste("a\r\n\r\nb")).toBe("a b");
  });
});
