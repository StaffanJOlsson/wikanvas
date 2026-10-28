// "Embed page…" (docs/ARCHITECTURE.md §18.6 (b)): a dialog that finds documents by title (boards filtered out); Enter
// takes the first, Esc closes; the chosen page is placed at the centre, selected, and the frame gets focus.
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { TitleHit } from '../../api/gen/TitleHit';
import { boardItem, boardRoutes, DOC_1, DOC_2, docItem, mockFetch, openBoardApp, type Recorded, type Rendered } from '../testkit';

let app: Rendered | null = null;

afterEach(() => {
  app?.unmount();
  app = null;
});

const board = boardItem(1);
const BOARD_HIT = '0192f0e0-0000-7000-8000-0000000000b1';

const hits: TitleHit[] = [
  { item: { ...docItem(BOARD_HIT).item, kind: 'board', title: 'Chart board' }, path: [] },
  {
    item: { ...docItem(DOC_1).item, title: 'Chapter one' },
    path: [{ id: 'p', url_id: 'PaReNt0001', kind: 'doc', title: 'Book' }],
  },
  { item: { ...docItem(DOC_2).item, title: 'Chapter two' }, path: [] },
];

function setup() {
  const m = mockFetch({ ...boardRoutes(board), 'GET /api/v1/search/titles': [200, hits] });
  vi.stubGlobal('fetch', m.fetch);
  return m.requests;
}

const searches = (requests: Recorded[]) => requests.filter((q) => q.path === '/api/v1/search/titles');

async function openPicker() {
  const button = (await screen.findByRole('button', { name: 'Embed page…' })) as HTMLButtonElement;
  await waitFor(() => expect(button.disabled).toBe(false));
  fireEvent.click(button);
  return screen.findByRole('dialog', { name: 'Embed a page' });
}

describe('Embed page…', () => {
  it('lists documents only, with their path; a click places the page at the centre and focuses the frame', async () => {
    const requests = setup();
    const r = (app = await openBoardApp(board));
    const dialog = await openPicker();
    const input = within(dialog).getByPlaceholderText('Find a page');
    expect(document.activeElement).toBe(input);
    await within(dialog).findByRole('button', { name: 'Chapter one' });
    expect(within(dialog).queryByRole('button', { name: 'Chart board' })).toBeNull();
    expect(within(dialog).getByRole('button', { name: 'Chapter one' }).textContent).toContain('Book');
    expect(searches(requests)[0].search).toBe('?q=&limit=20');
    fireEvent.change(input, { target: { value: 'Chap' } });
    await waitFor(() => expect(searches(requests).some((q) => q.search === '?q=Chap&limit=20')).toBe(true));
    const focusBefore = r.fake.calls.filter((c) => c.method === 'focus').length;
    fireEvent.click(within(dialog).getByRole('button', { name: 'Chapter two' }));
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    expect(r.fake.inserted[0]).toEqual({ id: 'inserted1', item: DOC_2, title: 'Chapter two', at: 'center' });
    expect(screen.queryByRole('dialog', { name: 'Embed a page' })).toBeNull();
    expect(r.fake.calls.filter((c) => c.method === 'focus').length).toBe(focusBefore + 1);
    // Placed and selected, not opened.
    expect(r.fake.embedEditing()).toBeNull();
    // The frame got focus after the dialog gave it back to its button.
    const lastFocus = r.fake.calls.map((c) => c.method).lastIndexOf('focus');
    expect(lastFocus).toBe(r.fake.calls.length - 1);
  });

  it('Enter takes the first document; Esc closes without placing anything', async () => {
    setup();
    const r = (app = await openBoardApp(board));
    let dialog = await openPicker();
    await within(dialog).findByRole('button', { name: 'Chapter one' });
    fireEvent.keyDown(within(dialog).getByPlaceholderText('Find a page'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Embed a page' })).toBeNull());
    expect(r.fake.inserted).toHaveLength(0);

    dialog = await openPicker();
    await within(dialog).findByRole('button', { name: 'Chapter one' });
    await act(async () => {
      fireEvent.keyDown(within(dialog).getByPlaceholderText('Find a page'), { key: 'Enter' });
    });
    await waitFor(() => expect(r.fake.inserted).toHaveLength(1));
    expect(r.fake.inserted[0]).toMatchObject({ item: DOC_1, title: 'Chapter one', at: 'center' });
  });
});
