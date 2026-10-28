// The slash menu (docs/ARCHITECTURE.md §9.6.5): typing filters; it closes after 3 characters that match nothing.
import { Editor } from '@tiptap/react';
import { afterEach, describe, expect, it } from 'vitest';
import { CLOSE_AFTER, filterSlash, SLASH_ITEMS, SlashCommand, SlashFilter } from './slash';
import { schemaExtensions } from './schema';
import { Popup } from './suggest';

let editor: Editor | null = null;
afterEach(() => editor?.destroy());

function type(e: Editor, text: string) {
  for (const ch of text) {
    const { from, to } = e.state.selection;
    const handled = e.view.someProp('handleTextInput', (f) => f(e.view, from, to, ch, () => e.state.tr.insertText(ch, from, to)));
    if (!handled) e.view.dispatch(e.state.tr.insertText(ch, from, to));
  }
}

describe('slash menu', () => {
  it('lists every §9.6.5 item', () => {
    expect(SLASH_ITEMS.map((i) => i.title)).toEqual([
      'Text', 'Heading 1', 'Heading 2', 'Heading 3', 'Bulleted list', 'Numbered list', 'To-do list', 'Quote',
      'Code block', 'Callout: Info', 'Callout: Tip', 'Callout: Warning', 'Callout: Danger', 'Divider', 'Table (3×3)',
      'Image', 'Mention a page', 'Embed board…', 'New board',
    ]);
  });

  it('filters by title and keywords', () => {
    expect(filterSlash('head').map((i) => i.key)).toEqual(['h1', 'h2', 'h3']);
    expect(filterSlash('todo').map((i) => i.key)).toEqual(['todo']);
    expect(filterSlash('warn').map((i) => i.key)).toEqual(['callout-warning']);
    expect(filterSlash('').length).toBe(SLASH_ITEMS.length);
    expect(filterSlash('zzz')).toEqual([]);
  });

  it('closes after 3 characters that match nothing', () => {
    const f = new SlashFilter();
    expect(f.update('ta')?.length).toBeGreaterThan(0);
    expect(f.update('tax')).toEqual([]);
    expect(f.update('taxy')).toEqual([]);
    expect(f.update('taxyz')).toBeNull();
    expect(CLOSE_AFTER).toBe(3);
  });

  it('opens on /, filters while typing, and closes in the editor', async () => {
    const popup = new Popup();
    const ctx = { pickImage: () => undefined, pickBoard: () => undefined, newBoard: () => undefined };
    editor = new Editor({ extensions: [...schemaExtensions(), SlashCommand(popup, ctx)] });
    editor.commands.focus('start');
    type(editor, '/');
    await Promise.resolve();
    expect(popup.get().open).toBe(true);
    type(editor, 'quo');
    expect(popup.get().items.map((i) => i.label)).toEqual(['Quote']);
    type(editor, 'xyz');
    expect(popup.get().open).toBe(false);
  });

  it('runs an item: Heading 1', () => {
    const popup = new Popup();
    const ctx = { pickImage: () => undefined, pickBoard: () => undefined, newBoard: () => undefined };
    editor = new Editor({ extensions: [...schemaExtensions(), SlashCommand(popup, ctx)] });
    editor.commands.focus('start');
    type(editor, '/heading');
    const item = popup.get().items.find((i) => i.label === 'Heading 1');
    item?.run();
    expect(editor.getJSON().content?.[0].type).toBe('heading');
    expect(editor.getText()).toBe('');
  });
});
