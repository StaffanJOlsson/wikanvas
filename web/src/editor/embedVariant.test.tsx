// QuireEditor's board card variant (docs/ARCHITECTURE.md §18.5 Editing step 2): popups in the overlay's popup root,
// the bubble and table menus appended to it with `strategy: 'fixed'`, no block handle, no heading anchors, and no
// board items in the slash menu without their callbacks; the page variant is unchanged.
import { act, waitFor } from '@testing-library/react';
import type { Editor } from '@tiptap/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';
import type { DocState } from '../api/gen/DocState';
import { toBase64 } from '../api/queries/docs';
import { render } from '../test/render';
import { DocSession, type Outbox } from './docSession';
import { writeDoc } from './fixtures/writer';
import { QuireEditor } from './QuireEditor';

const ID = '0192f0e0-0000-7000-8000-0000000000c1';

const memory: Outbox = {
  get: async () => undefined,
  set: async () => undefined,
  del: async () => undefined,
};

async function session(): Promise<DocSession> {
  const { bin } = writeDoc([
    { type: 'heading', attrs: { level: 1 }, content: [{ type: 'text', text: 'Heading' }] },
    { type: 'paragraph', content: [{ type: 'text', text: 'Some text here' }] },
  ]);
  const y = new Y.Doc();
  Y.applyUpdate(y, bin);
  const state: DocState = { update: toBase64(bin), state_vector: toBase64(Y.encodeStateVector(y)), seq: 1, field: 'default', schema_version: 1 };
  return DocSession.open(ID, {
    fetchDoc: async () => state,
    postUpdate: async () => ({ seq: 2, state_vector: '', updated_at: '' }),
    outbox: memory,
    status: () => undefined,
    win: new EventTarget() as unknown as Window,
    doc: Object.assign(new EventTarget(), { visibilityState: 'visible' }) as unknown as Document,
  });
}

let sessions: DocSession[] = [];
let roots: HTMLElement[] = [];
afterEach(() => {
  sessions.forEach((s) => s.destroy());
  sessions = [];
  roots.forEach((r) => r.remove());
  roots = [];
});

function type(e: Editor, text: string) {
  for (const ch of text) {
    const { from, to } = e.state.selection;
    const handled = e.view.someProp('handleTextInput', (f) => f(e.view, from, to, ch, () => e.state.tr.insertText(ch, from, to)));
    if (!handled) e.view.dispatch(e.state.tr.insertText(ch, from, to));
  }
}

async function mount(variant: 'page' | 'embed') {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } })));
  const s = await session();
  sessions.push(s);
  const root = document.createElement('div');
  root.className = 'board-embed-popups';
  document.body.appendChild(root);
  roots.push(root);
  let editor: Editor | null = null;
  const props =
    variant === 'embed'
      ? { variant: 'embed' as const, popupRoot: root }
      : { pickBoard: () => undefined, newBoard: () => undefined };
  const r = render(
    <QuireEditor itemId={ID} session={s} readOnly={false} onEditor={(e) => (editor = e ?? editor)} {...props} />,
  );
  await waitFor(() => expect(editor).not.toBeNull());
  return { ...r, root, editor: editor as unknown as Editor };
}

const slashLabels = (scope: ParentNode) =>
  [...scope.querySelectorAll('[role="listbox"][aria-label="Insert a block"] [role="option"]')].map((o) => o.textContent);

describe('QuireEditor variant="embed"', () => {
  it('renders the slash and mention popups in the popup root, without the board items', async () => {
    const { root, editor, container } = await mount('embed');
    act(() => {
      editor.commands.focus('end');
      type(editor, ' /');
    });
    await waitFor(() => expect(root.querySelector('[role="listbox"][aria-label="Insert a block"]')).not.toBeNull());
    expect(container.querySelector('[role="listbox"]')).toBeNull();
    const labels = slashLabels(root);
    expect(labels).toContain('Heading 1');
    expect(labels).not.toContain('Embed board…');
    expect(labels).not.toContain('New board');
    act(() => {
      type(editor, 'board');
    });
    await waitFor(() => expect(slashLabels(root)).toEqual([]));
    act(() => {
      editor.commands.setContent('<p>x</p>');
      editor.commands.focus('end');
      type(editor, ' @');
    });
    await waitFor(() => expect(root.querySelector('[role="listbox"][aria-label="Mention a page"]')).not.toBeNull());
  });

  it('appends the bubble and table menus to the popup root with strategy fixed', async () => {
    const { root, editor } = await mount('embed');
    act(() => {
      editor.commands.setTextSelection({ from: 10, to: 14 });
    });
    await waitFor(() => expect(root.querySelector('.bubble-menu')).not.toBeNull(), { timeout: 2000 });
    await waitFor(() => expect((root.querySelector('.bubble-menu') as HTMLElement).style.position).toBe('fixed'));
    act(() => {
      editor.commands.focus('end');
      editor.commands.insertTable({ rows: 2, cols: 2, withHeaderRow: false });
    });
    await waitFor(() => expect(root.querySelector('.table-menu')).not.toBeNull());
    await waitFor(() => expect((root.querySelector('.table-menu') as HTMLElement).style.position).toBe('fixed'));
  });

  it('has no block handle and no heading anchors', async () => {
    const { editor } = await mount('embed');
    expect(document.querySelector('.block-handle')).toBeNull();
    const h1 = editor.view.dom.querySelector('h1');
    expect(h1?.textContent).toBe('Heading');
    expect(h1?.hasAttribute('id')).toBe(false);
  });
});

describe('QuireEditor variant="page" (the default)', () => {
  it('keeps its popups in place, the block handle, heading anchors and both board items', async () => {
    const { root, editor, container } = await mount('page');
    expect(editor.view.dom.querySelector('h1')?.id).toMatch(/^h-/);
    expect(document.querySelector('.block-handle')).not.toBeNull();
    act(() => {
      editor.commands.focus('end');
      type(editor, ' /');
    });
    await waitFor(() => expect(container.querySelector('[role="listbox"][aria-label="Insert a block"]')).not.toBeNull());
    expect(root.childElementCount).toBe(0);
    const labels = slashLabels(container);
    expect(labels).toContain('Embed board…');
    expect(labels).toContain('New board');
  });
});
