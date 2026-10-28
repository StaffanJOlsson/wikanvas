// The Inkboard frame test (docs/ARCHITECTURE.md §14 "Inkboard patch", PLAN W5): web/dist-inkboard served by
// e2e/serve-static.mjs, frame.html opened directly, and window.quireBoardFrame (§10.4) driven like BoardFrameHost does.
// Run it only through `scripts/e2e.sh frame`, after `scripts/dev.sh scripts/inkboard.sh build` (or `build --quick`).
// The stub dist (`scripts/inkboard.sh stub`) has no engine: the beforeAll hook then fails once with that message, and no
// test runs.
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

/**
 * Opens frame.html (served with the frame CSP of §11, like quire-server does) and connects, recording every event in
 * window.__log and every CSP violation in window.__csp; window.__api is the BoardFrameApi.
 */
async function connect(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const w = window as any;
    w.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => w.__csp.push(`${e.violatedDirective} ${e.blockedURI}`));
  });
  const response = await page.goto('/frame.html');
  expect(response?.headers()['content-security-policy']).toContain("script-src 'self' 'wasm-unsafe-eval'");
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

/** No onError and no CSP violation so far. */
async function expectClean(page: Page) {
  expect((await log(page)).error).toEqual([]);
  expect(await page.evaluate(() => (window as any).__csp)).toEqual([]);
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

/** What connect() rejects with, and the onError events it sent, for a page opened without the helper above. */
async function connectOnce(page: Page): Promise<{ result: unknown; errors: unknown[] }> {
  return page.evaluate(async () => {
    const errors: unknown[] = [];
    const result = await (window as any).quireBoardFrame
      .connect({ onError: (e: unknown) => errors.push(e) })
      .then((api: { engineVersion: string }) => ({ engineVersion: api.engineVersion }), (e: unknown) => ({ rejected: e }));
    return { result, errors };
  });
}

test.describe('Inkboard frame', () => {
  // One clear failure instead of a failure per test when web/dist-inkboard is not a real engine build.
  test.beforeAll(async ({ playwright }, testInfo) => {
    const build = 'run scripts/dev.sh scripts/inkboard.sh build first';
    const request = await playwright.request.newContext({ baseURL: testInfo.project.use.baseURL });
    try {
      const response = await request.get('/manifest.json');
      if (response.status() !== 200) throw new Error(`web/dist-inkboard has no manifest.json (HTTP ${response.status()}): ${build}`);
      const manifest = await response.json();
      if (manifest.engine_version === 'stub') throw new Error(`web/dist-inkboard is the stub: ${build}`);
    } finally {
      await request.dispose();
    }
  });

  test('serves the dist and loads the engine', async ({ page, request }) => {
    const manifest = await (await request.get('/manifest.json')).json();
    expect(manifest.format_version).toBe(2);
    expect(manifest.engine_version).toMatch(/^0\.2\.0-beta\.3\+embed\.quire\.[0-9A-Za-z.]+$/);
    const wasm = await request.get('/inkboard_bg.wasm', { headers: { 'Accept-Encoding': 'br' } });
    expect(wasm.headers()['content-type']).toBe('application/wasm');
    expect(wasm.headers()['content-encoding']).toBe('br');

    await connect(page);
    const progress = (await log(page)).progress;
    expect(progress.length).toBeGreaterThan(1);
    const last = progress[progress.length - 1];
    expect(last.total).toBe(manifest.files['inkboard_bg.wasm'].raw);
    expect(last.loaded).toBe(last.total);
    expect(await page.evaluate(() => (window as any).__api.engineVersion)).toBe(manifest.engine_version);
    expect(await page.evaluate(() => (window as any).__api.formatVersion)).toBe(2);
    await expectClean(page);
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
    await expectClean(page);
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
    await expectClean(page);
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
    await expectClean(page);
  });

  test('setBoard swaps the board and resets undo', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    const count = async () => objectCount((await snapshot(page)).board);
    await drawRectangle(page);
    await expect.poll(count).toBe(2);
    // the control: in this setup Ctrl+Z reaches the engine and takes the rectangle back
    await page.locator('#board').focus();
    await press(page, 'Control+z');
    await expect.poll(count).toBe(1);
    // drawn again, so item-a's history has a step to undo when the board is swapped
    await drawRectangle(page);
    await expect.poll(count).toBe(2);

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
    // and the keys still reach the engine after the swap
    await press(page, 'Control+s');
    await expect.poll(async () => (await log(page)).save).toEqual(['item-b']);
    await expectClean(page);
  });

  test('a later open() with an unreadable board keeps the previous board', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    await drawRectangle(page);
    await expect.poll(async () => objectCount((await snapshot(page)).board)).toBe(2);
    const before = await snapshot(page);

    const rejected = await page.evaluate(() =>
      (window as any).__api
        .open({ itemId: 'item-b', board: '{"document": ', name: 'Broken', readOnly: true, theme: 'dark' })
        .then(() => null, (e: { kind: string }) => e.kind),
    );
    expect(rejected).toBe('load');
    const errors = (await log(page)).error;
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatchObject({ itemId: 'item-b', kind: 'load' });
    expect(await call(page, 'current')).toBe('item-a');
    const after = await snapshot(page);
    expect(after.itemId).toBe('item-a');
    expect(after.revision).toBe(before.revision);
    expect(withoutViews(after.board)).toEqual(withoutViews(before.board));
    expect(await call(page, 'isDirty')).toBe(true);
    expect((await log(page)).ready).toEqual(['item-a']);

    // the open() calls after a failed one still run
    await open(page, 'item-c', FIXTURE);
    expect(await call(page, 'current')).toBe('item-c');
    expect((await log(page)).ready).toEqual(['item-a', 'item-c']);
    expect(await page.evaluate(() => (window as any).__csp)).toEqual([]);
  });

  test('a WebAssembly.RuntimeError reaching the page is reported as one panic', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', FIXTURE);
    const seen = () => page.evaluate(() => (window as any).__seen);
    // the backup listeners (window `error` and `unhandledrejection`), for a trap the engine's panic hook did not report
    await page.evaluate(() => {
      const w = window as any;
      w.__seen = 0;
      window.addEventListener('error', () => w.__seen++);
      window.addEventListener('unhandledrejection', () => w.__seen++);
      setTimeout(() => {
        throw new Error('not a trap');
      });
      setTimeout(() => {
        throw new WebAssembly.RuntimeError('unreachable executed');
      });
    });
    await expect.poll(seen).toBe(2);
    expect((await log(page)).error).toEqual([{ itemId: 'item-a', kind: 'panic', message: 'unreachable executed' }]);

    // later traps, thrown or rejected, add nothing
    await page.evaluate(() => {
      setTimeout(() => {
        throw new WebAssembly.RuntimeError('a second trap');
      });
      void Promise.reject(new WebAssembly.RuntimeError('a trap in a promise'));
    });
    await expect.poll(seen).toBe(4);
    await page.waitForTimeout(200);
    expect((await log(page)).error).toHaveLength(1);

    // a stopped engine opens nothing more, and says so without another onError
    const rejected = await page.evaluate(() =>
      (window as any).__api
        .open({ itemId: 'item-b', board: null, name: 'After', readOnly: false, theme: 'light' })
        .then(() => null, (e: unknown) => e),
    );
    expect(rejected).toEqual({ kind: 'panic', message: 'the whiteboard engine stopped' });
    expect((await log(page)).error).toHaveLength(1);
    expect(await call(page, 'current')).toBe('item-a');
  });

  test('connect() may be retried after a failed download', async ({ page }) => {
    let requests = 0;
    await page.route('**/manifest.json', async (route) => {
      requests++;
      if (requests === 1) await route.fulfill({ status: 500, contentType: 'text/plain', body: 'unavailable' });
      else await route.continue();
    });
    await page.goto('/frame.html');
    const manifest = await (await page.request.get('/manifest.json')).json();

    const first = await connectOnce(page);
    const failure = { kind: 'download', message: 'manifest.json: HTTP 500' };
    expect(first).toEqual({ result: { rejected: failure }, errors: [{ itemId: null, ...failure }] });
    const second = await connectOnce(page);
    expect(second).toEqual({ result: { engineVersion: manifest.engine_version }, errors: [] });
    expect(requests).toBe(2);
    // a connected frame answers every later connect() with the same API
    expect(
      await page.evaluate(async () => {
        const frame = (window as any).quireBoardFrame;
        return (await frame.connect({})) === (await frame.connect({}));
      }),
    ).toBe(true);
  });

  test('connect() refuses an engine whose format differs from the manifest, for good', async ({ page }) => {
    await page.route('**/manifest.json', async (route) => {
      const manifest = await (await route.fetch()).json();
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...manifest, format_version: 3 }) });
    });
    await page.goto('/frame.html');
    const failure = { kind: 'start', message: 'the engine writes format 2, the manifest says 3' };
    expect(await connectOnce(page)).toEqual({ result: { rejected: failure }, errors: [{ itemId: null, ...failure }] });
    // not a download failure: connecting again gives the same rejection, without trying again
    expect(await connectOnce(page)).toEqual({ result: { rejected: failure }, errors: [] });
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
    await expectClean(page);
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
    await expectClean(page);
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
    await expectClean(page);
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
    await expectClean(page);

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
