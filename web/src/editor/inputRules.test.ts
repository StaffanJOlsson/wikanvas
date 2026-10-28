// Markdown shortcuts (docs/ARCHITECTURE.md §9.6.5): `:::info ` (callouts), `[] ` (to-do), ``` (code block), ==x==.
import { Editor, type JSONContent } from '@tiptap/react';
import { afterEach, describe, expect, it } from 'vitest';
import { schemaExtensions } from './schema';

let editor: Editor | null = null;
afterEach(() => editor?.destroy());

function fresh(): Editor {
  editor = new Editor({ extensions: schemaExtensions() });
  editor.commands.focus('start');
  return editor;
}

/** Types like a user: each character through the input-rule handlers. */
function type(e: Editor, text: string) {
  for (const ch of text) {
    const { from, to } = e.state.selection;
    const handled = e.view.someProp('handleTextInput', (f) => f(e.view, from, to, ch, () => e.state.tr.insertText(ch, from, to)));
    if (!handled) e.view.dispatch(e.state.tr.insertText(ch, from, to));
  }
}

const first = (e: Editor): JSONContent | undefined => (e.getJSON() as JSONContent).content?.[0];

describe('input rules', () => {
  for (const variant of ['info', 'tip', 'warning', 'danger']) {
    it(`:::${variant} + space makes a ${variant} callout`, () => {
      const e = fresh();
      type(e, `:::${variant} hello`);
      const c = first(e);
      expect(c?.type).toBe('callout');
      expect(c?.attrs?.variant).toBe(variant);
      expect(e.state.doc.textContent).toBe('hello');
    });
  }

  it('[] and [ ] make a to-do list', () => {
    for (const trigger of ['[] ', '[ ] ']) {
      const e = fresh();
      type(e, `${trigger}task`);
      expect(first(e)?.type).toBe('taskList');
      expect(first(e)?.content?.[0].attrs?.checked).toBe(false);
      e.destroy();
      editor = null;
    }
  });

  it('``` makes a code block, with a language', () => {
    const e = fresh();
    type(e, '```rust ');
    expect(first(e)?.type).toBe('codeBlock');
    expect(first(e)?.attrs?.language).toBe('rust');
  });

  it('==x== highlights', () => {
    const e = fresh();
    type(e, 'a ==marked== b');
    const texts = first(e)?.content ?? [];
    const marked = texts.find((t) => t.text === 'marked');
    expect(marked?.marks?.map((m) => m.type)).toEqual(['highlight']);
  });
});
