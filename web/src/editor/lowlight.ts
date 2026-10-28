// Code highlighting that loads lazily (docs/ARCHITECTURE.md §9.6.5): CodeBlockLowlight gets a stand-in that
// highlights nothing until `lowlight` (with its common grammars) arrives in its own chunk; then every open editor
// redraws its code blocks.
import type { Editor } from '@tiptap/react';

type Root = { type: 'root'; children: unknown[] };
interface Lowlight {
  highlight(language: string, value: string): Root;
  highlightAuto(value: string): Root;
  listLanguages(): string[];
  registered(language: string): boolean;
}

let real: Lowlight | null = null;
let loading: Promise<void> | null = null;
const editors = new Set<Editor>();

const empty = (): Root => ({ type: 'root', children: [] });

/** The lowlight CodeBlockLowlight uses. */
export const lazyLowlight: Lowlight = {
  highlight: (language, value) => (real ? real.highlight(language, value) : empty()),
  highlightAuto: (value) => (real ? real.highlightAuto(value) : empty()),
  listLanguages: () => (real ? real.listLanguages() : []),
  registered: (language) => (real ? real.registered(language) : false),
};

/** Starts loading the grammars; `editor` redraws its code blocks when they are there. */
export function loadLowlight(editor: Editor): void {
  editors.add(editor);
  if (real) return;
  loading ??= import('lowlight').then(({ common, createLowlight }) => {
    real = createLowlight(common) as unknown as Lowlight;
    for (const e of editors) {
      if (e.isDestroyed) continue;
      // A no-op setNodeMarkup on each code block makes the plugin recompute its decorations (no Y change).
      const tr = e.state.tr;
      e.state.doc.descendants((node, pos) => {
        if (node.type.name === 'codeBlock') tr.setNodeMarkup(pos, undefined, node.attrs);
      });
      if (tr.docChanged) e.view.dispatch(tr.setMeta('addToHistory', false));
    }
  });
}

export function forgetEditor(editor: Editor): void {
  editors.delete(editor);
}
