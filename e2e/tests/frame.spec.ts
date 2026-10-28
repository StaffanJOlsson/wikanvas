// The Inkboard frame test (docs/ARCHITECTURE.md §14 "Inkboard patch", PLAN W5): web/dist-inkboard served by
// e2e/serve-static.mjs, frame.html opened directly, and window.quireBoardFrame (§10.4) driven like BoardFrameHost does.
// Run it only through `scripts/e2e.sh frame`, after `scripts/dev.sh scripts/inkboard.sh build` (the stub dist has no
// engine: every test but the first then fails by design).
//
// Coordinates are CSS pixels of the 1280x720 page. The fixture's single 240x120 rectangle is fitted at zoom 2 around
// the canvas centre (640, 360), so it covers x 400-880, y 240-480; EMPTY is a free area right of it, clear of Inkboard's
// floating cards (menu and tools at the top, the style panel on the left, zoom and pages at the bottom, help bottom right).
import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

const FIXTURE = readFileSync(new URL('../../fixtures/boards/frame-test.inkboard', import.meta.url), 'utf8');
const FIXTURE_NAME = 'Frame test';
const CENTRE = { x: 640, y: 360 };
const EMPTY = { from: { x: 960, y: 300 }, to: { x: 1140, y: 430 } };

/** A second board: two rectangles, another page id and another name. */
function otherBoard(): string {
  const board = JSON.parse(FIXTURE);
  const page = board.document.pages[0];
  const box = page.objects[0];
  board.document.name = 'Other board';
  page.id = 'frametest2';
  page.objects = [
    { ...box, id: 'otherbox1', kind: { ...box.kind, frame: { ...box.kind.frame, x: -300 }, label: { ...box.kind.label, text: [{ text: 'One' }] } } },
    { ...box, id: 'otherbox2', kind: { ...box.kind, frame: { ...box.kind.frame, x: 60 }, label: { ...box.kind.label, text: [{ text: 'Two' }] } } },
  ];
  return JSON.stringify(board);
}

/** The board without the per-page view, which the engine refits to the canvas on open. */
function withoutViews(json: string): unknown {
  const board = JSON.parse(json);
  for (const page of board.document.pages) delete page.view;
  return board;
}

function objectCount(json: string): number {
  return JSON.parse(json).document.pages.reduce((n: number, p: { objects: unknown[] }) => n + p.objects.length, 0);
}

type Log = {
  progress: { loaded: number; total: number }[];
  ready: string[];
  change: { itemId: string; revision: number }[];
  save: string[];
  error: { itemId: string | null; kind: string; message: string }[];
  hostKey: { key: string; mod: boolean; shift: boolean; alt: boolean }[];
};

/** Opens frame.html and connects, recording every event in window.__log; window.__api is the BoardFrameApi. */
async function connect(page: Page): Promise<void> {
  const response = await page.goto('/frame.html');
  expect(response?.status()).toBe(200);
  await page.evaluate(async () => {
    const w = window as any;
    const log = { progress: [], ready: [], change: [], save: [], error: [], hostKey: [] } as any;
    w.__log = log;
    w.__api = await w.quireBoardFrame.connect({
      onProgress: (p: unknown) => log.progress.push(p),
      onReady: (id: string) => log.ready.push(id),
      onChange: (e: unknown) => log.change.push(e),
      onSave: (id: string) => log.save.push(id),
      onError: (e: unknown) => log.error.push(e),
      onHostKey: (k: unknown) => log.hostKey.push(k),
    });
  });
}

function log(page: Page): Promise<Log> {
  return page.evaluate(() => (window as any).__log);
}

async function open(page: Page, itemId: string, board: string | null, extra: { name?: string; readOnly?: boolean } = {}) {
  await page.evaluate(
    (a) => (window as any).__api.open(a),
    { itemId, board, name: extra.name ?? FIXTURE_NAME, readOnly: extra.readOnly ?? false, theme: 'light' as const },
  );
}

function snapshot(page: Page): Promise<{ itemId: string; board: string; revision: number }> {
  return page.evaluate(() => (window as any).__api.snapshot());
}

function call<T>(page: Page, method: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([m, a]) => (window as any).__api[m as string](...(a as unknown[])), [method, args] as const);
}

