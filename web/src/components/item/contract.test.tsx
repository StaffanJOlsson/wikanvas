// The W0b contract components render with their frozen props and their minimal behaviour works.
import { act, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemDetail } from '../../api/gen/ItemDetail';
import { hasPending } from '../../state/pending';
import { clearSaveStatus, setSaveStatus } from '../../state/saveStatus';
import { render } from '../../test/render';
import { TopBar } from '../topbar/TopBar';
import { ItemHeader } from './ItemHeader';
import { ItemMenu } from './ItemMenu';

const detail: ItemDetail = {
  item: {
    id: 'item-1',
    url_id: 'AbCdEfGh12',
    collection_id: 'col-1',
    parent_id: 'parent-1',
    kind: 'doc',
    title: 'Release notes',
    sort_key: 'a0',
    child_count: 0,
    published: true,
    archived: false,
    updated_at: '2026-10-28T09:00:00.000Z',
    content_at: null,
    created_at: '2026-10-28T09:00:00.000Z',
  },
  collection: {
    id: 'col-1',
    url_id: 'Zz09Zz09Zz',
    name: 'Engineering',
    description: '',
    icon: null,
    color: '#3E63DD',
    sort_key: 'a0',
    child_order: 'manual',
    item_count: 2,
    updated_at: '2026-10-28T09:00:00.000Z',
  },
  breadcrumbs: [{ id: 'parent-1', url_id: 'Pp11Pp11Pp', kind: 'doc', title: 'Handbook' }],
  starred: false,
  effectively_published: true,
  trashed_at: null,
  purge_at: null,
  archived_at: null,
  board: null,
  meta_version: 1,
};

afterEach(() => {
  vi.useRealTimers();
  clearSaveStatus('item-1');
});

describe('contract components', () => {
  it('TopBar shows the breadcrumbs, the save status and the actions', () => {
    render(<TopBar item={detail} actions={<button type="button">Extra</button>} />);
    expect(screen.getByRole('link', { name: 'Engineering' }).getAttribute('href')).toBe('/collection/engineering-Zz09Zz09Zz');
    expect(screen.getByRole('link', { name: 'Handbook' }).getAttribute('href')).toBe('/doc/handbook-Pp11Pp11Pp');
    expect(screen.getByText(/Release notes/).getAttribute('aria-current')).toBe('page');
    expect(screen.getByRole('button', { name: 'Extra' })).toBeTruthy();
    act(() => setSaveStatus('item-1', 'saving'));
    expect(screen.getByRole('status').textContent).toBe('Saving…');
  });

  it("TopBar with titleCrumb 'none' leaves out the current title", () => {
    render(<TopBar item={detail} titleCrumb="none" />);
    expect(screen.queryByText(/Release notes/)).toBeNull();
  });

  it('ItemHeader: Enter moves to the body; typing saves the title after 500 ms, registered in pending', async () => {
    vi.useFakeTimers();
    const fetch = vi.fn(async (_url: RequestInfo | URL, init?: RequestInit) =>
      new Response(JSON.stringify({ ...detail.item, title: JSON.parse(String(init?.body)).title }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }));
    vi.stubGlobal('fetch', fetch);
    const onEnterBody = vi.fn();
    render(<ItemHeader item={detail} onEnterBody={onEnterBody} readOnly={false} />);
    const input = screen.getByRole('textbox', { name: 'Title' }) as HTMLInputElement;
    expect(input.value).toBe('Release notes');
    expect(screen.getByText(/^You created /)).toBeTruthy();

    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onEnterBody).toHaveBeenCalledOnce();

    fireEvent.change(input, { target: { value: 'Release notes 2' } });
    expect(hasPending('item-1')).toBe(true);
    await act(async () => vi.advanceTimersByTimeAsync(499));
    expect(fetch).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(fetch).toHaveBeenCalledOnce();
    const [url, init] = fetch.mock.calls[0]!;
    expect(url).toBe('/api/v1/items/item-1');
    expect(init?.method).toBe('PATCH');
    expect(JSON.parse(String(init?.body))).toEqual({ title: 'Release notes 2' });
    expect(hasPending('item-1')).toBe(false);
  });

  it('ItemMenu lists the actions and the extra exports', async () => {
    const run = vi.fn();
    render(<ItemMenu item={detail} extraExports={[{ label: 'HTML', run }]} />);
    fireEvent.click(screen.getByRole('button', { name: 'Item menu' }));
    const items = screen.getAllByRole('menuitem').map((b) => b.textContent);
    expect(items).toEqual(['Star', 'Duplicate', 'Archive', 'Export HTML', 'Delete']);
    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: 'Export HTML' }));
    });
    expect(run).toHaveBeenCalledOnce();
    expect(screen.queryByRole('menu')).toBeNull();
  });
});
