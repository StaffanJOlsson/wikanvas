// DocPage renders the item's "Full width" setting (`ItemDetail.full_width`, §9.6.1: the document column loses its
// 720 px cap).
import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { DocState } from '../api/gen/DocState';
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
    await waitFor(() => expect(container.querySelector('.quire-editor')).not.toBeNull());
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
});
