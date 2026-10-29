// The Inkboard frame test (docs/ARCHITECTURE.md §14 "Inkboard patch", PLAN W5): web/dist-inkboard served by
// e2e/serve-static.mjs, frame.html opened directly, and window.wikanvasBoardFrame (§10.4) driven like BoardFrameHost
// does. Run it only through `scripts/e2e.sh frame`, after `scripts/dev.sh scripts/inkboard.sh build` (or
// `build --quick`). The stub dist (`scripts/inkboard.sh stub`) has no engine: the beforeAll hook then fails once with
// that message, and no test runs.
//
// Coordinates are CSS pixels of the 1280x720 page. The fixture's single 240x120 rectangle is fitted at zoom 2 around
// the canvas centre (640, 360), so it covers x 400-880, y 240-480; EMPTY is a free area right of it, clear of Inkboard's
// floating cards (menu and tools at the top, the style panel on the left, zoom and pages at the bottom, help bottom right).
//
// The "embeds:" tests (PLAN M1.1 E1, ARCHITECTURE §18.3-§18.4) open fixtures/boards/embed.inkboard: two 480x360 cards side by side
// and a plain box under them, fitted at zoom ~1.07, so card 1 covers about x 82-597, y 70-457 and card 2 x 683-1199; EMBED_EMPTY is
// a free area under card 1, left of the plain box (x 511-769, y 521-650). Points on a board are computed from the reported layout
// (screen = origin + world * zoom), and each test checks that its points are on the canvas, clear of Inkboard's floating cards.
import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

const FIXTURE = readFileSync(new URL('../../fixtures/boards/frame-test.inkboard', import.meta.url), 'utf8');
const EMBED = readFileSync(new URL('../../fixtures/boards/embed.inkboard', import.meta.url), 'utf8');
const ITEM1 = '0192f0e0-0000-7000-8000-000000000001';
const ITEM2 = '0192f0e0-0000-7000-8000-000000000002';
const ITEM3 = '0192f0e0-0000-7000-8000-000000000003';
const EMBED_EMPTY = { from: { x: 300, y: 520 }, to: { x: 450, y: 620 } };
const DRAG_TYPE = 'application/x-wikanvas-item';
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

type Rect = { x: number; y: number; width: number; height: number };
type Point = { x: number; y: number };
type EmbedBox = {
  objectId: string; item: string; x: number; y: number; w: number; h: number; angle: number; label: string; selected: boolean; locked: boolean;
};
type Layout = {
  itemId: string; page: string; zoom: number; originX: number; originY: number; canvas: Rect; occluders: Rect[]; embeds: EmbedBox[];
  items: string[]; clipboard: string[]; editing: string | null;
};

type Log = {
  progress: { loaded: number; total: number }[];
  ready: string[];
  change: { itemId: string; revision: number }[];
  save: string[];
  error: { itemId: string | null; kind: string; message: string }[];
  hostKey: { key: string; mod: boolean; shift: boolean; alt: boolean }[];
  layout: Layout[];
  open: { itemId: string; objectId: string; item: string; point: Point | null }[];
  end: { itemId: string; objectId: string; reason: string }[];
  focusBack: { itemId: string; objectId: string }[];
  drop: { itemId: string; item: unknown; point: Point }[];
};

/**
 * Opens frame.html (served with the frame CSP of §11, like wikanvas-server does) and connects, recording every event in
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
    const log = { progress: [], ready: [], change: [], save: [], error: [], hostKey: [], layout: [], open: [], end: [], focusBack: [], drop: [] } as any;
    w.__log = log;
    w.__api = await w.wikanvasBoardFrame.connect({
      onProgress: (p: unknown) => log.progress.push(p),
      onReady: (id: string) => log.ready.push(id),
      onChange: (e: unknown) => log.change.push(e),
      onSave: (id: string) => log.save.push(id),
      onError: (e: unknown) => log.error.push(e),
      onHostKey: (k: unknown) => log.hostKey.push(k),
      onEmbedLayout: (e: unknown) => log.layout.push(e),
      onEmbedOpen: (e: unknown) => log.open.push(e),
      onEmbedEditEnd: (e: unknown) => log.end.push(e),
      onEmbedFocusBack: (e: unknown) => log.focusBack.push(e),
      onDropItem: (e: unknown) => log.drop.push(e),
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
    const result = await (window as any).wikanvasBoardFrame
      .connect({ onError: (e: unknown) => errors.push(e) })
      .then((api: { engineVersion: string }) => ({ engineVersion: api.engineVersion }), (e: unknown) => ({ rejected: e }));
    return { result, errors };
  });
}

/* ---------------------------------------------------------------------------------------------- board document embeds */

