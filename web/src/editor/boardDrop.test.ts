// A sidebar board dropped on a document (docs/ARCHITECTURE.md §9.6.5, D145): `BoardDrop` turns the board row's native
// drag data into a board card at the nearest block boundary of the drop point, through ProseMirror's own drop path;
// a document row's drag, bad data and a read-only editor insert nothing.
import { Editor } from '@tiptap/react';
import { NodeSelection } from '@tiptap/pm/state';
import { afterEach, describe, expect, it } from 'vitest';
import { boardDragDataOf, DRAG_BOARD_TYPE, DRAG_ITEM_TYPE, dragDataOf, parseDraggedBoard } from '../board/embed/dragItem';
import { BoardDrop } from './extensions';
import { schemaExtensions } from './schema';

const BOARD = '0192f0a0-5555-7000-8000-00000000000b';
const DOC = '0192f0a0-5555-7000-8000-00000000000d';

const editors: Editor[] = [];

afterEach(() => {
  for (const e of editors.splice(0)) e.destroy();
});

function makeEditor(html: string, editable = true): Editor {
  const e = new Editor({ extensions: [...schemaExtensions(), BoardDrop], content: html, editable });
  editors.push(e);
  return e;
}

/** Drops `data` on the editor with the drop point at document position `pos` (happy-dom has no layout). */
function drop(e: Editor, data: Record<string, string>, pos: number): DragEvent {
  e.view.posAtCoords = () => ({ pos, inside: -1 });
  const dt = new DataTransfer();
  for (const [k, v] of Object.entries(data)) dt.setData(k, v);
  const ev = new Event('drop', { bubbles: true, cancelable: true }) as DragEvent;
  Object.defineProperty(ev, 'dataTransfer', { value: dt });
  Object.defineProperty(ev, 'clientX', { value: 10 });
  Object.defineProperty(ev, 'clientY', { value: 10 });
  e.view.dom.dispatchEvent(ev);
  return ev;
}

function types(e: Editor): (string | undefined)[] {
  return (e.getJSON().content ?? []).map((n) => n.type);
}

describe('dropping a sidebar board on a document', () => {
  it('embeds the board before or after the block it lands in, whole, and selects the card', () => {
    // "Hello world" spans positions 1 to 12: a drop in its first half goes before it, in its second half after it.
    const e = makeEditor('<p>Hello world</p>');
    const ev = drop(e, boardDragDataOf({ id: BOARD, title: 'Plan' }), 3);
    expect(ev.defaultPrevented).toBe(true);
    expect(types(e)).toEqual(['boardEmbed', 'paragraph']);
    expect(e.getJSON().content?.[0].attrs?.itemId).toBe(BOARD);
    expect(e.state.doc.textContent).toBe('Hello world');
    expect(e.state.selection).toBeInstanceOf(NodeSelection);
    expect((e.state.selection as NodeSelection).node.type.name).toBe('boardEmbed');

    drop(e, boardDragDataOf({ id: BOARD, title: 'Plan' }), e.state.doc.content.size - 2);
    expect(types(e)).toEqual(['boardEmbed', 'paragraph', 'boardEmbed']);
    expect(e.state.doc.textContent).toBe('Hello world');
  });

  it('a document row, data that does not parse and a read-only editor insert nothing', () => {
    const e = makeEditor('<p>Hello world</p>');
    const before = JSON.stringify(e.getJSON());
    // A sidebar document is the board frame's (§18.6 (c)); the editor leaves it alone.
    expect(drop(e, dragDataOf({ id: DOC, title: 'Notes' }), 3).defaultPrevented).toBe(false);
    for (const raw of [
      '{not json',
      JSON.stringify({ v: 1, id: BOARD, kind: 'doc', title: 'Plan' }),
      JSON.stringify({ v: 2, id: BOARD, kind: 'board', title: 'Plan' }),
      JSON.stringify({ v: 1, id: 'not-a-uuid', kind: 'board', title: 'Plan' }),
      JSON.stringify({ v: 1, id: BOARD.toUpperCase(), kind: 'board', title: 'Plan' }),
    ]) {
      // The drag carries the type, so it is taken (nothing else inserts it as text), but nothing is placed.
      expect(drop(e, { [DRAG_BOARD_TYPE]: raw }, 3).defaultPrevented).toBe(true);
    }
    expect(JSON.stringify(e.getJSON())).toBe(before);

    const ro = makeEditor('<p>Read only</p>', false);
    const roBefore = JSON.stringify(ro.getJSON());
    drop(ro, boardDragDataOf({ id: BOARD, title: 'Plan' }), 3);
    expect(JSON.stringify(ro.getJSON())).toBe(roBefore);
  });
});

describe('the board drag data', () => {
  it('round-trips, and is not the document type the board frame reads', () => {
    const data = boardDragDataOf({ id: BOARD, title: 'Plan' });
    expect(Object.keys(data)).toEqual([DRAG_BOARD_TYPE]);
    expect(DRAG_BOARD_TYPE).not.toBe(DRAG_ITEM_TYPE);
    expect(parseDraggedBoard(data[DRAG_BOARD_TYPE])).toEqual({ v: 1, id: BOARD, kind: 'board', title: 'Plan' });
    expect(parseDraggedBoard(null)).toBeNull();
    expect(parseDraggedBoard('null')).toBeNull();
  });
});
