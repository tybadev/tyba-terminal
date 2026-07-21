import { StreamLanguage } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import type { StreamParser } from "@codemirror/language";

import { langForFile } from "./highlight";

async function stream(
  loader: () => Promise<Record<string, unknown>>,
  key: string,
): Promise<Extension> {
  const mod = await loader();
  return StreamLanguage.define(mod[key] as StreamParser<unknown>);
}

/// Gramáticas Lezer/legacy carregadas on-demand com code-split, mesmo padrão do
/// Shiki (`langForFile`, fatia 1): só a linguagem do arquivo aberto entra no
/// bundle. Sem gramática conhecida → null (o editor abre em texto puro).
export async function langExtension(
  filename: string,
): Promise<Extension | null> {
  const lang = langForFile(filename);
  if (!lang) return null;
  switch (lang) {
    case "typescript": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript({ typescript: true });
    }
    case "tsx": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript({ typescript: true, jsx: true });
    }
    case "javascript": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript();
    }
    case "jsx": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript({ jsx: true });
    }
    case "rust": {
      const { rust } = await import("@codemirror/lang-rust");
      return rust();
    }
    case "python": {
      const { python } = await import("@codemirror/lang-python");
      return python();
    }
    case "go": {
      const { go } = await import("@codemirror/lang-go");
      return go();
    }
    case "java": {
      const { java } = await import("@codemirror/lang-java");
      return java();
    }
    case "json":
    case "jsonc": {
      const { json } = await import("@codemirror/lang-json");
      return json();
    }
    case "yaml": {
      const { yaml } = await import("@codemirror/lang-yaml");
      return yaml();
    }
    case "markdown": {
      const { markdown } = await import("@codemirror/lang-markdown");
      return markdown();
    }
    case "html": {
      const { html } = await import("@codemirror/lang-html");
      return html();
    }
    case "css": {
      const { css } = await import("@codemirror/lang-css");
      return css();
    }
    case "sql": {
      const { sql } = await import("@codemirror/lang-sql");
      return sql();
    }
    case "shellscript":
      return stream(() => import("@codemirror/legacy-modes/mode/shell"), "shell");
    case "toml":
      return stream(() => import("@codemirror/legacy-modes/mode/toml"), "toml");
    case "dockerfile":
      return stream(
        () => import("@codemirror/legacy-modes/mode/dockerfile"),
        "dockerFile",
      );
    case "swift":
      return stream(() => import("@codemirror/legacy-modes/mode/swift"), "swift");
    default:
      return null;
  }
}
