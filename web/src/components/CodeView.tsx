import { onMount, onCleanup, createEffect } from "solid-js";
import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, Decoration } from "@codemirror/view";
import type { DecorationSet } from "@codemirror/view";
import { EditorState, StateField, StateEffect } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { syntaxHighlighting, HighlightStyle } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import type { DslError } from "../audio/TatumAudio";

interface CodeViewProps {
  onSource: (source: string) => void;
  /** External source updates (e.g. from mixer fader persisting to DSL) */
  source: string;
  errors: DslError[];
  accentColor: string;
}

// Tatum-themed dark highlighting
const tatumHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#ff6b35" },
  { tag: tags.string, color: "#00c9b1" },
  { tag: tags.number, color: "#ffd23f" },
  { tag: tags.comment, color: "#555", fontStyle: "italic" },
  { tag: tags.lineComment, color: "#555", fontStyle: "italic" },
]);

// ── Error line highlighting ──
// The engine reports parse and compile errors with a 1-based line. We mark
// those lines with a line decoration and let CSS style `.cm-error-line`.
const setErrorLines = StateEffect.define<number[]>();

const errorLineMark = Decoration.line({ class: "cm-error-line" });

const errorLineField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const effect of tr.effects) {
      if (!effect.is(setErrorLines)) continue;
      const doc = tr.state.doc;
      const marks = [...new Set(effect.value)]
        .filter((line) => line >= 1 && line <= doc.lines)
        .sort((a, b) => a - b)
        .map((line) => errorLineMark.range(doc.line(line).from));
      deco = Decoration.set(marks);
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

const tatumTheme = EditorView.theme({
  "&": {
    backgroundColor: "#0d0d0f",
    color: "#c8c8cc",
    fontSize: "13px",
    fontFamily: "'JetBrains Mono', 'Fira Code', 'SF Mono', monospace",
  },
  ".cm-content": {
    caretColor: "#ff6b35",
    padding: "12px 0",
  },
  ".cm-cursor": { borderLeftColor: "#ff6b35" },
  ".cm-activeLine": { backgroundColor: "#ffffff08" },
  ".cm-gutters": {
    backgroundColor: "#0d0d0f",
    color: "#444",
    border: "none",
    minWidth: "32px",
  },
  ".cm-activeLineGutter": { backgroundColor: "#ffffff08" },
  ".cm-selectionBackground": { backgroundColor: "#ff6b3530 !important" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "#ff6b3540 !important" },
  ".cm-line": { padding: "0 8px" },
});

export default function CodeView(props: CodeViewProps) {
  let container!: HTMLDivElement;
  let view: EditorView | undefined;
  let debounceTimer: number | undefined;
  let lastSource = "";
  // Track externally-set source to break feedback loops.
  // When an external update sets the editor text, we store it here.
  // The debounced evaluate checks against this to avoid re-sending.
  let externalSource = "";

  const evaluate = (doc: string) => {
    // If this doc matches what was just set externally, skip — no recompile needed
    if (doc === externalSource) return;
    if (doc !== lastSource) {
      lastSource = doc;
      props.onSource(doc);
    }
  };

  const errors = () => props.errors;

  // Push error lines into the editor whenever the engine reports a new result
  createEffect(() => {
    const lines = props.errors.map((e) => e.line);
    view?.dispatch({ effects: setErrorLines.of(lines) });
  });

  const jumpToLine = (line: number) => {
    if (!view || line < 1) return;
    const pos = view.state.doc.line(Math.min(line, view.state.doc.lines)).from;
    view.dispatch({
      selection: { anchor: pos },
      effects: EditorView.scrollIntoView(pos, { y: "center" }),
    });
    view.focus();
  };

  // React to external source changes (e.g. from mixer persisting to DSL)
  createEffect(() => {
    const ext = props.source;
    if (view && ext && ext !== lastSource) {
      lastSource = ext;
      externalSource = ext; // mark as external so debounce won't re-send
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: ext },
      });
    }
  });

  onMount(() => {
    const initialDoc = props.source || "";
    const state = EditorState.create({
      doc: initialDoc,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        drawSelection(),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        syntaxHighlighting(tatumHighlight),
        tatumTheme,
        errorLineField,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            clearTimeout(debounceTimer);
            debounceTimer = window.setTimeout(() => {
              evaluate(update.state.doc.toString());
            }, 150);
          }
        }),
        // Ctrl+Enter: force evaluate immediately
        keymap.of([{
          key: "Ctrl-Enter",
          run: (v) => { evaluate(v.state.doc.toString()); return true; },
        }, {
          key: "Cmd-Enter",
          run: (v) => { evaluate(v.state.doc.toString()); return true; },
        }]),
      ],
    });

    view = new EditorView({ state, parent: container });

    // Store initial source (don't send to WASM — wait for user gesture via Play)
    if (initialDoc) {
      lastSource = initialDoc;
    }
  });

  onCleanup(() => {
    clearTimeout(debounceTimer);
    view?.destroy();
  });

  return (
    <div class="code-view">
      <div class="code-header">
        <span class="code-title">DSL</span>
        <span class="code-hint">Ctrl+Enter to evaluate</span>
        {errors().length > 0 && (
          <span class="code-errors">
            {errors().length} error{errors().length > 1 ? "s" : ""}
            {errors().length > 0 && `: ${errors()[0].msg}`}
          </span>
        )}
      </div>
      <div ref={container} class="code-editor" />
      {errors().length > 0 && (
        <ul class="code-error-list">
          {errors().map((e) => (
            <li onClick={() => jumpToLine(e.line)} classList={{ jumpable: e.line > 0 }}>
              <span class="code-error-line">{e.line > 0 ? `L${e.line}` : "—"}</span>
              <span class="code-error-msg">{e.msg}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
