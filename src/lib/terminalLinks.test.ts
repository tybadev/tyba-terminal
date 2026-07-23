import { describe, expect, test } from "bun:test";

import {
  findUrlsAcrossRows,
  findUrlsInLine,
  hasOpenModifier,
  logicalLineWindow,
  type WrappedRow,
} from "./terminalLinks";

describe("findUrlsInLine", () => {
  test("detects a plain http url", () => {
    const [match] = findUrlsInLine("serving on http://localhost:5173 now");
    expect(match?.url).toBe("http://localhost:5173");
  });

  test("detects an https url with a path and query", () => {
    const [match] = findUrlsInLine(
      "open https://example.com/a/b?x=1&y=2 to continue",
    );
    expect(match?.url).toBe("https://example.com/a/b?x=1&y=2");
  });

  test("reports the string offsets of the match", () => {
    const line = "go https://example.com done";
    const [match] = findUrlsInLine(line);
    expect(match).toBeDefined();
    expect(line.slice(match!.start, match!.end)).toBe("https://example.com");
  });

  test("finds multiple urls on one line", () => {
    const matches = findUrlsInLine("a http://one.test b https://two.test c");
    expect(matches.map((m) => m.url)).toEqual([
      "http://one.test",
      "https://two.test",
    ]);
  });

  test("drops trailing sentence punctuation", () => {
    const [match] = findUrlsInLine("see https://example.com/page.");
    expect(match?.url).toBe("https://example.com/page");
  });

  test("ignores schemeless hosts and bare paths", () => {
    expect(findUrlsInLine("visit www.example.com or localhost:3000")).toEqual(
      [],
    );
    expect(findUrlsInLine("edit src/lib/terminalLinks.ts:42")).toEqual([]);
  });

  test("ignores dangerous schemes", () => {
    expect(findUrlsInLine("javascript:alert(1)")).toEqual([]);
    expect(findUrlsInLine("data:text/html,<script>x</script>")).toEqual([]);
  });

  test("does not flag dense prose without a scheme", () => {
    expect(
      findUrlsInLine("the quick brown fox: jumps/over the lazy.dog, again"),
    ).toEqual([]);
  });
});

describe("logicalLineWindow", () => {
  test("a standalone line is its own window", () => {
    const rows = [{ isWrapped: false }];
    expect(logicalLineWindow(rows, 0)).toEqual({ start: 0, end: 0 });
  });

  test("soft-wrapped continuations join upward and downward", () => {
    const rows = [
      { isWrapped: false },
      { isWrapped: true },
      { isWrapped: true },
    ];
    expect(logicalLineWindow(rows, 0)).toEqual({ start: 0, end: 2 });
    expect(logicalLineWindow(rows, 2)).toEqual({ start: 0, end: 2 });
  });

  test("a hard newline breaks the window", () => {
    const rows = [{ isWrapped: false }, { isWrapped: false }];
    expect(logicalLineWindow(rows, 0)).toEqual({ start: 0, end: 0 });
    expect(logicalLineWindow(rows, 1)).toEqual({ start: 1, end: 1 });
  });
});

describe("findUrlsAcrossRows", () => {
  const softWrapped: WrappedRow[] = [
    { text: "grab https://example.com/very/long/", isWrapped: false },
    { text: "path/to/a/page?token=abc123", isWrapped: true },
  ];

  test("joins a url split by soft-wrap into one link", () => {
    const [match] = findUrlsAcrossRows(softWrapped, 0);
    expect(match?.url).toBe(
      "https://example.com/very/long/path/to/a/page?token=abc123",
    );
  });

  test("reconstructs the same url from the continuation row", () => {
    const [match] = findUrlsAcrossRows(softWrapped, 1);
    expect(match?.url).toBe(
      "https://example.com/very/long/path/to/a/page?token=abc123",
    );
  });

  test("does not join a url split by a hard newline", () => {
    const hardWrapped: WrappedRow[] = [
      { text: "grab https://example.com/very/long/", isWrapped: false },
      { text: "path/to/a/page?token=abc123", isWrapped: false },
    ];
    const [first] = findUrlsAcrossRows(hardWrapped, 0);
    expect(first?.url).toBe("https://example.com/very/long/");
    const second = findUrlsAcrossRows(hardWrapped, 1);
    expect(second).toEqual([]);
  });
});

describe("hasOpenModifier", () => {
  test("mac requires the meta key", () => {
    expect(hasOpenModifier({ metaKey: true, ctrlKey: false }, true)).toBe(true);
    expect(hasOpenModifier({ metaKey: false, ctrlKey: true }, true)).toBe(false);
  });

  test("non-mac requires the ctrl key", () => {
    expect(hasOpenModifier({ metaKey: false, ctrlKey: true }, false)).toBe(true);
    expect(hasOpenModifier({ metaKey: true, ctrlKey: false }, false)).toBe(
      false,
    );
  });

  test("a plain click never opens", () => {
    expect(hasOpenModifier({ metaKey: false, ctrlKey: false }, true)).toBe(
      false,
    );
    expect(hasOpenModifier({ metaKey: false, ctrlKey: false }, false)).toBe(
      false,
    );
  });
});
