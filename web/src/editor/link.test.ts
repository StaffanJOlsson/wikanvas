// Links stay within the §6.4 scheme allow-list (docs/ARCHITECTURE.md §9.6.5): a pasted or typed javascript: or
// data: URL stays text.
import { Editor } from '@tiptap/react';
import { afterEach, describe, expect, it } from 'vitest';
import { isAllowedImageSrc, isAllowedLink, wikanvasItemId, sameInstanceUrlId } from './links';
import { schemaExtensions } from './schema';

let editor: Editor | null = null;
afterEach(() => editor?.destroy());

const marksOf = (e: Editor) => {
  const out: string[] = [];
  e.state.doc.descendants((n) => {
    for (const m of n.marks) out.push(`${m.type.name}:${String(m.attrs.href ?? '')}`);
  });
  return out;
};

function type(e: Editor, text: string) {
  for (const ch of text) {
    const { from, to } = e.state.selection;
    const handled = e.view.someProp('handleTextInput', (f) => f(e.view, from, to, ch, () => e.state.tr.insertText(ch, from, to)));
    if (!handled) e.view.dispatch(e.state.tr.insertText(ch, from, to));
  }
}

describe('links', () => {
  it('allow-lists schemes', () => {
    for (const ok of ['https://example.com', 'http://x', 'mailto:a@b.c', 'wikanvas://item/x', '/doc/a', '#h-1']) {
      expect(isAllowedLink(ok), ok).toBe(true);
    }
    // The same list as wikanvas-core's convert::tests::allow_list: the two allow-lists must agree.
    for (const bad of [
      'javascript:alert(1)', 'JAVASCRIPT:x', 'java\tscript:x', ' javascript:x', 'data:text/html,x', 'vbscript:x',
      '//evil.com', '/\\evil.com', '/\t/evil.com', '/\n/evil.com', '/\r/evil.com', ' //evil.com', '/ /evil.com', '', '\t',
    ]) {
      expect(isAllowedLink(bad), JSON.stringify(bad)).toBe(false);
      expect(isAllowedImageSrc(bad), JSON.stringify(bad)).toBe(false);
    }
    expect(isAllowedImageSrc('mailto:a@b.c')).toBe(false);
    expect(isAllowedImageSrc('data:image/png;base64,AA')).toBe(false);
    expect(wikanvasItemId('wikanvas://item/0192F0A0-1111-7000-8000-00000000000A')).toBe('0192f0a0-1111-7000-8000-00000000000a');
    expect(sameInstanceUrlId('https://wiki.example/doc/my-page-AbCdEfGh12', 'https://wiki.example')).toBe('AbCdEfGh12');
    expect(sameInstanceUrlId('https://other.example/doc/my-page-AbCdEfGh12', 'https://wiki.example')).toBeNull();
    expect(sameInstanceUrlId('/board/AbCdEfGh12', 'https://wiki.example')).toBe('AbCdEfGh12');
  });

  it('keeps pasted javascript: and data: links as text', () => {
    editor = new Editor({ extensions: schemaExtensions() });
    editor.commands.setContent('<p><a href="javascript:alert(1)">bad</a> <a href="data:text/html,x">worse</a> <a href="https://ok.example">ok</a></p>');
    expect(editor.getText()).toBe('bad worse ok');
    expect(marksOf(editor)).toEqual(['link:https://ok.example']);
    expect(editor.commands.setTextSelection({ from: 1, to: 4 })).toBe(true);
    editor.commands.setLink({ href: 'javascript:alert(1)' });
    expect(marksOf(editor)).toEqual(['link:https://ok.example']);
  });

  it('does not autolink a typed javascript: URL', () => {
    editor = new Editor({ extensions: schemaExtensions() });
    editor.commands.focus('end');
    type(editor, 'javascript:alert(1) data:text/html,x https://good.example ');
    expect(marksOf(editor).filter((m) => m.startsWith('link:javascript') || m.startsWith('link:data'))).toEqual([]);
    expect(editor.getText()).toContain('javascript:alert(1)');
  });
});
