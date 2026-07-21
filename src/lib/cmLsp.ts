import {
  autocompletion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import {
  Prec,
  StateEffect,
  StateField,
  type EditorState,
  type Extension,
  type Text,
} from "@codemirror/state";
import {
  Decoration,
  EditorView,
  hoverTooltip,
  showTooltip,
  type DecorationSet,
  type Tooltip,
} from "@codemirror/view";

import { IS_MAC } from "@/lib/platform";
import type {
  LspCompletionItem,
  LspContentChange,
  LspDiagnostic,
  LspHover,
  LspLocation,
  LspPosition,
  LspSignatureHelp,
} from "@/lib/ipc";

export interface LspBridge {
  completion: (line: number, character: number) => Promise<LspCompletionItem[]>;
  hover: (line: number, character: number) => Promise<LspHover | null>;
  signature: (
    line: number,
    character: number,
  ) => Promise<LspSignatureHelp | null>;
  definition: (line: number, character: number) => Promise<LspLocation[]>;
  change: (changes: LspContentChange[]) => void;
  gotoDefinition: (loc: LspLocation) => void;
}

function offsetToPos(doc: Text, offset: number): LspPosition {
  const line = doc.lineAt(offset);
  return { line: line.number - 1, character: offset - line.from };
}

function posToOffset(doc: Text, pos: LspPosition): number {
  if (pos.line >= doc.lines) return doc.length;
  const line = doc.line(pos.line + 1);
  return Math.min(line.from + pos.character, line.to);
}

const KIND_TYPE: Record<number, string> = {
  1: "text",
  2: "method",
  3: "function",
  4: "function",
  5: "variable",
  6: "variable",
  7: "class",
  8: "interface",
  9: "namespace",
  10: "property",
  11: "type",
  12: "constant",
  13: "enum",
  14: "keyword",
  15: "text",
  16: "constant",
  17: "text",
  21: "constant",
  22: "type",
  23: "enum",
  25: "type",
};

const TRIGGER = new Set([".", ":", ">", "/", "@", '"', "'", "(", " "]);

function completionExtension(bridge: LspBridge): Extension {
  const source = async (
    context: CompletionContext,
  ): Promise<CompletionResult | null> => {
    const word = context.matchBefore(/[\w$]*/);
    const before = context.state.sliceDoc(
      Math.max(0, context.pos - 1),
      context.pos,
    );
    const hasWord = word && word.from !== word.to;
    if (!context.explicit && !hasWord && !TRIGGER.has(before)) {
      return null;
    }
    const pos = offsetToPos(context.state.doc, context.pos);
    const items = await bridge.completion(pos.line, pos.character);
    if (context.aborted || items.length === 0) return null;
    const from = word ? word.from : context.pos;
    return {
      from,
      options: items.slice(0, 200).map((item) => ({
        label: item.label,
        detail: item.detail ?? undefined,
        type: item.kind != null ? KIND_TYPE[item.kind] : undefined,
        apply: item.insert_text ?? item.label,
      })),
      validFor: /^[\w$]*$/,
    };
  };
  return autocompletion({
    override: [source],
    activateOnTyping: true,
    activateOnTypingDelay: 140,
    icons: false,
    maxRenderedOptions: 60,
  });
}

const setDiagnosticsEffect = StateEffect.define<LspDiagnostic[]>();

function buildDiagnostics(state: EditorState, diags: LspDiagnostic[]): DecorationSet {
  const ranges = diags
    .map((diag) => {
      const from = posToOffset(state.doc, diag.range.start);
      let to = posToOffset(state.doc, diag.range.end);
      if (to <= from) to = Math.min(from + 1, state.doc.length);
      if (to <= from) return null;
      const severity = diag.severity || 1;
      return Decoration.mark({
        class: `cm-lsp-diag cm-lsp-diag-sev${severity}`,
        message: diag.message,
      }).range(from, to);
    })
    .filter((r): r is NonNullable<typeof r> => r !== null)
    .sort((a, b) => a.from - b.from || a.to - b.to);
  return Decoration.set(ranges, true);
}

const diagnosticsField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    let next = value.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setDiagnosticsEffect)) {
        next = buildDiagnostics(tr.state, effect.value);
      }
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function applyLspDiagnostics(view: EditorView, diags: LspDiagnostic[]) {
  view.dispatch({ effects: setDiagnosticsEffect.of(diags) });
}

function diagnosticAt(state: EditorState, pos: number): string | null {
  let message: string | null = null;
  state
    .field(diagnosticsField)
    .between(pos, pos, (_from, _to, deco) => {
      const spec = deco.spec as { message?: string };
      if (spec.message) {
        message = spec.message;
        return false;
      }
      return undefined;
    });
  return message;
}

function hoverExtension(bridge: LspBridge): Extension {
  return hoverTooltip(
    async (view, pos): Promise<Tooltip | null> => {
      const diagMessage = diagnosticAt(view.state, pos);
      const p = offsetToPos(view.state.doc, pos);
      const info = await bridge.hover(p.line, p.character);
      if (!info && !diagMessage) return null;
      return {
        pos,
        above: true,
        create: () => {
          const dom = document.createElement("div");
          dom.className = "cm-lsp-tooltip";
          if (diagMessage) {
            const diag = document.createElement("div");
            diag.className = "cm-lsp-tooltip-diag";
            diag.textContent = diagMessage;
            dom.appendChild(diag);
          }
          if (info) {
            const body = document.createElement("div");
            body.className = "cm-lsp-tooltip-body";
            body.textContent = info.contents;
            dom.appendChild(body);
          }
          return { dom };
        },
      };
    },
    { hoverTime: 320 },
  );
}

