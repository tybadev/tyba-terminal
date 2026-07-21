import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";

import {
  Compartment,
  EditorState,
  RangeSet,
  RangeSetBuilder,
  StateEffect,
  StateField,
  type Extension,
} from "@codemirror/state";
import {
  EditorView,
  GutterMarker as CMGutterMarker,
  drawSelection,
  dropCursor,
  gutter,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  highlightSelectionMatches,
  search,
  searchKeymap,
} from "@codemirror/search";

import type { GutterMarker } from "@/lib/ipc";
import { cmEditorTheme, cmHighlightStyle } from "@/lib/cmTheme";
import { langExtension } from "@/lib/cmLang";

export interface CodeEditorHandle {
  getValue: () => string;
}

interface Props {
  doc: string;
  filename: string;
  dark: boolean;
  markers: GutterMarker[];
  onDirtyChange: (dirty: boolean) => void;
  onSave: () => void;
}

class DiffGutterMarker extends CMGutterMarker {
  constructor(readonly kind: string) {
    super();
  }
  eq(other: DiffGutterMarker) {
    return other.kind === this.kind;
  }
  toDOM() {
    const el = document.createElement("div");
    el.className = `cm-diff-marker cm-diff-${this.kind}`;
    return el;
  }
}

const setMarkers = StateEffect.define<GutterMarker[]>();

function buildMarkers(
  state: EditorState,
  markers: GutterMarker[],
): RangeSet<CMGutterMarker> {
  const lines = state.doc.lines;
  const sorted = [...markers]
    .filter((m) => m.line >= 1 && m.line <= lines)
    .sort((a, b) => a.line - b.line);
  const builder = new RangeSetBuilder<CMGutterMarker>();
  for (const marker of sorted) {
    const line = state.doc.line(marker.line);
    builder.add(line.from, line.from, new DiffGutterMarker(marker.kind));
  }
  return builder.finish();
}

const markersField = StateField.define<RangeSet<CMGutterMarker>>({
  create: () => RangeSet.empty,
  update(value, tr) {
    let next = value.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setMarkers)) {
        next = buildMarkers(tr.state, effect.value);
      }
    }
    return next;
  },
});

const diffGutter = gutter({
  class: "cm-diff-lane",
  markers: (view) => view.state.field(markersField),
  initialSpacer: () => new DiffGutterMarker("added"),
});

export const CodeEditor = forwardRef<CodeEditorHandle, Props>(function CodeEditor(
  { doc, filename, dark, markers, onDirtyChange, onSave },
  ref,
) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const saveRef = useRef(onSave);
  const dirtyRef = useRef(onDirtyChange);
  const baselineRef = useRef(doc);
  const themeCompartment = useRef(new Compartment());
  const langCompartment = useRef(new Compartment());

  saveRef.current = onSave;
  dirtyRef.current = onDirtyChange;

  useImperativeHandle(ref, () => ({
    getValue: () => viewRef.current?.state.doc.toString() ?? "",
  }));

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    baselineRef.current = doc;

    const themeExt: Extension = [
      cmEditorTheme(dark),
      syntaxHighlighting(cmHighlightStyle(dark)),
    ];

    const state = EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        history(),
        drawSelection(),
        dropCursor(),
        indentOnInput(),
        bracketMatching(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        markersField,
        diffGutter,
        search({ top: true }),
        keymap.of([
          {
            key: "Mod-s",
            preventDefault: true,
            run: () => {
              saveRef.current();
              return true;
            },
          },
          indentWithTab,
          ...searchKeymap,
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        themeCompartment.current.of(themeExt),
        langCompartment.current.of([]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            dirtyRef.current(
              update.state.doc.toString() !== baselineRef.current,
            );
          }
        }),
      ],
    });

    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    view.focus();

    let alive = true;
    void langExtension(filename).then((ext) => {
      if (alive && ext) {
        view.dispatch({
          effects: langCompartment.current.reconfigure(ext),
        });
      }
    });

    return () => {
      alive = false;
      view.destroy();
      viewRef.current = null;
    };
    // Remonta ao trocar de arquivo (filename) ou de documento base (reload).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filename, doc]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: themeCompartment.current.reconfigure([
        cmEditorTheme(dark),
        syntaxHighlighting(cmHighlightStyle(dark)),
      ]),
    });
  }, [dark]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({ effects: setMarkers.of(markers) });
  }, [markers]);

  return <div ref={hostRef} className="h-full min-h-0 overflow-hidden" />;
});
