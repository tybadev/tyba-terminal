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
  /** `return`, `if`, `import`/`export`: no escuro é o `control`. */
  flow: string;
  /** `true`, `false`, `null`: no escuro é o `number`. */
  literal: string;
  /** Propriedade e atributo (`turn.id`, `className=`): no escuro é o `type`. */
  property: string;
  /** `<`, `/>` em volta da tag: no escuro é a cor da tag. */
  bracket: string;
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
  flow: "#89DDFF",
  literal: "#F78C6C",
  property: "#FFCB6B",
  bracket: "#f07178",
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
  literal: "#f5a93b",
};

/// Quadro Files do Muriki (`.q-k/.q-s/.q-f/.q-ty/.q-c`): palavra-chave
/// violeta (com `return`, `export` e `true`/`false`), string verde, função
/// azul da marca, tipo e nome de tag âmbar, comentário no ink-subtle; o resto
/// (pontuação, propriedade, atributo, `<`/`/>`) no marinho do texto.
const TYBA_LIGHT: SyntaxPalette = {
  comment: "#5c6880",
  keyword: "#7a4ce8",
  control: "#0a1830",
  string: "#15803d",
  function: "#1152d2",
  number: "#9a5f0c",
  type: "#a16207",
  variable: "#0a1830",
  tag: "#a16207",
  invalid: "#b5321f",
  flow: "#7a4ce8",
  literal: "#7a4ce8",
  property: "#0a1830",
  bracket: "#0a1830",
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
      color: p.flow,
    },
    { tag: [t.string, t.special(t.string)], color: p.string },
    { tag: [t.regexp, t.escape], color: p.control },
    {
      tag: [t.function(t.variableName), t.function(t.propertyName)],
      color: p.function,
    },
    { tag: [t.number, t.unit], color: p.number },
    { tag: [t.bool, t.null, t.atom], color: p.literal },
    {
      tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName)],
      color: p.type,
    },
    { tag: [t.propertyName, t.attributeName], color: p.property },
    { tag: [t.variableName, t.self], color: p.variable },
    { tag: t.tagName, color: p.tag },
    { tag: t.angleBracket, color: p.bracket },
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
        // Opção G: quem pinta é a folha (ou a tela) em volta. No claro do
        // Muriki o código fica no branco do quadro (`.q-code`).
        backgroundColor: "var(--tyba-editor-code-bg, transparent)",
        height: "100%",
      },
      ".cm-scroller": {
        fontFamily: "var(--tyba-font-mono)",
      },
      ".cm-content": { caretColor: "var(--tyba-green)", padding: "var(--tyba-editor-pad, 10px 0)" },
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
      ".cm-activeLine": {
        backgroundColor: "var(--tyba-editor-active-line, color-mix(in srgb, var(--tyba-text) 3%, transparent))",
      },
      ".cm-gutters": {
        // Opaca: a calha fica sticky quando o código rola na horizontal.
        // No acrílico do escuro a calha é transparente com blur: sem faixa
        // entre o número e o código, e o que rola por baixo fica desfocado.
        backgroundColor:
          "var(--tyba-editor-gutter-bg, var(--tyba-editor-code-bg, var(--tyba-paper-solid)))",
        backdropFilter: "var(--tyba-editor-gutter-fx, none)",
        color: "var(--tyba-text-ghost)",
        border: "none",
      },
      ".cm-activeLineGutter": {
        backgroundColor: "var(--tyba-editor-active-line, color-mix(in srgb, var(--tyba-text) 3%, transparent))",
        color: "var(--tyba-text-muted)",
      },
      // Quadro Files (`.q-cr`, grade 6px | 44px | 1fr com 12 de folga): a
      // faixa de alteração vem primeiro (3px numa coluna de 6), depois o
      // número alinhado à direita em 44px, depois o código.
      ".cm-diff-lane": { width: "6px", padding: "0", marginRight: "12px" },
      ".cm-lineNumbers .cm-gutterElement": {
        padding: "0 12px 0 0",
        minWidth: "var(--tyba-editor-ln-min, 56px)",
      },
      // O código começa logo depois dos 12px de folga do número (o padrão do
      // CodeMirror soma mais 6px à esquerda de cada linha).
      ".cm-line": { paddingLeft: "2px" },
      ".cm-diff-marker": { width: "3px", height: "100%", marginLeft: "0" },
      ".cm-diff-added": { backgroundColor: "var(--tyba-editor-diff-add, var(--tyba-green))" },
      ".cm-diff-modified": { backgroundColor: "var(--tyba-editor-diff-mod, var(--tyba-amber))" },
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
