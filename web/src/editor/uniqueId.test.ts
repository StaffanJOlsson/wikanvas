// Block ids that arrive from the server (or another tab), applied with origin 'server', are never reassigned
// (docs/ARCHITECTURE.md §6.1: UniqueID's filterTransaction skips change-origin transactions).
import Collaboration from '@tiptap/extension-collaboration';
import { Editor, type JSONContent } from '@tiptap/react';
import { describe, expect, it } from 'vitest';
import * as Y from 'yjs';
import { schemaExtensions } from './schema';

const p = (id: string, text: string) => ({ type: 'paragraph', attrs: { blockId: id }, content: [{ type: 'text', text }] });

/** Server state written by another client (with its own block ids). */
function serverState(content: JSONContent[]): Uint8Array {
  const doc = new Y.Doc();
  const editor = new Editor({ extensions: [...schemaExtensions(), Collaboration.configure({ document: doc })] });
  editor.commands.setContent({ type: 'doc', content });
  const state = Y.encodeStateAsUpdate(doc);
  editor.destroy();
  return state;
}

describe('UniqueID with collaboration', () => {
  it('keeps the block ids of server state, at load and when more arrives', () => {
    let generated = 0;
    const ydoc = new Y.Doc();
    Y.applyUpdate(ydoc, serverState([p('id-a', 'A'), p('id-b', 'B')]), 'server');
    const editor = new Editor({
      extensions: [
        ...schemaExtensions({ generateID: () => `local-${++generated}` }),
        Collaboration.configure({ document: ydoc }),
      ],
    });
    const ids = () => ((editor.getJSON() as JSONContent).content ?? []).map((n) => n.attrs?.blockId);
    expect(ids()).toEqual(['id-a', 'id-b']);

    // Another tab adds a paragraph with its own id: it arrives with origin 'server'.
    const other = new Y.Doc();
    Y.applyUpdate(other, Y.encodeStateAsUpdate(ydoc));
    const otherEditor = new Editor({ extensions: [...schemaExtensions({ generateID: () => 'id-c' }), Collaboration.configure({ document: other })] });
    const sv = Y.encodeStateVector(other);
    otherEditor.commands.insertContentAt(otherEditor.state.doc.content.size, { type: 'paragraph', content: [{ type: 'text', text: 'C' }] });
    Y.applyUpdate(ydoc, Y.encodeStateAsUpdate(other, sv), 'server');
    expect(ids()).toEqual(['id-a', 'id-b', 'id-c']);
    expect(generated).toBe(0);

    // A local edit still gets a new id for its new block.
    editor.commands.insertContentAt(editor.state.doc.content.size, { type: 'paragraph', content: [{ type: 'text', text: 'D' }] });
    expect(ids()).toEqual(['id-a', 'id-b', 'id-c', 'local-1']);
    editor.destroy();
    otherEditor.destroy();
  });
});