/** The last layout reported, once no new one came for 400 ms (the floating cards settle over the first frames). */
async function settledLayout(page: Page): Promise<Layout> {
  let count = -1;
  await expect
    .poll(
      async () => {
        const n = (await log(page)).layout.length;
        const settled = n > 0 && n === count;
        count = n;
        return settled;
      },
      { intervals: [400], timeout: 15_000 },
    )
    .toBe(true);
  const layouts = (await log(page)).layout;
  return layouts[layouts.length - 1];
}

async function lastLayout(page: Page): Promise<Layout | undefined> {
  const layouts = (await log(page)).layout;
  return layouts[layouts.length - 1];
}

/** A world point in frame pixels. */
function screen(l: Layout, x: number, y: number): Point {
  return { x: l.originX + x * l.zoom, y: l.originY + y * l.zoom };
}

function inside(r: Rect, p: Point): boolean {
  return p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height;
}

/** The point is on the canvas and clear of Inkboard's own interface (the test's choice of points is still right). */
function expectClear(l: Layout, p: Point, what: string) {
  expect(inside(l.canvas, p), `${what} is on the canvas`).toBe(true);
  const over = l.occluders.filter((r) => inside(r, p));
  expect(over, `${what} at ${p.x},${p.y} is under the engine's interface`).toEqual([]);
}

/** The board without the per-page views, as text (the engine refits each page's view on open). */
function withoutViewText(json: string): string {
  return json.replace(/"view":\{[^}]*\}/g, '"view":null');
}

/** How many pixels of the frame-pixel rectangle are darker than #808080 (every channel below 0x80). */
async function darkPixels(page: Page, r: Rect): Promise<number> {
  const png = (await page.screenshot({ clip: r })).toString('base64');
  return page.evaluate(async (data) => {
    const img = new Image();
    img.src = `data:image/png;base64,${data}`;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext('2d')!;
    ctx.drawImage(img, 0, 0);
    const px = ctx.getImageData(0, 0, c.width, c.height).data;
    let dark = 0;
    for (let i = 0; i < px.length; i += 4) if (px[i] < 0x80 && px[i + 1] < 0x80 && px[i + 2] < 0x80) dark++;
    return dark;
  }, png);
}

/** A variant of the embed fixture: `f` edits its parsed board. */
function embedVariant(f: (board: any) => void): string {
  const board = JSON.parse(EMBED);
  f(board);
  return JSON.stringify(board);
}

