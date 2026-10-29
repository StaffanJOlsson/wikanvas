// DocPage renders the item's "Full width" setting (`ItemDetail.full_width`, §9.6.1: the document column loses its
// 720 px cap).
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { DocState } from '../api/gen/DocState';
import type { ItemDetail } from '../api/gen/ItemDetail';
import { READ_ONLY_COLLECTION } from '../lib/access';
import { itemPath } from '../lib/urls';
import { makeCollection, makeDetail, makeItem, mockApi } from '../test/mockApi';
import { render } from '../test/render';
import DocPage from './DocPage';

const EMPTY: DocState = { update: 'AAA=', state_vector: '', seq: 0, field: 'default', schema_version: 1 };

function renderDoc(fullWidth: boolean) {
  const c = makeCollection({ id: 'c1' });
  const doc = makeItem({ id: 'd1', collection_id: 'c1', title: 'Plan' });
  mockApi()
    .on('GET', `/items/${doc.url_id}`, [200, makeDetail(doc, c, { full_width: fullWidth })])
    .on('GET', '/items/:id/doc', [200, EMPTY])
    .on('GET', '/items/:id/children', [200, []]);
  return render(<DocPage />, { route: itemPath(doc), path: '/doc/:slugId' });
}

describe('DocPage', () => {
  it('marks the page full width when ItemDetail.full_width is set', async () => {
    const { container } = renderDoc(true);
    await screen.findByDisplayValue('Plan');
    await waitFor(() => expect(container.querySelector('.doc-page')?.hasAttribute('data-full-width')).toBe(true));
  });

  it('mounts the editor without an inline <style> (the SPA CSP blocks it; editor.css has the base rules)', async () => {
    const { container } = renderDoc(false);
    await waitFor(() => expect(container.querySelector('.wikanvas-editor')).not.toBeNull());
    expect(document.querySelector('style[data-tiptap-style]')).toBeNull();
    expect(document.querySelectorAll('style')).toHaveLength(0);
  });

  it('records one view of the document (Recently viewed fills only through POST …/view, §7.2)', async () => {
    const c = makeCollection({ id: 'c1' });
    const doc = makeItem({ id: 'd1', collection_id: 'c1', title: 'Plan' });
    const api = mockApi()
      .on('GET', `/items/${doc.url_id}`, [200, makeDetail(doc, c)])
      .on('GET', '/items/:id/doc', [200, EMPTY])
      .on('GET', '/items/:id/children', [200, []])
      .on('POST', '/items/:id/view', [204]);
    render(<DocPage />, { route: itemPath(doc), path: '/doc/:slugId' });
    await screen.findByDisplayValue('Plan');
    await waitFor(() => expect(api.calls('POST', '/items/:id/view')).toHaveLength(1));
    expect(api.calls('POST', '/items/:id/view')[0]!.params.id).toBe('d1');
  });

  it('keeps the 720 px column otherwise', async () => {
    const { container } = renderDoc(false);
    await screen.findByDisplayValue('Plan');
    expect(container.querySelector('.doc-page')).not.toBeNull();
    expect(container.querySelector('.doc-page')?.hasAttribute('data-full-width')).toBe(false);
  });

  describe('in a collection the caller may only read (D152)', () => {
    function renderReadOnly(over: Partial<ItemDetail> = {}) {
      const c = makeCollection({ id: 'c1', name: 'Handbook', my_access: 'read' });
      const doc = makeItem({ id: 'd1', collection_id: 'c1', title: 'Plan' });
      const api = mockApi()
        .on('GET', `/items/${doc.url_id}`, [200, makeDetail(doc, c, over)])
        .on('GET', '/items/:id/doc', [200, EMPTY])
        .on('GET', '/items/:id/children', [200, []])
        .on('POST', '/items/:id/view', [204]);
      const view = render(<DocPage />, { route: itemPath(doc), path: '/doc/:slugId' });
      return { ...view, api };
    }

    it('opens read-only: the banner, no title or body editing, no create or destructive actions', async () => {
      const { container, api } = renderReadOnly();
      const title = (await screen.findByDisplayValue('Plan')) as HTMLInputElement;
      expect(screen.getByText(READ_ONLY_COLLECTION).className).toBe('read-only-banner');
      expect(screen.queryByRole('button', { name: 'Restore' })).toBeNull();
      expect(title.readOnly).toBe(true);
      await waitFor(() => expect(container.querySelector('.ProseMirror')).not.toBeNull());
      expect(container.querySelector('.ProseMirror')!.getAttribute('contenteditable')).toBe('false');
      expect(screen.queryByRole('button', { name: /Add a page/ })).toBeNull();
      expect(screen.queryByRole('button', { name: /New doc/ })).toBeNull();
      // Star stays (read is enough); the menu keeps Star and the exports only.
      expect(screen.getByRole('button', { name: 'Star' })).toBeTruthy();
      fireEvent.click(screen.getByRole('button', { name: 'Item menu' }));
      const menu = screen.getByRole('menu');
      expect([...menu.querySelectorAll('[role^="menuitem"]')].map((b) => b.textContent)).toEqual(['Star', 'Export Markdown', 'Export HTML']);
      // Nothing was written.
      expect(api.calls('PATCH')).toHaveLength(0);
      expect(api.calls('POST').filter((c) => !c.path.endsWith('/view'))).toHaveLength(0);
    });

    it('a trashed document there says it is in the Trash, without Restore', async () => {
      renderReadOnly({ trashed_at: '2026-10-28T10:00:00.000Z', purge_at: '2026-11-27T10:00:00.000Z' });
      await screen.findByDisplayValue('Plan');
      expect(screen.getByText('This document is in the Trash.')).toBeTruthy();
      expect(screen.queryByRole('button', { name: 'Restore' })).toBeNull();
      expect(screen.queryByText(READ_ONLY_COLLECTION)).toBeNull();
    });

    it('a writable trashed document keeps its Restore', async () => {
      const c = makeCollection({ id: 'c1', my_access: 'write' });
      const doc = makeItem({ id: 'd1', collection_id: 'c1', title: 'Plan' });
      mockApi()
        .on('GET', `/items/${doc.url_id}`, [200, makeDetail(doc, c, { trashed_at: '2026-10-28T10:00:00.000Z' })])
        .on('GET', '/items/:id/doc', [200, EMPTY])
        .on('GET', '/items/:id/children', [200, []]);
      render(<DocPage />, { route: itemPath(doc), path: '/doc/:slugId' });
      await screen.findByDisplayValue('Plan');
      expect(screen.getByRole('button', { name: 'Restore' })).toBeTruthy();
    });
  });
});
