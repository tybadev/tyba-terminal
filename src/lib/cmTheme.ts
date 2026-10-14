import { HighlightStyle } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

export interface SyntaxPalette {
  comment: string;
  keyword: string;
  control: string;
  string: string;
  function: string;
  number: string;
  type: string;
  variable: string;
  tag: string;
  invalid: string;
}

const MONO_DARK: SyntaxPalette = {
  comment: "#546E7A",
  keyword: "#C792EA",
  control: "#89DDFF",
  string: "#C3E88D",
  function: "#82AAFF",
  number: "#F78C6C",
  type: "#FFCB6B",
  variable: "#f07178",
  tag: "#f07178",
  invalid: "#FF5370",
};

/// Mockup Editor: palavra-chave lilás, string lima, função azul, número âmbar,
/// comentário apagado e variável na cor do texto — os acentos do TYBA.
const TYBA_DARK: SyntaxPalette = {
  ...MONO_DARK,
  comment: "#5f6066",
  keyword: "#c792ea",
  string: "#a8e05f",
  function: "#4c7df0",
  number: "#f5a93b",
  variable: "#f5f5f6",
};

/// Mockup EditorClaro, com os tons de texto da paleta do claro (PropostaClaro).
const TYBA_LIGHT: SyntaxPalette = {
  comment: "#8b8b96",
  keyword: "#7a4ce8",
  control: "#55555f",
  string: "#3f7a1b",
  function: "#3b66d6",
  number: "#9a5f0c",
  type: "#0c7c6f",
  variable: "#121218",
  tag: "#b5321f",
  invalid: "#b5321f",
};

/// A paleta de sintaxe dos mockups Editor/EditorClaro. Função pura pra testar
/// o mapeamento sem montar o app.
export function cmPalette(dark: boolean): SyntaxPalette {
  return dark ? TYBA_DARK : TYBA_LIGHT;
}

export function cmHighlightStyle(dark: boolean): HighlightStyle {
  const p = cmPalette(dark);
  return HighlightStyle.define([
    { tag: t.comment, color: p.comment, fontStyle: "italic" },
    { tag: [t.keyword, t.modifier, t.definitionKeyword], color: p.keyword },
    {
      tag: [t.controlKeyword, t.operatorKeyword, t.moduleKeyword],
      color: p.control,
    },
    { tag: [t.string, t.special(t.string)], color: p.string },
    { tag: [t.regexp, t.escape], color: p.control },
    {
      tag: [t.function(t.variableName), t.function(t.propertyName)],
      color: p.function,
    },
    {
      tag: [t.number, t.bool, t.null, t.atom, t.unit],
      color: p.number,
    },
    {
      tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName)],
      color: p.type,
    },
    { tag: [t.propertyName, t.attributeName], color: p.type },
    { tag: [t.variableName, t.self], color: p.variable },
    { tag: [t.tagName, t.angleBracket], color: p.tag },
    { tag: [t.operator, t.punctuation, t.separator], color: p.control },
    { tag: [t.heading, t.strong], color: p.type, fontWeight: "bold" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: t.link, color: p.function, textDecoration: "underline" },
    { tag: t.invalid, color: p.invalid },
  ]);
}

/// Cromo do editor amarrado às CSS vars do app: acompanha a troca de tema em
/// runtime sem reconstruir a HighlightStyle. Coerente com o viewer (bg/text).
export function cmEditorTheme(dark: boolean): ReturnType<typeof EditorView.theme> {
  return EditorView.theme(
    {
      "&": {
        color: "var(--tyba-text)",
        backgroundColor: "var(--tyba-sunken)",
        height: "100%",
      },
      ".cm-scroller": {
        fontFamily: "var(--tyba-font-mono)",
      },
      ".cm-content": { caretColor: "var(--tyba-green)", padding: "10px 0" },
      "&.cm-focused": { outline: "none" },
      // Mockup: cursor verde de 2px, aceso (o brilho some no claro).
      ".cm-cursor, .cm-dropCursor": {
        borderLeft: "2px solid var(--tyba-green)",
        boxShadow: "var(--tyba-glow-green)",
      },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
        {
          backgroundColor: "var(--tyba-selection)",
        },
      ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--tyba-text) 3%, transparent)" },
      ".cm-gutters": {
        backgroundColor: "var(--tyba-sunken)",
        color: "var(--tyba-text-ghost)",
        border: "none",
      },
      ".cm-activeLineGutter": {
        backgroundColor: "color-mix(in srgb, var(--tyba-text) 3%, transparent)",
        color: "var(--tyba-text-muted)",
      },
      // Mockup: número alinhado à direita em 44px com 14px de folga, a faixa
      // de alteração de 3px e 10px até o código.
      ".cm-lineNumbers .cm-gutterElement": { padding: "0 14px 0 8px", minWidth: "22px" },
      ".cm-diff-lane": { width: "3px", padding: "0", marginRight: "10px" },
      ".cm-diff-marker": { width: "3px", height: "100%", marginLeft: "0" },
      ".cm-diff-added": { backgroundColor: "var(--tyba-green)" },
      ".cm-diff-modified": { backgroundColor: "var(--tyba-amber)" },
      ".cm-diff-deleted": {
        backgroundColor: "var(--tyba-red)",
        height: "2px",
        marginTop: "-1px",
      },
      ".cm-panels": {
        backgroundColor: "var(--tyba-surface)",
        color: "var(--tyba-text)",
      },
      ".cm-panel.cm-search": {
        backgroundColor: "var(--tyba-surface)",
        fontFamily: "var(--tyba-font-ui)",
      },
      ".cm-searchMatch": {
        backgroundColor: "color-mix(in srgb, var(--tyba-amber) 30%, transparent)",
      },
      ".cm-searchMatch.cm-searchMatch-selected": {
        backgroundColor: "color-mix(in srgb, var(--tyba-amber) 55%, transparent)",
      },
      ".cm-matchingBracket": {
        backgroundColor: "color-mix(in srgb, var(--tyba-text) 14%, transparent)",
        outline: "none",
      },
    },
    { dark },
  );
}