/** Synthetic drag events at a frame point, as a sidebar row's drag would bring them; whether each was accepted. */
async function dispatchDrag(page: Page, data: Record<string, string>, point: Point, types: string[] = ['dragenter', 'dragover', 'drop']) {
  return page.evaluate(
    ({ data, point, types }) => {
      const target = document.getElementById('board')!;
      const dt = new DataTransfer();
      for (const [type, value] of Object.entries(data)) dt.setData(type, value);
      const accepted: Record<string, boolean> = {};
      for (const type of types) {
        const e = new DragEvent(type, { dataTransfer: dt, clientX: point.x, clientY: point.y, bubbles: true, cancelable: true });
        target.dispatchEvent(e);
        accepted[type] = e.defaultPrevented;
      }
      return accepted;
    },
    { data, point, types },
  );
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
    expect(manifest.engine_version).toMatch(/^0\.2\.0-beta\.3\+embed\.wikanvas\.[0-9A-Za-z.]+$/);
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
        const frame = (window as any).wikanvasBoardFrame;
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


  test('embeds: the layout of the fixture', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    expect((await log(page)).layout.length).toBeGreaterThanOrEqual(1);
    expect(l.itemId).toBe('item-a');
    expect(l.page).toBe('embedtest1');
    expect(l.embeds).toEqual([
      { objectId: 'embedcard1', item: ITEM1, x: -520, y: -200, w: 480, h: 360, angle: 0, label: 'First page', selected: false, locked: false },
      { objectId: 'embedcard2', item: ITEM2, x: 40, y: -200, w: 480, h: 360, angle: 0, label: 'Second page', selected: false, locked: false },
    ]);
    expect(l.items).toEqual([ITEM1, ITEM2]);
    expect(l.clipboard).toEqual([]);
    expect(l.editing).toBeNull();
    expect(l.canvas).toEqual({ x: 0, y: 0, width: 1280, height: 720 });
    expect(l.occluders.length).toBeGreaterThan(0);
    for (const r of l.occluders) {
      expect(r.width > 0 && r.height > 0).toBe(true);
      expect(r.x >= l.canvas.x && r.y >= l.canvas.y && r.x + r.width <= l.canvas.x + l.canvas.width + 0.01).toBe(true);
      expect(r.y + r.height <= l.canvas.y + l.canvas.height + 0.01).toBe(true);
    }
    // the camera formula: the engine's centred title snapshot is where origin + centre * zoom says, inside the white card
    for (const card of l.embeds) {
      const c = screen(l, card.x + card.w / 2, card.y + card.h / 2);
      const above = screen(l, card.x + card.w / 2, card.y + card.h / 2 - 120);
      expectClear(l, c, `the centre of ${card.objectId}`);
      expect(await darkPixels(page, { x: c.x - 40, y: c.y - 12, width: 80, height: 24 })).toBeGreaterThan(20);
      expect(await darkPixels(page, { x: above.x - 40, y: above.y - 12, width: 80, height: 24 })).toBe(0);
    }
    await expectClean(page);
  });

  test('embeds: pan and zoom report at most once per frame', async ({ page }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__frames = 0;
      const raf = window.requestAnimationFrame.bind(window);
      window.requestAnimationFrame = (cb) =>
        raf((t) => {
          w.__frames++;
          cb(t);
        });
    });
    await connect(page);
    await open(page, 'item-a', EMBED);
    const start = await settledLayout(page);
    const counts = async () => ({ layouts: (await log(page)).layout.length, frames: await page.evaluate(() => (window as any).__frames) });
    const before = await counts();
    // a wheel zoom over the empty area, then a pan with the Hand tool
    const empty = EMBED_EMPTY.from;
    expectClear(start, empty, 'the empty area');
    await page.mouse.move(empty.x, empty.y);
    for (let i = 0; i < 4; i++) {
      await page.mouse.wheel(0, -120);
      await page.waitForTimeout(60);
    }
    await page.locator('#board').focus();
    await press(page, 'h');
    await drag(page, EMBED_EMPTY.from, EMBED_EMPTY.to);
    const after = await counts();
    const moved = (await lastLayout(page))!;
    test.info().annotations.push({ type: 'layouts / animation frames during the gesture', description: `${after.layouts - before.layouts} / ${after.frames - before.frames}` });
    expect(moved.zoom).toBeGreaterThan(start.zoom * 1.05);
    expect(Math.abs(moved.originX - start.originX) + Math.abs(moved.originY - start.originY)).toBeGreaterThan(50);
    expect(after.layouts - before.layouts).toBeGreaterThan(1);
    expect(after.layouts - before.layouts).toBeLessThanOrEqual(after.frames - before.frames + 2);
    // idle: no report
    await page.waitForTimeout(1000);
    expect((await counts()).layouts).toBe(after.layouts);

    // a board without cards reports no pan at all
    await open(page, 'item-b', FIXTURE);
    await settledLayout(page);
    const plain = (await log(page)).layout.length;
    await page.locator('#board').focus();
    await press(page, 'h');
    await drag(page, EMPTY.from, EMPTY.to);
    await page.waitForTimeout(500);
    expect((await log(page)).layout.length).toBe(plain);
    await expectClean(page);
  });

  test('embeds: open instead of label editing', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    const before = await snapshot(page);
    const p = screen(l, -280, 60);
    expectClear(l, p, 'card 1');
    await page.mouse.dblclick(p.x, p.y);
    await expect.poll(async () => (await log(page)).open.length).toBe(1);
    const [opened] = (await log(page)).open;
    expect(opened).toMatchObject({ itemId: 'item-a', objectId: 'embedcard1', item: ITEM1 });
    expect(Math.hypot(opened.point!.x - p.x, opened.point!.y - p.y)).toBeLessThanOrEqual(2);
    await expect.poll(async () => (await lastLayout(page))!.embeds[0].selected).toBe(true);
    await page.waitForTimeout(300);
    expect((await log(page)).change).toEqual([]);
    const after = await snapshot(page);
    expect(after.revision).toBe(before.revision);
    expect(after.board).toBe(before.board);
    // Enter on the selected card: no point
    await press(page, 'Enter');
    await expect.poll(async () => (await log(page)).open.length).toBe(2);
    expect((await log(page)).open[1]).toEqual({ itemId: 'item-a', objectId: 'embedcard1', item: ITEM1, point: null });
    // the control: the plain box still edits its label
    const box = screen(l, 0, 280);
    expectClear(l, box, 'the plain box');
    await page.mouse.dblclick(box.x, box.y);
    await page.waitForTimeout(300);
    await page.keyboard.type(' edited', { delay: 30 });
    await page.waitForTimeout(300);
    expect((await snapshot(page)).board).toContain('Plain box edited');
    expect((await log(page)).open).toHaveLength(2);
    await expectClean(page);
  });

  test('embeds: the editing state', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    const card = screen(l, -280, 60);
    expectClear(l, card, 'card 1');
    expectClear(l, EMBED_EMPTY.from, 'the empty area');
    // selected and copied first, so that Delete and Ctrl+V would have something to act on
    await page.mouse.click(card.x, card.y);
    await press(page, 'Control+c');
    // the layout names the copied card's document: it can still be pasted, so the host keeps it
    await expect.poll(async () => (await lastLayout(page))!.clipboard).toEqual([ITEM1]);
    const editing = async () => (await lastLayout(page))!.editing;
    await call(page, 'setEmbedEditing', 'embedcard1');
    await expect.poll(editing).toBe('embedcard1');
    const before = await snapshot(page);
    // the canvas still has the focus: the keys reach the engine, which handles none of them
    await page.locator('#board').focus();
    await press(page, 'r');
    await press(page, 'Delete');
    await press(page, 'Control+v');
    await page.waitForTimeout(300);
    expect((await log(page)).change).toEqual([]);
    expect((await snapshot(page)).board).toBe(before.board);
    expect((await log(page)).end).toEqual([]);

    // a click on empty canvas ends it, and does nothing else: no marquee, the card stays selected
    await page.mouse.click(EMBED_EMPTY.from.x, EMBED_EMPTY.from.y);
    await expect.poll(async () => (await log(page)).end).toEqual([{ itemId: 'item-a', objectId: 'embedcard1', reason: 'press' }]);
    await expect.poll(editing).toBeNull();
    expect((await lastLayout(page))!.embeds[0].selected).toBe(true);
    // 'r' did not pick the rectangle tool: a drag selects instead of drawing
    await drag(page, EMBED_EMPTY.from, EMBED_EMPTY.to);
    await page.waitForTimeout(300);
    expect((await log(page)).change).toEqual([]);

    // a click on a toolbar button ends it, and the button still works (Hand: the next drag pans)
    await call(page, 'setEmbedEditing', 'embedcard1');
    await expect.poll(editing).toBe('embedcard1');
    const now = (await lastLayout(page))!;
    const tools = now.occluders.find((r) => r.y < 30 && r.x < 640 && r.x + r.width > 640);
    expect(tools, 'the tools card').toBeDefined();
    // the card's border (1) and margin (4), the Select button (34), the spacing (1), half the Hand button (17)
    const hand = { x: tools!.x + 57, y: tools!.y + tools!.height / 2 };
    await page.mouse.click(hand.x, hand.y);
    await expect.poll(async () => (await log(page)).end.length).toBe(2);
    expect((await log(page)).end[1]).toEqual({ itemId: 'item-a', objectId: 'embedcard1', reason: 'press' });
    const unpanned = (await lastLayout(page))!;
    await drag(page, EMBED_EMPTY.from, EMBED_EMPTY.to);
    await expect.poll(async () => (await lastLayout(page))!.originX).not.toBe(unpanned.originX);

    // the frame taking the focus ends it
    await press(page, 'v');
    await call(page, 'setEmbedEditing', 'embedcard1');
    await expect.poll(editing).toBe('embedcard1');
    await page.evaluate(() => {
      const input = document.createElement('input');
      input.id = 'elsewhere';
      document.body.append(input);
      input.focus();
    });
    await page.waitForTimeout(300);
    expect((await log(page)).end).toHaveLength(2);
    await page.locator('#board').focus();
    await expect.poll(async () => (await log(page)).end.length).toBe(3);
    expect((await log(page)).end[2]).toEqual({ itemId: 'item-a', objectId: 'embedcard1', reason: 'focus' });
    await page.evaluate(() => document.getElementById('elsewhere')!.remove());

    // another board: the editing is gone, reported for the board it was on
    await call(page, 'setEmbedEditing', 'embedcard2');
    await expect.poll(editing).toBe('embedcard2');
    await open(page, 'item-b', otherBoard(), { name: 'Other board' });
    await expect.poll(async () => (await log(page)).end.length).toBe(4);
    expect((await log(page)).end[3]).toEqual({ itemId: 'item-a', objectId: 'embedcard2', reason: 'gone' });
    await expect.poll(async () => (await lastLayout(page))!.itemId).toBe('item-b');
    expect((await lastLayout(page))!.editing).toBeNull();
    await expectClean(page);
  });

  test('embeds: while the host edits, Shift and Alt with the wheel pan and the middle button pans, keeping the focus', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    const empty = EMBED_EMPTY.from;
    expectClear(l, empty, 'the empty area');
    await call(page, 'setEmbedEditing', 'embedcard1');
    await expect.poll(async () => (await lastLayout(page))!.editing).toBe('embedcard1');
    // the host's editor has the keyboard focus: the board gets no key event, so it never hears of Shift or Alt
    await page.mouse.move(empty.x, empty.y);
    await page.evaluate(() => {
      // fixed and tiny: an input below the canvas would make the page scroll to it
      const input = document.createElement('input');
      input.id = 'host-editor';
      input.style.cssText = 'position: fixed; right: 0; bottom: 0; width: 1px; height: 1px; opacity: 0';
      document.body.append(input);
      input.focus();
    });
    const camera = async () => {
      await page.waitForTimeout(300);
      const now = (await lastLayout(page))!;
      return { x: now.originX, y: now.originY, zoom: now.zoom };
    };
    const start = await camera();
    await page.keyboard.down('Shift');
    await page.mouse.wheel(0, 120);
    await page.keyboard.up('Shift');
    const shifted = await camera();
    expect(shifted.zoom, 'Shift + wheel does not zoom').toBeCloseTo(start.zoom, 6);
    expect(Math.abs(shifted.x - start.x), 'Shift + wheel pans sideways').toBeGreaterThan(30);
    expect(shifted.y).toBeCloseTo(start.y, 3);
    await page.keyboard.down('Alt');
    await page.mouse.wheel(0, 120);
    await page.keyboard.up('Alt');
    const alted = await camera();
    expect(alted.zoom, 'Alt + wheel does not zoom').toBeCloseTo(start.zoom, 6);
    expect(Math.abs(alted.y - shifted.y), 'Alt + wheel pans up and down').toBeGreaterThan(30);
    expect(alted.x).toBeCloseTo(shifted.x, 3);
    // the middle button pans and the editing goes on: the press leaves the focus in the host's editor, and once eframe has
    // focused its canvas on the release, the host hears that it may take the focus back
    await page.mouse.move(empty.x, empty.y);
    await page.mouse.down({ button: 'middle' });
    for (let i = 1; i <= 10; i++) {
      await page.mouse.move(empty.x + 8 * i, empty.y + 4 * i);
      await page.waitForTimeout(30);
    }
    expect(await page.evaluate(() => document.activeElement?.id), 'the press keeps the focus').toBe('host-editor');
    expect((await log(page)).focusBack).toEqual([]);
    await page.mouse.up({ button: 'middle' });
    const panned = await camera();
    expect(panned.x - alted.x).toBeCloseTo(80, 0);
    expect(panned.y - alted.y).toBeCloseTo(40, 0);
    expect((await log(page)).focusBack).toEqual([{ itemId: 'item-a', objectId: 'embedcard1' }]);
    expect((await log(page)).end).toEqual([]);
    expect((await lastLayout(page))!.editing).toBe('embedcard1');
    // a secondary press ends it like a primary one (its menu opens as usual)
    await page.mouse.click(empty.x + 80, empty.y + 40, { button: 'right' });
    await expect.poll(async () => (await log(page)).end).toEqual([{ itemId: 'item-a', objectId: 'embedcard1', reason: 'press' }]);
    await page.keyboard.press('Escape');
    await expectClean(page);
  });

  test('embeds: insertEmbed', async ({ page }) => {
    await connect(page);
    const early = await page.evaluate(() => {
      try {
        (window as any).__api.insertEmbed({ item: 'x', title: 'x', at: 'center' });
        return null;
      } catch (e) {
        return String((e as Error).message);
      }
    });
    expect(early).toBe('no board is open');
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    const id = await call<string | null>(page, 'insertEmbed', { item: ITEM3, title: 'Meeting notes', at: 'center' });
    expect(typeof id).toBe('string');
    await expect.poll(async () => (await log(page)).change.length).toBeGreaterThan(0);
    const snap = await snapshot(page);
    const objects = JSON.parse(snap.board).document.pages[0].objects;
    const card = objects.find((o: { id: string }) => o.id === id);
    const frame = card.kind.frame;
    expect(card).toEqual({
      id,
      kind: {
        type: 'box',
        geom: 'rect',
        frame: { x: frame.x, y: frame.y, w: 480, h: 360 },
        label: { text: [{ text: 'Meeting notes', bold: true, color: '#1d2430' }], family: 'sans', size: 24, align: 'center' },
        embed: { item: ITEM3 },
      },
      style: { stroke: '#c4cad4', fill: '#ffffff', width: 1, dash: 'solid', opacity: 1, roundness: 0.04, shadow: true },
      layer: 'main',
    });
    // the field order of §18.2, as the engine writes it
    expect(snap.board).toContain(
      `"label":{"text":[{"text":"Meeting notes","bold":true,"color":"#1d2430"}],"family":"sans","size":24.0,"align":"center"},` +
        `"embed":{"item":"${ITEM3}"}},"style":{"stroke":"#c4cad4","fill":"#ffffff","width":1.0,"dash":"solid","opacity":1.0,` +
        `"roundness":0.04,"shadow":true},"layer":"main"}`,
    );
    // centred on the view's centre (the canvas's centre in world units)
    const centre = { x: (640 - l.originX) / l.zoom, y: (360 - l.originY) / l.zoom };
    expect(Math.abs(frame.x + 240 - centre.x)).toBeLessThan(1);
    expect(Math.abs(frame.y + 180 - centre.y)).toBeLessThan(1);
    await expect.poll(async () => (await lastLayout(page))!.embeds.map((e) => e.objectId)).toEqual(['embedcard1', 'embedcard2', id]);
    expect((await lastLayout(page))!.embeds[2].selected).toBe(true);
    expect((await lastLayout(page))!.items).toEqual([ITEM1, ITEM2, ITEM3]);

    // at a frame point
    const at = { x: 900, y: 600 };
    const second = await call<string>(page, 'insertEmbed', { item: ITEM1, title: 'Here', at });
    const placed = JSON.parse((await snapshot(page)).board).document.pages[0].objects.find((o: { id: string }) => o.id === second);
    const want = { x: (at.x - l.originX) / l.zoom, y: (at.y - l.originY) / l.zoom };
    expect(Math.abs(placed.kind.frame.x + 240 - want.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(placed.kind.frame.y + 180 - want.y)).toBeLessThanOrEqual(1);
    // one undo step each
    await page.locator('#board').focus();
    await press(page, 'Control+z');
    await expect.poll(async () => (await snapshot(page)).board.includes(`"id":"${second}"`)).toBe(false);
    expect((await snapshot(page)).board).toContain(`"id":"${id}"`);

    // read-only: null, and nothing added
    await open(page, 'item-c', EMBED, { readOnly: true });
    await settledLayout(page);
    const readOnly = await snapshot(page);
    expect(await call(page, 'insertEmbed', { item: ITEM3, title: 'No', at: 'center' })).toBeNull();
    await page.waitForTimeout(200);
    const unchanged = await snapshot(page);
    expect(unchanged.revision).toBe(readOnly.revision);
    expect(withoutViewText(unchanged.board)).toBe(withoutViewText(readOnly.board));

    // the first layer hidden: the card goes on the second, visible one
    const layered = embedVariant((board) => {
      board.document.pages[0].layers = [
        { id: 'hidden', name: 'Hidden', visible: false, locked: false },
        { id: 'main', name: 'Layer 1', visible: true, locked: false },
      ];
    });
    await open(page, 'item-d', layered);
    const third = await call<string>(page, 'insertEmbed', { item: ITEM3, title: 'Layered', at: 'center' });
    const onLayer = JSON.parse((await snapshot(page)).board).document.pages[0].objects.find((o: { id: string }) => o.id === third);
    expect(onLayer.layer).toBe('main');
    await expect.poll(async () => (await lastLayout(page))!.embeds.some((e) => e.objectId === third)).toBe(true);
    await expectClean(page);
  });

  test('embeds: titles in place', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED);
    await settledLayout(page);
    const before = await snapshot(page);
    await call(page, 'setEmbedTitles', { [ITEM1]: 'Renamed page' });
    await expect.poll(async () => (await lastLayout(page))!.embeds[0].label).toBe('Renamed page');
    expect((await lastLayout(page))!.embeds[1].label).toBe('Second page');
    await page.waitForTimeout(300);
    expect((await log(page)).change).toEqual([]);
    expect((await snapshot(page)).revision).toBe(before.revision);
    expect(await call(page, 'isDirty')).toBe(false);
    // the next save keeps it
    await drawRectangle(page);
    await expect.poll(() => lastRevision(page)).toBeGreaterThan(before.revision);
    expect((await snapshot(page)).board).toContain('"text":"Renamed page"');
    // a new open() of the board: the map is cleared
    await open(page, 'item-a', EMBED);
    await expect.poll(async () => (await lastLayout(page))!.embeds[0].label).toBe('First page');
    await expectClean(page);
  });

  test('embeds: round trip and lenient model', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED, { name: 'Embed test' });
    expect(withoutViewText((await snapshot(page)).board)).toBe(withoutViewText(EMBED.trim()));
    const lenient = embedVariant((board) => {
      const objects = board.document.pages[0].objects;
      objects[2].kind.embed = 5;
      objects.push({ ...objects[0], id: 'embedellipse', kind: { ...objects[0].kind, geom: 'ellipse', embed: { item: ITEM3 } } });
    });
    await open(page, 'item-b', lenient);
    expect((await log(page)).error).toEqual([]);
    const objects = JSON.parse((await snapshot(page)).board).document.pages[0].objects;
    expect(objects[2].kind).not.toHaveProperty('embed');
    expect(objects[3].kind.embed).toEqual({ item: ITEM3 });
    await expect.poll(async () => (await lastLayout(page))!.itemId).toBe('item-b');
    const l = (await lastLayout(page))!;
    expect(l.embeds.map((e) => e.objectId)).toEqual(['embedcard1', 'embedcard2']);
    expect(l.items).toEqual([ITEM1, ITEM2]);
    await expectClean(page);
  });

  test('embeds: drop of a sidebar document', async ({ page }) => {
    await connect(page);
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    const before = await snapshot(page);
    const at = EMBED_EMPTY.from;
    expectClear(l, at, 'the drop point');
    const item = { v: 1, id: ITEM3, kind: 'doc', title: 'Dropped page' };
    expect(await dispatchDrag(page, { [DRAG_TYPE]: JSON.stringify(item) }, at)).toEqual({ dragenter: true, dragover: true, drop: true });
    await expect.poll(async () => (await log(page)).drop).toEqual([{ itemId: 'item-a', item, point: at }]);
    // ignored: another type, JSON that does not parse, another version, a board, missing fields
    await dispatchDrag(page, { 'text/plain': 'just text' }, at);
    for (const value of ['{"v": 1, "id"', JSON.stringify({ ...item, v: 2 }), JSON.stringify({ ...item, kind: 'board' }), JSON.stringify({ v: 1, id: ITEM3, kind: 'doc' })]) {
      await dispatchDrag(page, { [DRAG_TYPE]: value }, at);
    }
    await page.waitForTimeout(300);
    expect((await log(page)).drop).toHaveLength(1);
    // read-only: not accepted, and no event
    await call(page, 'setReadOnly', true);
    expect(await dispatchDrag(page, { [DRAG_TYPE]: JSON.stringify(item) }, at, ['dragenter', 'dragover'])).toEqual({ dragenter: false, dragover: false });
    await dispatchDrag(page, { [DRAG_TYPE]: JSON.stringify(item) }, at, ['drop']);
    await page.waitForTimeout(300);
    expect((await log(page)).drop).toHaveLength(1);
    // the engine never saw any of it
    expect((await snapshot(page)).board).toBe(before.board);
    await expectClean(page);
  });

  test('embeds: no rotation handle for a single card', async ({ page }) => {
    // a card left and a plain box right, fitted with room above both for a rotation handle
    const board = embedVariant((b) => {
      const [card, , plain] = b.document.pages[0].objects;
      card.kind.frame = { x: -600, y: -180, w: 480, h: 360 };
      plain.kind.frame = { x: 120, y: -60, w: 480, h: 120 };
      b.document.pages[0].objects = [card, plain];
    });
    await connect(page);
    await open(page, 'item-a', board);
    let l = await settledLayout(page);
    const angle = async (id: string) => JSON.parse((await snapshot(page)).board).document.pages[0].objects.find((o: { id: string }) => o.id === id).kind.frame.angle ?? 0;
    // a selected card: where its rotation handle would be, 28 px above the middle of its top edge, nothing rotates
    const card = screen(l, -360, 0);
    await page.mouse.click(card.x, card.y);
    l = await settledLayout(page);
    const top = screen(l, -360, -180);
    const handle = { x: top.x, y: top.y - 28 };
    expectClear(l, handle, "the card's rotation handle");
    await drag(page, handle, { x: handle.x + 200, y: handle.y + 150 });
    await page.waitForTimeout(300);
    expect(await angle('embedcard1')).toBe(0);
    // the control: the plain box rotates from the same place relative to it
    const box = screen(l, 360, 0);
    await page.mouse.click(box.x, box.y);
    l = await settledLayout(page);
    const boxTop = screen(l, 360, -60);
    const boxHandle = { x: boxTop.x, y: boxTop.y - 28 };
    expectClear(l, boxHandle, "the plain box's rotation handle");
    await drag(page, boxHandle, { x: boxHandle.x + 200, y: boxHandle.y + 150 });
    await expect.poll(async () => Math.abs(await angle('embedplain'))).toBeGreaterThan(0.3);
    await expectClean(page);
  });

  test('embeds: read-only', async ({ page }) => {
    /** A right click on card 1 (it selects under any tool, even the read-only Hand), then the menu's "Edit text". */
    const editTextFromTheMenu = async (card: Point) => {
      const known = new Set(((await lastLayout(page))?.occluders ?? []).map((r) => JSON.stringify(r)));
      await page.mouse.click(card.x, card.y, { button: 'right' });
      await expect.poll(async () => (await lastLayout(page))!.embeds[0].selected).toBe(true);
      const l = await settledLayout(page);
      // opened at the pointer, moved up as far as it needs to fit the window
      const menu = l.occluders.find((r) => !known.has(JSON.stringify(r)) && Math.abs(r.x - card.x) < 40 && inside(r, card));
      expect(menu, `the context menu, among ${JSON.stringify(l.occluders)}, near ${JSON.stringify(card)}`).toBeDefined();
      // the sixth row (Cut, Copy, Copy as image, Paste, Duplicate, Edit text): border 1, margin 6, rows of 32 px
      await page.mouse.click(menu!.x + 60, menu!.y + 1 + 6 + 5 * 32 + 16);
      await page.waitForTimeout(400);
    };
    await connect(page);
    // the control, on a writable board: that row is "Edit text", which opens the card
    await open(page, 'item-a', EMBED);
    let l = await settledLayout(page);
    const card = screen(l, -280, 60);
    expectClear(l, card, 'card 1');
    await editTextFromTheMenu(card);
    await expect.poll(async () => (await log(page)).open).toEqual([{ itemId: 'item-a', objectId: 'embedcard1', item: ITEM1, point: null }]);

    await open(page, 'item-b', EMBED, { readOnly: true });
    l = await settledLayout(page);
    expect(screen(l, -280, 60)).toEqual(card);
    await page.mouse.dblclick(card.x, card.y);
    await page.waitForTimeout(400);
    await editTextFromTheMenu(card);
    await page.locator('#board').focus();
    await press(page, 'Enter');
    await page.waitForTimeout(400);
    expect((await log(page)).open).toHaveLength(1);
    expect((await log(page)).change).toEqual([]);
    await expectClean(page);
  });

  test('embeds: frame pixels when the canvas is not at the frame origin', async ({ page }) => {
    // frame.js adds the canvas's offset to every point, rect and origin it reports, and takes it off insertEmbed's point. The
    // canvas fills frame.html at 0, 0, so the other tests cannot tell a missing or reversed conversion: move it, then check each
    // direction against something absolute (the pixels drawn, the mouse, the engine's own hit test).
    const offset = { x: 100, y: 80 };
    await connect(page);
    await page.evaluate(({ x, y }) => {
      document.getElementById('board')!.style.cssText =
        `position: fixed; left: ${x}px; top: ${y}px; width: calc(100% - ${x}px); height: calc(100% - ${y}px)`;
    }, offset);
    await open(page, 'item-a', EMBED);
    const l = await settledLayout(page);
    expect(l.canvas).toEqual({ x: offset.x, y: offset.y, width: 1280 - offset.x, height: 720 - offset.y });
    expect(l.occluders.length).toBeGreaterThan(0);
    for (const r of l.occluders) {
      expect(r.x >= l.canvas.x - 0.01 && r.y >= l.canvas.y - 0.01, `${JSON.stringify(r)} inside the canvas`).toBe(true);
    }
    // the origin: each card's title snapshot is drawn where origin + centre * zoom says, in frame pixels
    for (const card of l.embeds) {
      const c = screen(l, card.x + card.w / 2, card.y + card.h / 2);
      expectClear(l, c, `the centre of ${card.objectId}`);
      expect(await darkPixels(page, { x: c.x - 40, y: c.y - 12, width: 80, height: 24 })).toBeGreaterThan(20);
    }
    // onEmbedOpen's point: where the mouse double-clicked
    const p = screen(l, -280, 60);
    expectClear(l, p, 'card 1');
    await page.mouse.dblclick(p.x, p.y);
    await expect.poll(async () => (await log(page)).open.length).toBe(1);
    const [opened] = (await log(page)).open;
    expect(opened.objectId).toBe('embedcard1');
    expect(Math.hypot(opened.point!.x - p.x, opened.point!.y - p.y)).toBeLessThanOrEqual(2);
    // insertEmbed's point: a small card centred there, on top of card 2, is what a double-click at that frame point opens
    const at = screen(l, 280, -40);
    expectClear(l, at, 'card 2');
    const small = await call<string>(page, 'insertEmbed', { item: ITEM3, title: 'Small', at, w: 40, h: 40 });
    const placed = JSON.parse((await snapshot(page)).board).document.pages[0].objects.find((o: { id: string }) => o.id === small);
    expect(Math.abs(placed.kind.frame.x + 20 - 280)).toBeLessThanOrEqual(1);
    expect(Math.abs(placed.kind.frame.y + 20 + 40)).toBeLessThanOrEqual(1);
    await expect.poll(async () => (await lastLayout(page))!.embeds.some((e) => e.objectId === small)).toBe(true);
    await page.waitForTimeout(600); // far enough from the last double-click
    await page.mouse.dblclick(at.x, at.y);
    await expect.poll(async () => (await log(page)).open.length).toBe(2);
    expect((await log(page)).open[1]).toMatchObject({ itemId: 'item-a', objectId: small, item: ITEM3 });
    await expectClean(page);
  });
});
