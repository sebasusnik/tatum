import { onMount, onCleanup, createSignal, createEffect } from "solid-js";
import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { syntaxHighlighting, HighlightStyle } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import type { DslError } from "../audio/SynthAudio";

interface CodeViewProps {
  onSource: (source: string) => void;
  /** External source updates (e.g. from mixer fader persisting to DSL) */
  source: string;
  errors: DslError[];
  accentColor: string;
}

// Synth-themed dark highlighting
const synthHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#ff6b35" },
  { tag: tags.string, color: "#00c9b1" },
  { tag: tags.number, color: "#ffd23f" },
  { tag: tags.comment, color: "#555", fontStyle: "italic" },
  { tag: tags.lineComment, color: "#555", fontStyle: "italic" },
]);

const synthTheme = EditorView.theme({
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
  const [errorCount, setErrorCount] = createSignal(0);
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

  // Update error count from props
  const errors = () => {
    const errs = props.errors;
    setErrorCount(errs.length);
    return errs;
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
        syntaxHighlighting(synthHighlight),
        synthTheme,
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
    </div>
  );
}
