import type { HighlighterCore, ThemedToken } from "shiki/core";

import monoDark from "../assets/themes/mono-dark.json";

export type HighlightTheme = "vitesse-dark" | "mono-dark";

let highlighterPromise: Promise<HighlighterCore> | null = null;

const LOADED_LANGS = new Set<string>();

async function highlighter(): Promise<HighlighterCore> {
  if (!highlighterPromise) {
    highlighterPromise = (async () => {
      const [{ createHighlighterCore }, { createJavaScriptRegexEngine }, theme] =
        await Promise.all([
          import("shiki/core"),
          import("shiki/engine/javascript"),
          import("@shikijs/themes/vitesse-dark"),
        ]);
      return createHighlighterCore({
        themes: [theme.default, monoDark as unknown as typeof theme.default],
        langs: [],
        engine: createJavaScriptRegexEngine({ forgiving: true }),
      });
    })();
  }
  return highlighterPromise;
}

const LANG_LOADERS: Record<string, () => Promise<unknown>> = {
  typescript: () => import("@shikijs/langs/typescript"),
  tsx: () => import("@shikijs/langs/tsx"),
  javascript: () => import("@shikijs/langs/javascript"),
  jsx: () => import("@shikijs/langs/jsx"),
  rust: () => import("@shikijs/langs/rust"),
  json: () => import("@shikijs/langs/json"),
  jsonc: () => import("@shikijs/langs/jsonc"),
  css: () => import("@shikijs/langs/css"),
  html: () => import("@shikijs/langs/html"),
  markdown: () => import("@shikijs/langs/markdown"),
  shellscript: () => import("@shikijs/langs/shellscript"),
  toml: () => import("@shikijs/langs/toml"),
  yaml: () => import("@shikijs/langs/yaml"),
  python: () => import("@shikijs/langs/python"),
  go: () => import("@shikijs/langs/go"),
  java: () => import("@shikijs/langs/java"),
  sql: () => import("@shikijs/langs/sql"),
  swift: () => import("@shikijs/langs/swift"),
  dockerfile: () => import("@shikijs/langs/dockerfile"),
  make: () => import("@shikijs/langs/make"),
};

const LANG_BY_NAME: Record<string, string> = {
  dockerfile: "dockerfile",
  "docker-compose.yml": "yaml",
  "docker-compose.yaml": "yaml",
  "compose.yml": "yaml",
  "compose.yaml": "yaml",
  makefile: "make",
  gnumakefile: "make",
};

const LANG_BY_EXT: Record<string, string> = {
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  tsx: "tsx",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "jsx",
  rs: "rust",
  py: "python",
  go: "go",
  java: "java",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  md: "markdown",
  mdx: "markdown",
  markdown: "markdown",
  json: "json",
  jsonc: "jsonc",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  css: "css",
  scss: "css",
  less: "css",
  html: "html",
  htm: "html",
  sql: "sql",
  swift: "swift",
};

const FENCE_ALIASES: Record<string, string> = {
  typescript: "typescript",
  ts: "typescript",
  tsx: "tsx",
  javascript: "javascript",
  js: "javascript",
  jsx: "jsx",
  rust: "rust",
  rs: "rust",
  python: "python",
  py: "python",
  go: "go",
  golang: "go",
  java: "java",
  shell: "shellscript",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  markdown: "markdown",
  md: "markdown",
  json: "json",
  jsonc: "jsonc",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  css: "css",
  html: "html",
  sql: "sql",
  dockerfile: "dockerfile",
  docker: "dockerfile",
  make: "make",
  makefile: "make",
};

export function langForFile(name: string): string | null {
  const lower = name.toLowerCase();
  if (LANG_BY_NAME[lower]) return LANG_BY_NAME[lower];
  if (lower === "dockerfile" || lower.startsWith("dockerfile.")) return "dockerfile";
  const dot = lower.lastIndexOf(".");
  const ext = dot > 0 ? lower.slice(dot + 1) : "";
  return LANG_BY_EXT[ext] ?? null;
}

export function langForFence(fence: string): string | null {
  return FENCE_ALIASES[fence.toLowerCase()] ?? null;
}

export interface TokenSpan {
  text: string;
  color?: string;
}

/// Tokeniza um bloco de linhas de hunk como um trecho só (fidelidade
/// melhor que linha a linha) e devolve spans por linha. Falha → null,
/// o chamador renderiza texto puro.
export async function highlightBlock(
  lines: string[],
  lang: string,
  theme: HighlightTheme = "vitesse-dark",
): Promise<TokenSpan[][] | null> {
  const loader = LANG_LOADERS[lang];
  if (!loader) return null;
  try {
    const hl = await highlighter();
    if (!LOADED_LANGS.has(lang)) {
      const mod = (await loader()) as { default: unknown };
      await hl.loadLanguage(mod.default as never);
      LOADED_LANGS.add(lang);
    }
    const result = hl.codeToTokensBase(lines.join("\n"), {
      lang: lang as never,
      theme,
    });
    return result.map((tokens: ThemedToken[]) =>
      tokens.map((tk) => ({ text: tk.content, color: tk.color })),
    );
  } catch {
    return null;
  }
}
