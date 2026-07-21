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

const VITESSE_DARK: SyntaxPalette = {
  comment: "#758575dd",
  keyword: "#4d9375",
  control: "#4d9375",
  string: "#c98a7d",
  function: "#80a665",
  number: "#4C9A91",
  type: "#5DA994",
  variable: "#bd976a",
  tag: "#4d9375",
  invalid: "#cb7676",
};

/// Espelha o viewer da fatia 1: no tema escuro do app o editor usa o mono-dark
/// do design system; no claro, o vitesse-dark — mesma fonte de tokens, não um
/// tema CM6 de terceiro. Função pura pra testar o mapeamento sem montar o app.
export function cmPalette(dark: boolean): SyntaxPalette {
  return dark ? MONO_DARK : VITESSE_DARK;
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
        backgroundColor: "var(--tyba-bg)",
        fontSize: "12px",
        height: "100%",
      },
      ".cm-scroller": {
        fontFamily: "var(--tyba-font-mono)",
        lineHeight: "1.5",
      },
      ".cm-content": { caretColor: "var(--tyba-text)" },
      "&.cm-focused": { outline: "none" },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--tyba-text)" },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
        {
          backgroundColor: "var(--tyba-selection)",
        },
      ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--tyba-text) 4%, transparent)" },
      ".cm-gutters": {
        backgroundColor: "var(--tyba-bg)",
        color: "var(--tyba-text-faint)",
        border: "none",
      },
      ".cm-activeLineGutter": {
        backgroundColor: "color-mix(in srgb, var(--tyba-text) 4%, transparent)",
      },
      ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 8px" },
      ".cm-diff-lane": { width: "2px", padding: "0" },
      ".cm-diff-marker": { width: "2px", height: "100%", marginLeft: "1px" },
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