const setSignatureEffect = StateEffect.define<Tooltip | null>();

const signatureField = StateField.define<Tooltip | null>({
  create: () => null,
  update(value, tr) {
    let next = value;
    if (next && tr.selection) next = null;
    for (const effect of tr.effects) {
      if (effect.is(setSignatureEffect)) next = effect.value;
    }
    return next;
  },
  provide: (field) => showTooltip.from(field),
});

function signatureTooltip(help: LspSignatureHelp, pos: number): Tooltip {
  return {
    pos,
    above: true,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-lsp-signature";
      const active = help.signatures[help.active_signature] ?? help.signatures[0];
      if (active) {
        const label = document.createElement("div");
        label.className = "cm-lsp-signature-label";
        label.textContent = active.label;
        dom.appendChild(label);
        const param = active.parameters[help.active_parameter];
        if (param) {
          const hint = document.createElement("div");
          hint.className = "cm-lsp-signature-active";
          hint.textContent = param;
          dom.appendChild(hint);
        }
      }
      return { dom };
    },
  };
}

function signatureExtension(bridge: LspBridge): Extension {
  return [
    signatureField,
    EditorView.updateListener.of((update) => {
      if (!update.docChanged) return;
      let trigger: "open" | "close" | null = null;
      update.changes.iterChanges((_a, _b, _c, _d, inserted) => {
        const text = inserted.toString();
        if (text.endsWith("(") || text.endsWith(",")) trigger = "open";
        else if (text.endsWith(")")) trigger = "close";
      });
      if (trigger === "close") {
        update.view.dispatch({ effects: setSignatureEffect.of(null) });
        return;
      }
      if (trigger !== "open") return;
      const view = update.view;
      const head = view.state.selection.main.head;
      const p = offsetToPos(view.state.doc, head);
      void bridge.signature(p.line, p.character).then((help) => {
        if (!help || help.signatures.length === 0) {
          view.dispatch({ effects: setSignatureEffect.of(null) });
          return;
        }
        view.dispatch({
          effects: setSignatureEffect.of(signatureTooltip(help, head)),
        });
      });
    }),
  ];
}

function gotoDefinition(bridge: LspBridge): Extension {
  return EditorView.domEventHandlers({
    mousedown(event, view) {
      const mod = IS_MAC ? event.metaKey : event.ctrlKey;
      if (!mod || event.button !== 0) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null) return false;
      event.preventDefault();
      const p = offsetToPos(view.state.doc, pos);
      void bridge.definition(p.line, p.character).then((locs) => {
        if (locs.length > 0) bridge.gotoDefinition(locs[0]);
      });
      return true;
    },
  });
}

function changeExtension(bridge: LspBridge): Extension {
  return EditorView.updateListener.of((update) => {
    if (!update.docChanged) return;
    const doc = update.startState.doc;
    const changes: LspContentChange[] = [];
    update.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
      changes.push({
        range: { start: offsetToPos(doc, fromA), end: offsetToPos(doc, toA) },
        text: inserted.toString(),
      });
    });
    changes.reverse();
    if (changes.length > 0) bridge.change(changes);
  });
}

const lspTheme = EditorView.baseTheme({
  ".cm-lsp-diag": {
    textDecoration: "underline wavy",
    textDecorationSkipInk: "none",
    textUnderlineOffset: "3px",
  },
  ".cm-lsp-diag-sev1": { textDecorationColor: "var(--tyba-red, #e5484d)" },
  ".cm-lsp-diag-sev2": { textDecorationColor: "var(--tyba-amber, #f5a623)" },
  ".cm-lsp-diag-sev3": { textDecorationColor: "var(--tyba-blue, #4a90d9)" },
  ".cm-lsp-diag-sev4": { textDecorationColor: "var(--tyba-blue, #4a90d9)" },
  ".cm-lsp-tooltip": {
    maxWidth: "480px",
    padding: "4px 8px",
    fontSize: "12px",
    lineHeight: "1.5",
    whiteSpace: "pre-wrap",
  },
  ".cm-lsp-tooltip-diag": { color: "var(--tyba-red, #e5484d)", marginBottom: "4px" },
  ".cm-lsp-tooltip-body": { fontFamily: "monospace" },
  ".cm-lsp-signature": {
    maxWidth: "480px",
    padding: "4px 8px",
    fontSize: "12px",
    fontFamily: "monospace",
    whiteSpace: "pre-wrap",
  },
  ".cm-lsp-signature-active": { marginTop: "2px", fontWeight: "600" },
});

export function lspExtensions(bridge: LspBridge): Extension {
  return [
    diagnosticsField,
    Prec.high(completionExtension(bridge)),
    hoverExtension(bridge),
    signatureExtension(bridge),
    gotoDefinition(bridge),
    changeExtension(bridge),
    lspTheme,
  ];
}
