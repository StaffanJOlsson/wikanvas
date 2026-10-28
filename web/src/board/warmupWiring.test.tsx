// The sidebar's 300 ms board-row hover (W3, `boardRowHovered`) reaches the board host's engine warm-up (W6,
// `useBoardRowWarmup`, registered by BoardFrameHost): §9.5 "hovering a board row for 300 ms warms the cache".
import { fireEvent, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { makeCollection, makeConfig, makeItem, mockApi, type MockApi } from '../test/mockApi';
import { render } from '../test/render';
import { findRow, renderSidebar } from '../test/sidebarHarness';
import { resetWarmup, useBoardRowWarmup } from './warmup';

const BASE = '/inkboard/abc123def456/';

function Warmup() {
  useBoardRowWarmup();
  return null;
}

function setup(): MockApi {
  const c = makeCollection({ id: 'c1' });
  const board = makeItem({ id: 'b1', collection_id: 'c1', kind: 'board', title: 'Sketch' });
  const doc = makeItem({ id: 'd1', collection_id: 'c1', title: 'Notes', sort_key: 'a1' });
  const inkboard = { base: BASE, engine_version: 'x', format_version: 2, wasm_bytes: { raw: 1, br: 1, gz: 1 } };
  const api = mockApi().on('GET', '/config', [200, makeConfig({ inkboard })]);
  renderSidebar({ collections: [c], trees: { c1: [board, doc] }, expanded: ['c1'], api });
  return api;
}

function engineFetches(api: MockApi): string[] {
  return api.calls('GET').map((r) => r.path).filter((p) => p.startsWith('/inkboard/'));
}

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

afterEach(() => resetWarmup());

describe('board row warm-up', () => {
  it('a 300 ms hover on a board row fetches the engine; a doc row or a shorter hover does not', async () => {
    const api = setup();
    render(<Warmup />);
    await waitFor(() => expect(api.calls('GET', '/config').length).toBeGreaterThan(0));
    const doc = await findRow('Notes');
    fireEvent.mouseEnter(doc);
    await pause(400);
    fireEvent.mouseLeave(doc);
    const board = await findRow('Sketch');
    fireEvent.mouseEnter(board);
    await pause(100);
    fireEvent.mouseLeave(board);
    await pause(400);
    expect(engineFetches(api)).toEqual([]);
    fireEvent.mouseEnter(board);
    await waitFor(() => expect(engineFetches(api)).toEqual([`${BASE}inkboard_bg.wasm`, `${BASE}inkboard.js`]));
  });
});