/** A key pressed on the focused canvas, then one frame or two for the engine to see it. */
async function press(page: Page, key: string) {
  await page.keyboard.press(key);
  await page.waitForTimeout(150);
}

/** Drags with the left button in small steps, so that the engine sees the gesture over several frames. */
async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y);
  await page.waitForTimeout(50);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) {
    await page.mouse.move(from.x + ((to.x - from.x) * i) / 10, from.y + ((to.y - from.y) * i) / 10);
    await page.waitForTimeout(30);
  }
  await page.mouse.up();
  await page.waitForTimeout(150);
}

/** R (the rectangle tool, keymap.rs) and a drag in the empty area. */
async function drawRectangle(page: Page) {
  await page.locator('#board').focus();
  await press(page, 'r');
  await drag(page, EMPTY.from, EMPTY.to);
}

async function lastRevision(page: Page): Promise<number> {
  const changes = (await log(page)).change;
  return changes.length === 0 ? -1 : changes[changes.length - 1].revision;
}

test.describe('Inkboard frame', () => {
  test('serves the dist and loads the engine', async ({ page, request }) => {
    const manifest = await (await request.get('/manifest.json')).json();
    expect(manifest.format_version).toBe(2);
    expect(manifest.engine_version).toMatch(/^0\.2\.0-beta\.3\+embed\.quire\.[0-9A-Za-z.]+$/);
    const wasm = await request.get('/inkboard_bg.wasm');
    expect(wasm.headers()['content-type']).toBe('application/wasm');

    await connect(page);
    const progress = (await log(page)).progress;
    expect(progress.length).toBeGreaterThan(1);
    const last = progress[progress.length - 1];
    expect(last.total).toBe(manifest.files['inkboard_bg.wasm'].raw);
    expect(last.loaded).toBe(last.total);
    expect(await page.evaluate(() => (window as any).__api.engineVersion)).toBe(manifest.engine_version);
    expect(await page.evaluate(() => (window as any).__api.formatVersion)).toBe(2);
    expect((await log(page)).error).toEqual([]);
  });

  test('open, snapshot round trip, title, second mount', async ({ page }) => {
    await connect(page);
    const title = await page.title();
    await open(page, 'item-a', FIXTURE);
    expect((await log(page)).ready).toEqual(['item-a']);
    expect(await call(page, 'current')).toBe('item-a');
    const snap = await snapshot(page);
    expect(snap.itemId).toBe('item-a');
    expect(withoutViews(snap.board)).toEqual(withoutViews(FIXTURE));
    expect(await call(page, 'isDirty')).toBe(false);
    expect(await page.title()).toBe(title);

    const second = await page.evaluate(() =>
      (window as any).inkboardEmbed.mount(document.createElement('canvas'), {}).then(
        () => null,
        (e: { kind: string; message: string }) => ({ kind: e.kind, message: e.message }),
      ),
    );
    expect(second).toEqual({ kind: 'start', message: 'already mounted' });
    expect((await log(page)).error).toEqual([]);
  });

  test('draw, onChange, markSaved, Ctrl+S, host keys', async ({ page }) => {
    await connect(page);
    const title = await page.title();
    await open(page, 'item-a', FIXTURE);
    const before = await snapshot(page);
    await drawRectangle(page);
    await expect.poll(() => lastRevision(page)).toBeGreaterThan(before.revision);
    const after = await snapshot(page);
    expect(after.revision).toBeGreaterThan(before.revision);
    expect(objectCount(after.board)).toBe(objectCount(before.board) + 1);
    expect((await log(page)).change.every((c) => c.itemId === 'item-a')).toBe(true);
    expect(await call(page, 'isDirty')).toBe(true);
    expect(await page.title()).toBe(title);

    await call(page, 'markSaved', after.revision);
    expect(await call(page, 'isDirty')).toBe(false);

    await page.locator('#board').focus();
    await press(page, 'Control+s');
    await expect.poll(async () => (await log(page)).save).toEqual(['item-a']);

    await press(page, 'Control+k');
    await press(page, 'Escape');
    await press(page, 'Escape');
    expect((await log(page)).hostKey).toEqual([
      { key: 'k', mod: true, shift: false, alt: false },
      { key: 'Escape', mod: false, shift: false, alt: false },
    ]);
    expect((await log(page)).error).toEqual([]);
  });

  test('read-only blocks the drag; location.hash changes nothing', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE, { readOnly: true });
    const before = await snapshot(page);
    await drawRectangle(page);
    await page.waitForTimeout(500);
    const after = await snapshot(page);
    expect(after.revision).toBe(before.revision);
    expect(objectCount(after.board)).toBe(objectCount(before.board));
    expect(await call(page, 'isDirty')).toBe(false);

    await call(page, 'setReadOnly', false);
    await page.evaluate(() => {
      location.hash = '#join=inkboard-ticket-from-the-address';
    });
    await page.waitForTimeout(500);
    // the page of its own would take an invitation out of the address and open "Join a session"
    expect(await page.evaluate(() => location.hash)).toBe('#join=inkboard-ticket-from-the-address');
    expect((await snapshot(page)).revision).toBe(before.revision);
    expect(await call(page, 'isDirty')).toBe(false);
    expect((await log(page)).error).toEqual([]);
  });

  test('setBoard swaps the board and resets undo', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    await drawRectangle(page);
    await expect.poll(async () => objectCount((await snapshot(page)).board)).toBe(2);

    const other = otherBoard();
    await open(page, 'item-b', other, { name: 'Other board' });
    expect(await call(page, 'current')).toBe('item-b');
    expect((await log(page)).ready).toEqual(['item-a', 'item-b']);
    const swapped = await snapshot(page);
    expect(swapped.itemId).toBe('item-b');
    expect(withoutViews(swapped.board)).toEqual(withoutViews(other));
    expect(await call(page, 'isDirty')).toBe(false);

    // the rectangle drawn on item-a is not in item-b's history
    await page.locator('#board').focus();
    await press(page, 'Control+z');
    await page.waitForTimeout(300);
    const undone = await snapshot(page);
    expect(undone.revision).toBe(swapped.revision);
    expect(withoutViews(undone.board)).toEqual(withoutViews(other));
    expect((await log(page)).error).toEqual([]);
  });

  test('setBoard while a label is being edited', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    await page.mouse.dblclick(CENTRE.x, CENTRE.y);
    await page.waitForTimeout(300);
    await page.keyboard.type(' world', { delay: 30 });
    await page.waitForTimeout(300);
    expect(await call(page, 'isDirty')).toBe(true);
    // the text being typed is part of the snapshot
    expect((await snapshot(page)).board).toContain('Hello world');

    await open(page, 'item-b', otherBoard(), { name: 'Other board' });
    await call(page, 'markSaved', (await snapshot(page)).revision);
    expect(await call(page, 'isDirty')).toBe(false);
    await page.waitForTimeout(300);
    expect(await call(page, 'isDirty')).toBe(false);
    expect((await snapshot(page)).board).not.toContain('Hello world');
    expect((await log(page)).error).toEqual([]);
  });

  test('F5 does not present: R and a drag still draw', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    await page.evaluate(() => {
      (window as any).__notReloaded = true;
    });
    await page.locator('#board').focus();
    await press(page, 'F5');
    await page.waitForTimeout(300);
    expect(await page.evaluate(() => (window as any).__notReloaded)).toBe(true);
    const before = await snapshot(page);
    await drawRectangle(page);
    await expect.poll(async () => objectCount((await snapshot(page)).board)).toBe(objectCount(before.board) + 1);
    expect((await log(page)).error).toEqual([]);
  });

  test('setActive(false) stops repaints', async ({ page }) => {
    // eframe asks for an animation frame on every display frame and paints only when a repaint is due, so the paints
    // are counted as WebGL draw calls; the requestAnimationFrame calls are counted too, for the report
    await page.addInitScript(() => {
      const w = window as any;
      w.__paints = 0;
      w.__frames = 0;
      for (const proto of [WebGL2RenderingContext.prototype, WebGLRenderingContext.prototype] as any[]) {
        for (const name of ['drawElements', 'drawArrays']) {
          const original = proto[name];
          proto[name] = function (this: unknown, ...args: unknown[]) {
            w.__paints++;
            return original.apply(this, args);
          };
        }
      }
      const raf = window.requestAnimationFrame.bind(window);
      window.requestAnimationFrame = (cb) => {
        w.__frames++;
        return raf(cb);
      };
    });
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    const counts = () => page.evaluate(() => ({ paints: (window as any).__paints, frames: (window as any).__frames }));
    // active and idle: the periodic repaint (every 500 ms) keeps painting
    let start = await counts();
    await page.waitForTimeout(2000);
    let end = await counts();
    expect(end.paints).toBeGreaterThan(start.paints);

    await call(page, 'setActive', false);
    await page.waitForTimeout(700);
    start = await counts();
    await page.waitForTimeout(2000);
    end = await counts();
    test.info().annotations.push({ type: 'requestAnimationFrame calls while inactive', description: String(end.frames - start.frames) });
    expect(end.paints).toBe(start.paints);

    await call(page, 'setActive', true);
    await expect.poll(async () => (await counts()).paints).toBeGreaterThan(end.paints);
    expect((await log(page)).error).toEqual([]);
  });

  test('a board opened while the page is hidden mounts once it is visible (emulated visibility)', async ({ page }) => {
    // frame.js's own wait, with document.visibilityState under the test's control
    await page.addInitScript(() => {
      const w = window as any;
      w.__visibility = 'visible';
      Object.defineProperty(Document.prototype, 'visibilityState', { configurable: true, get: () => w.__visibility });
      Object.defineProperty(Document.prototype, 'hidden', { configurable: true, get: () => w.__visibility !== 'visible' });
    });
    await connect(page);
    await page.evaluate(() => {
      const w = window as any;
      w.__visibility = 'hidden';
      document.dispatchEvent(new Event('visibilitychange'));
      w.__opened = false;
      w.__api
        .open({ itemId: 'item-a', board: null, name: 'Hidden', readOnly: false, theme: 'light' })
        .then(() => (w.__opened = true));
    });
    await page.waitForTimeout(1500);
    expect(await page.evaluate(() => (window as any).__opened)).toBe(false);
    expect(await call(page, 'current')).toBe(null);
    expect((await log(page)).ready).toEqual([]);
    expect(await call(page, 'isDirty')).toBe(false);

    await page.evaluate(() => {
      (window as any).__visibility = 'visible';
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await expect.poll(() => page.evaluate(() => (window as any).__opened)).toBe(true);
    expect(await call(page, 'current')).toBe('item-a');
    expect((await log(page)).ready).toEqual(['item-a']);
    expect(objectCount((await snapshot(page)).board)).toBe(0);
  });

  test('background page for 60 s', async ({ page, context }) => {
    test.setTimeout(180_000);
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    await drawRectangle(page);
    await expect.poll(() => call(page, 'isDirty')).toBe(true);

    const other = await context.newPage();
    await other.goto('/manifest.json');
    await other.bringToFront();
    const state = await page.evaluate(() => document.visibilityState);
    test.skip(
      state === 'visible',
      'this Chromium reports visibilityState "visible" for a background page: the real background case is a manual browser check (ARCHITECTURE §16)',
    );
    await page.waitForTimeout(60_000);
    const snap = await snapshot(page);
    expect(objectCount(snap.board)).toBe(objectCount(FIXTURE) + 1);
    expect((await log(page)).error).toEqual([]);

    // a fresh frame, connected and opened while hidden, mounts only once it is in front again
    const late = await context.newPage();
    await connect(late);
    await other.bringToFront();
    expect(await late.evaluate(() => document.visibilityState)).toBe('hidden');
    await late.evaluate(() => {
      const w = window as any;
      w.__opened = false;
      w.__api.open({ itemId: 'item-c', board: null, name: 'Late', readOnly: false, theme: 'light' }).then(() => (w.__opened = true));
    });
    await late.waitForTimeout(2000);
    expect(await late.evaluate(() => (window as any).__opened)).toBe(false);
    await late.bringToFront();
    await expect.poll(() => late.evaluate(() => (window as any).__opened)).toBe(true);
    expect(await late.evaluate(() => (window as any).__api.current())).toBe('item-c');
  });
});
