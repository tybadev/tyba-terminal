import { describe, expect, it, test } from "bun:test";

import {
  cellAtPoint,
  findLinksInRows,
  findPathsInLine,
  findUrlsAcrossRows,
  findUrlsInLine,
  hasOpenModifier,
  logicalLineWindow,
  opensLinkOnMouseDown,
  parsePathUri,
  rangeContains,
  type LinkRow,
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

describe("findPathsInLine", () => {
  const paths = (line: string) => findPathsInLine(line).map((p) => [p.path, p.line ?? null]);

  it("acha caminho com barra e com :linha", () => {
    expect(paths("error at src/app.tsx:12:3 near")).toEqual([["src/app.tsx", 12]]);
    expect(paths("see ./scripts/gen.ts")).toEqual([["./scripts/gen.ts", null]]);
    expect(paths("/Users/x/repo/a.rs:7")).toEqual([["/Users/x/repo/a.rs", 7]]);
    expect(paths("main.go:40: undefined")).toEqual([["main.go", 40]]);
  });

  it("ignora versão, palavra com ponto solta e o que está dentro de URL", () => {
    expect(paths("bun 1.2.3 e.g. package.json")).toEqual([]);
    expect(paths("https://github.com/a/b/blob/main/src/x.ts")).toEqual([]);
  });

  it("guarda a posição para sublinhar o trecho certo", () => {
    const [m] = findPathsInLine("  at lib/x.ts:3");
    expect(m.start).toBe(5);
    expect(m.end).toBe(15);
  });
});

describe("parsePathUri", () => {
  it("lê caminho e linha; recusa o que não é do TYBA", () => {
    expect(parsePathUri("tyba-path:src/a.ts:12")).toEqual({ path: "src/a.ts", line: 12 });
    expect(parsePathUri("tyba-path:src/a.ts")).toEqual({ path: "src/a.ts", line: undefined });
    expect(parsePathUri("https://x.dev")).toBeNull();
  });
});

describe("findLinksInRows", () => {
  const COLS = 40;
  // O Claude Code quebra a URL ele mesmo: `\n` + recuo, sem `isWrapped`.
  const claude: LinkRow[] = [
    { text: "⏺ Abra https://claude.ai/design/p/0f3a9", isWrapped: false },
    { text: "  c2e-b1c2-4e5f-9a8b-1234567890ab no", isWrapped: false },
    { text: "  navegador.", isWrapped: false },
  ];
  const full = "https://claude.ai/design/p/0f3a9c2e-b1c2-4e5f-9a8b-1234567890ab";

  test("junta a URL que o programa quebrou na borda, pulando o recuo", () => {
    const [link] = findLinksInRows(claude, 0, COLS);
    expect(link?.uri).toBe(full);
    expect(link?.start).toEqual({ row: 0, index: 7 });
    expect(link?.end).toEqual({ row: 1, index: 32 });
  });

  test("a mesma URL inteira vale de cima da linha de continuação", () => {
    const [link] = findLinksInRows(claude, 1, COLS);
    expect(link?.uri).toBe(full);
  });

  test("linha que não continua a URL fica fora do trecho", () => {
    expect(findLinksInRows(claude, 2, COLS)).toEqual([]);
  });

  test("URL longe da borda não junta com a palavra de baixo", () => {
    const rows: LinkRow[] = [
      { text: "veja https://example.com/docs", isWrapped: false },
      { text: "para detalhes", isWrapped: false },
    ];
    const [link] = findLinksInRows(rows, 0, 80);
    expect(link?.uri).toBe("https://example.com/docs");
    expect(findLinksInRows(rows, 1, 80)).toEqual([]);
  });

  test("uma URL nova na linha de baixo não é continuação", () => {
    const rows: LinkRow[] = [
      { text: "a https://one.example.com/aaaaaaaaaaaaaaa", isWrapped: false },
      { text: "https://two.example.com", isWrapped: false },
    ];
    const links = findLinksInRows(rows, 0, 40);
    expect(links.map((l) => l.uri)).toEqual(["https://one.example.com/aaaaaaaaaaaaaaa"]);
  });

  test("continua juntando o wrap do próprio xterm", () => {
    const rows: LinkRow[] = [
      { text: "grab https://example.com/very/long/", isWrapped: false },
      { text: "path/to/a/page?token=abc123", isWrapped: true },
    ];
    const [link] = findLinksInRows(rows, 1, 35);
    expect(link?.uri).toBe("https://example.com/very/long/path/to/a/page?token=abc123");
    expect(link?.start).toEqual({ row: 0, index: 5 });
  });

  test("caminho de arquivo só com `files`", () => {
    const rows: LinkRow[] = [{ text: "erro em src/app.tsx:12", isWrapped: false }];
    expect(findLinksInRows(rows, 0, 80)).toEqual([]);
    const [link] = findLinksInRows(rows, 0, 80, { files: true });
    expect(link?.uri).toBe("tyba-path:src/app.tsx:12");
  });
});

describe("rangeContains", () => {
  const range = { start: { x: 8, y: 3 }, end: { x: 36, y: 4 } };

  test("cobre do início na primeira linha ao fim na última", () => {
    expect(rangeContains(range, 8, 3)).toBe(true);
    expect(rangeContains(range, 40, 3)).toBe(true);
    expect(rangeContains(range, 1, 4)).toBe(true);
    expect(rangeContains(range, 36, 4)).toBe(true);
  });

  test("fora das pontas e das linhas não conta", () => {
    expect(rangeContains(range, 7, 3)).toBe(false);
    expect(rangeContains(range, 37, 4)).toBe(false);
    expect(rangeContains(range, 10, 2)).toBe(false);
    expect(rangeContains(range, 10, 5)).toBe(false);
  });
});

describe("cellAtPoint", () => {
  const box = { left: 100, top: 50, width: 800, height: 400 };

  test("converte o ponto na célula da viewport", () => {
    expect(cellAtPoint({ clientX: 100, clientY: 50 }, box, 80, 20)).toEqual({ col: 0, row: 0 });
    expect(cellAtPoint({ clientX: 155, clientY: 95 }, box, 80, 20)).toEqual({ col: 5, row: 2 });
    expect(cellAtPoint({ clientX: 899, clientY: 449 }, box, 80, 20)).toEqual({ col: 79, row: 19 });
  });

  test("fora da tela não há célula", () => {
    expect(cellAtPoint({ clientX: 99, clientY: 60 }, box, 80, 20)).toBeNull();
    expect(cellAtPoint({ clientX: 900, clientY: 60 }, box, 80, 20)).toBeNull();
    expect(cellAtPoint({ clientX: 200, clientY: 60 }, { ...box, width: 0 }, 80, 20)).toBeNull();
  });
});

describe("opensLinkOnMouseDown", () => {
  const click = { button: 0, metaKey: false, ctrlKey: false };

  test("⌘+clique no mac abre — com o mouse tracking do programa ligado ou não", () => {
    // A decisão não olha o tracking de propósito: com ele ligado o xterm
    // mandaria o clique ao Claude, e é por isso que o TYBA intercepta antes.
    expect(opensLinkOnMouseDown({ ...click, metaKey: true }, true, true)).toBe(true);
  });

  test("Ctrl+clique abre fora do mac; ⌘ não", () => {
    expect(opensLinkOnMouseDown({ ...click, ctrlKey: true }, false, true)).toBe(true);
    expect(opensLinkOnMouseDown({ ...click, metaKey: true }, false, true)).toBe(false);
  });

  test("Ctrl+clique no mac não abre (é o menu de contexto)", () => {
    expect(opensLinkOnMouseDown({ ...click, ctrlKey: true }, true, true)).toBe(false);
  });

  test("clique sem modificador vai para o programa", () => {
    expect(opensLinkOnMouseDown(click, true, true)).toBe(false);
  });

  test("sem link sob o mouse o clique segue para o terminal", () => {
    expect(opensLinkOnMouseDown({ ...click, metaKey: true }, true, false)).toBe(false);
  });

  test("só o botão principal", () => {
    expect(opensLinkOnMouseDown({ ...click, button: 2, metaKey: true }, true, true)).toBe(false);
  });
});
