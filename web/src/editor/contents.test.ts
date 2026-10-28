// The Contents column rules (docs/ARCHITECTURE.md §9.6.4): ≥2 headings, `#h-<blockId>` anchors, 30% scroll spy.
import Collaboration from '@tiptap/extension-collaboration';
import { Editor } from '@tiptap/react';
import { describe, expect, it } from 'vitest';
import * as Y from 'yjs';
import { activeHeading, HeadingAnchors, headingAnchor, headingsOf, showContents } from './contents';
import { schemaExtensions, seededIds } from './schema';

const h = (id: string, text: string, level = 2) => ({ type: 'heading', attrs: { level, blockId: id }, content: [{ type: 'text', text }] });
const p = (text: string) => ({ type: 'paragraph', attrs: { blockId: null }, content: [{ type: 'text', text }] });

describe('contents', () => {
  it('shows with at least two headings', () => {
    expect(showContents(headingsOf({ type: 'doc', content: [h('a', 'One'), p('x')] }))).toBe(false);
    const two = headingsOf({ type: 'doc', content: [h('a', 'One'), p('x'), h('b', 'Two', 3)] });
    expect(showContents(two)).toBe(true);
    expect(two).toEqual([
      { blockId: 'a', level: 2, text: 'One' },
      { blockId: 'b', level: 3, text: 'Two' },
    ]);
  });

  it('anchors headings as #h-<blockId>', () => {
    expect(headingAnchor('abc')).toBe('h-abc');
    const ydoc = new Y.Doc();
    const editor = new Editor({
      extensions: [...schemaExtensions({ generateID: seededIds() }), HeadingAnchors, Collaboration.configure({ document: ydoc })],
    });
    editor.commands.setContent({ type: 'doc', content: [h('ignored', 'Title', 1), p('Body')] });
    const id = editor.getJSON().content?.[0].attrs?.blockId as string;
    expect(id).toBeTruthy();
    expect(editor.view.dom.querySelector('h1')?.id).toBe(`h-${id}`);
    editor.destroy();
  });

  it('picks the last heading above 30% of the viewport', () => {
    const tops = [
      { blockId: 'a', top: -400 },
      { blockId: 'b', top: 250 },
      { blockId: 'c', top: 400 },
    ];
    expect(activeHeading(tops, 1000)).toBe('b');
    expect(activeHeading(tops, 800)).toBe('a');
    expect(activeHeading([{ blockId: 'a', top: 500 }], 1000)).toBe('a');
    expect(activeHeading([], 1000)).toBeNull();
    expect(activeHeading(tops, 10_000)).toBe('c');
    expect(activeHeading([{ blockId: 'x', top: 300 }, { blockId: 'y', top: 301 }], 1000)).toBe('x');
  });
});
