// The Contents column rules (docs/ARCHITECTURE.md §9.6.4): ≥2 headings, `#h-<blockId>` anchors, 30% scroll spy.
import Collaboration from '@tiptap/extension-collaboration';
import { Editor } from '@tiptap/react';
import { describe, expect, it } from 'vitest';
import * as Y from 'yjs';
import { activeHeading, HeadingAnchors, headingAnchor, headingsOf, scrolledToEnd, showContents } from './contents';
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

  it('at the end of the page, picks the last heading in view (D147)', () => {
    // The live "Known gaps" page scrolled to its end: "Tests and tooling" never reaches the 30% line.
    const tops = [
      { blockId: 'behaviour', top: -147 },
      { blockId: 'performance', top: 148 },
      { blockId: 'tests', top: 356 },
    ];
    expect(activeHeading(tops, 795)).toBe('performance');
    expect(activeHeading(tops, 795, true)).toBe('tests');
    // A heading below the viewport is not in view; with none in view the 30% rule decides.
    expect(activeHeading([...tops, { blockId: 'below', top: 900 }], 795, true)).toBe('tests');
    expect(activeHeading([{ blockId: 'a', top: -900 }, { blockId: 'b', top: 900 }], 795, true)).toBe('a');
  });

  it('knows when the scroller of a heading is at its end', () => {
    const scroller = document.createElement('div');
    scroller.style.overflowY = 'auto';
    const heading = document.createElement('h2');
    scroller.append(heading);
    document.body.append(scroller);
    const set = (top: number, height: number, client: number) => {
      Object.defineProperty(scroller, 'scrollTop', { configurable: true, value: top });
      Object.defineProperty(scroller, 'scrollHeight', { configurable: true, value: height });
      Object.defineProperty(scroller, 'clientHeight', { configurable: true, value: client });
    };
    // The live page at its end: scrollTop 392 of a possible 393.
    set(392, 1188, 795);
    expect(scrolledToEnd(heading)).toBe(true);
    set(391, 1188, 795);
    expect(scrolledToEnd(heading)).toBe(true);
    set(390, 1188, 795);
    expect(scrolledToEnd(heading)).toBe(false);
    set(200, 1188, 795);
    expect(scrolledToEnd(heading)).toBe(false);
    // Nothing to scroll: never at the end (the document does not scroll in the test either).
    set(0, 600, 795);
    expect(scrolledToEnd(heading)).toBe(false);
    expect(scrolledToEnd(null)).toBe(false);
    scroller.remove();
  });
});
