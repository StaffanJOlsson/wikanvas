// The Playwright smoke of docs/PLAN.md W7 ("Smoke spec", steps 1-15), plus the real-browser board host check of
// docs/handoff/merge.md (the item menu and the Nested popover render above the board canvas), W7's review step 16 and
// M1.1's step 17 (board document embeds, PLAN "M1.1" "Smoke step 17"). Quire runs from the
// production image in the compose e2e project with an empty /data and is reached directly at http://quire:8080 over
// plain HTTP (docs/ARCHITECTURE.md §13.3, §14), so the session cookie is `quire_session`. Run it only through
// `scripts/e2e.sh smoke`, after `scripts/image.sh`.
//
// The steps build on each other (one workspace, one owner), so they are one test with a test.step per PLAN step.
// Decisions taken within the PLAN's wording (recorded in docs/handoff/W7.md):
// - Step 1: "Home shows Getting started" is the seeded collection in the sidebar of /home.
// - Step 4: "the body" is the paragraph under the heading "Plan" (step 15 expects that heading to still be there), so
//   step 4 deletes the paragraph's text, not the heading.
// - Step 11: the reloaded engine holding the object is shown by the next save: a second rectangle drawn after the
//   reload is saved as a board of two objects.
// - Step 9's fallback branch (WebGL unavailable) is taken only when this browser really has no WebGL 2; the reason is
//   printed and recorded as an annotation, and the engine steps 10-12 are skipped.
// - Step 13: the export is fetched by the page itself, as the browser sends it (cookie and `Sec-Fetch-Site`); the same
//   GET without fetch metadata is refused (`csrf_rejected`), which the step checks too.
// - Every page of the run must log no CSP violation (the production headers of §11, which happy-dom never applies).
// Step 17's choices (recorded in docs/handoff/E-merge.md):
// - It runs in a 1600 × 1000 window, so that three 480 × 360 cards at zoom 1 leave empty canvas around them (the default
//   1280 × 720 leaves almost none beside the two cards that "Text doc" and "Embed page…" both centre on the view).
// - The sidebar drag is real (mouse down on A's row, moves, up over the canvas); Playwright carries it into the frame,
//   so the synthetic drop PLAN allows as a fallback is not used.
// - Before the double-click, a single click on C's card checks that it selects ("Double-click to edit") without editing.
// - "Ctrl+Z ten times … leave the text as it was": C's stored text equals the text saved before the ten Enters.
import { expect, test, type Locator, type Page, type Response } from '@playwright/test';
import { readFile } from 'node:fs/promises';

const SETUP_CODE = 'e2e-setup-code';
const OWNER = { workspace: 'Smoke workspace', name: 'Smoke Owner', email: 'owner@example.com', password: 'smoke-password-123' };
const BOARD_TITLE = 'Sketch';
/** Step 17's board (PLAN M1.1 "Smoke step 17"). */
const EMBED_BOARD = 'Embed board';
/** The engine download and start under SwiftShader. */
const ENGINE_TIMEOUT = 120_000;
/** A save: the doc session's 1 s debounce, the board loop's 1.5 s quiet time, and the request. */
const SAVE_TIMEOUT = 20_000;

type Json = any; // eslint-disable-line @typescript-eslint/no-explicit-any

/** The 10-character url_id at the end of a /doc/… or /board/… path. */
function urlIdOf(page: Page): string {
  const m = /\/(?:doc|board)\/(?:[^/]*-)?([0-9A-Za-z]{10})\/?$/.exec(new URL(page.url()).pathname);
  if (!m) throw new Error(`not an item page: ${page.url()}`);
  return m[1];
}

/** `GET /api/v1/items/{ref}` with the page's session. */
async function detail(page: Page, ref: string): Promise<Json> {
  const r = await page.request.get(`/api/v1/items/${ref}`);
  expect(r.status(), `GET /api/v1/items/${ref}`).toBe(200);
  return r.json();
}

function isItemPatch(r: Response): boolean {
  return r.request().method() === 'PATCH' && /\/api\/v1\/items\/[^/]+$/.test(new URL(r.url()).pathname);
}

function isDocUpdate(r: Response): boolean {
  return r.request().method() === 'POST' && /\/api\/v1\/items\/[^/]+\/doc\/updates$/.test(new URL(r.url()).pathname);
}

function isBoardPut(r: Response): boolean {
  return r.request().method() === 'PUT' && /\/api\/v1\/items\/[^/]+\/board$/.test(new URL(r.url()).pathname);
}

function sidebar(page: Page): Locator {
  return page.getByRole('complementary', { name: 'Sidebar' });
}

function treeItem(page: Page, name: string): Locator {
  return sidebar(page).getByRole('treeitem', { name, exact: true });
}

function saveStatus(page: Page): Locator {
  return page.locator('.save-status');
}

function editor(page: Page): Locator {
  return page.locator('.quire-editor');
}

/**
 * A pause after a selection change, as a person makes between selecting and typing: ProseMirror takes a DOM
 * selection over on `selectionchange`, which a key pressed in the same instant can overtake.
 */
async function settle(page: Page): Promise<void> {
  await page.waitForTimeout(150);
}

/** Types a title into the document header (or the board's title crumb) and waits for its PATCH. */
async function setTitle(page: Page, input: Locator, title: string): Promise<void> {
  await expect(input).toBeVisible();
  await expect(input).toHaveValue('');
  const saved = page.waitForResponse((r) => isItemPatch(r) && r.ok());
  await input.fill(title);
  await saved;
  await expect(input).toHaveValue(title);
}

/** Opens a menu button's menu and picks an item. */
async function pick(page: Page, button: Locator, item: string): Promise<void> {
  await button.click();
  await page.getByRole('menu').getByRole('menuitem', { name: item, exact: true }).click();
}

/** The url_id of the item page the browser is on, or null on another page. */
function currentRef(page: Page): string | null {
  const m = /\/(?:doc|board)\/(?:[^/]*-)?([0-9A-Za-z]{10})\/?$/.exec(new URL(page.url()).pathname);
  return m ? m[1] : null;
}

/**
 * Waits for a navigation to the page of another item of `kind` (by url_id: renaming an item rewrites its own path to
 * the new slug, which is not a new item).
 */
async function waitForNewItem(page: Page, kind: 'doc' | 'board', before: string | null): Promise<void> {
  await page.waitForURL((u) => {
    const m = /^\/(doc|board)\/(?:[^/]*-)?([0-9A-Za-z]{10})\/?$/.exec(u.pathname);
    return m !== null && m[1] === kind && m[2] !== before;
  });
}

/** A nested item of the current item, through the top bar's "New…" menu. */
async function newNested(page: Page, kind: 'doc' | 'board'): Promise<void> {
  const before = currentRef(page);
  await pick(page, page.getByRole('button', { name: 'New…', exact: true }), kind === 'doc' ? 'Nested document' : 'Nested board');
  await waitForNewItem(page, kind, before);
}

/** The sidebar tree as "level:name" rows, top to bottom. */
async function treeRows(page: Page): Promise<string[]> {
  return sidebar(page)
    .getByRole('treeitem')
    .evaluateAll((rows) => rows.map((r) => `${r.getAttribute('aria-level')}:${r.getAttribute('aria-label')}`));
}

/** The names of the entries of a tar archive (ustar, with GNU/PAX extension headers passed over). */
function tarEntries(buf: Buffer): Map<string, Buffer> {
  const out = new Map<string, Buffer>();
  const field = (h: Buffer, from: number, len: number) => h.subarray(from, from + len).toString('utf8').replace(/\0[\s\S]*$/, '');
  let off = 0;
  while (off + 512 <= buf.length) {
    const h = buf.subarray(off, off + 512);
    if (h.every((b) => b === 0)) break;
    const size = parseInt(field(h, 124, 12).trim() || '0', 8);
    const type = String.fromCharCode(h[156] || 48);
    const prefix = field(h, 345, 155);
    const name = prefix ? `${prefix}/${field(h, 0, 100)}` : field(h, 0, 100);
    if (type === '0' || type === '\0') out.set(name.replace(/^\.\//, ''), buf.subarray(off + 512, off + 512 + size));
    off += 512 + Math.ceil(size / 512) * 512;
  }
  return out;
}

/** The board iframe, visible over the surface: the frame connected and the board opened (`onReady`). */
function boardFrame(page: Page): Locator {
  return page.locator('.board-frame-layer iframe.board-frame');
}

async function waitForEngine(page: Page): Promise<void> {
  await expect(boardFrame(page)).toBeVisible({ timeout: ENGINE_TIMEOUT });
  await expect(page.locator('.board-loading')).toHaveCount(0);
  await expect(page.locator('.board-preview')).toHaveCount(0);
  await expect(page.locator('.board-surface .board-error')).toHaveCount(0);
}

/** Drags with the left button in small steps, so that the engine sees the gesture over several frames. */
async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }): Promise<void> {
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

/**
 * Clicks the canvas, presses R (the rectangle tool) and drags a rectangle over the fractions `area` of the canvas, away
 * from Inkboard's floating cards (menu and tools at the top, style panel on the left, zoom and pages at the bottom,
 * help bottom right). Resolves with the board PUT that saves it.
 */
async function drawRectangle(page: Page, area: [number, number, number, number], expectStatus = 200): Promise<Response> {
  const box = await boardFrame(page).boundingBox();
  if (!box) throw new Error('the board frame has no box');
  const at = (fx: number, fy: number) => ({ x: box.x + box.width * fx, y: box.y + box.height * fy });
  const put = page.waitForResponse((r) => isBoardPut(r) && r.status() === expectStatus, { timeout: SAVE_TIMEOUT });
  const from = at(area[0], area[1]);
  await page.mouse.click(from.x, from.y);
  await page.waitForTimeout(150);
  await page.keyboard.press('r');
  await page.waitForTimeout(150);
  await drag(page, from, at(area[2], area[3]));
  return put;
}

/** A board card of the embeds overlay (docs/ARCHITECTURE.md §18.5 test contract), by its target. */
function cardOf(page: Page, item: string): Locator {
  return page.locator(`.board-embeds .board-embed-card[data-object-id][data-item-id="${item}"][data-state]`);
}

function isDocUpdateOf(r: Response, id: string): boolean {
  return isDocUpdate(r) && new URL(r.url()).pathname === `/api/v1/items/${id}/doc/updates`;
}

/** The centre of `el`'s box is within 40 px of `at`. */
async function expectCentredOn(el: Locator, at: { x: number; y: number }, what: string): Promise<void> {
  const b = await el.boundingBox();
  if (!b) throw new Error(`${what} has no box`);
  expect(Math.abs(b.x + b.width / 2 - at.x), `${what}: x of its centre`).toBeLessThanOrEqual(40);
  expect(Math.abs(b.y + b.height / 2 - at.y), `${what}: y of its centre`).toBeLessThanOrEqual(40);
}

/** The cards of a stored board, in drawing order: their target and the plain text of their label. */
async function embedCards(page: Page, board: string): Promise<{ item: string; label: string }[]> {
  const r = await page.request.get(`/api/v1/items/${board}/board`);
  expect(r.status()).toBe(200);
  const stored = await r.json();
  return (stored.document.pages as Json[])
    .flatMap((p) => p.objects as Json[])
    .filter((o) => o.kind?.type === 'box' && typeof o.kind.embed?.item === 'string')
    .map((o) => ({ item: o.kind.embed.item, label: (o.kind.label?.text ?? []).map((s: Json) => s.text).join('') }));
}

/** The viewport centre of the first occurrence of `word` in `root`'s text. */
async function wordBox(root: Locator, word: string): Promise<{ x: number; y: number }> {
  const at = await root.evaluate((el, w) => {
    const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      const i = n.textContent?.indexOf(w) ?? -1;
      if (i < 0) continue;
      const range = document.createRange();
      range.setStart(n, i);
      range.setEnd(n, i + w.length);
      const r = range.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    }
    return null;
  }, word);
  if (!at) throw new Error(`"${word}" is not in the card`);
  return at;
}

/** The overlay paints above the board iframe where both overlap: the topmost element there is inside the overlay. */
async function expectAboveCanvas(page: Page, overlay: Locator, what: string): Promise<void> {
  await expect(overlay).toBeVisible();
  const f = await boardFrame(page).boundingBox();
  const o = await overlay.boundingBox();
  if (!f || !o) throw new Error(`${what}: no box`);
  const x0 = Math.max(f.x, o.x);
  const x1 = Math.min(f.x + f.width, o.x + o.width);
  const y0 = Math.max(f.y, o.y);
  const y1 = Math.min(f.y + f.height, o.y + o.height);
  expect(x1 - x0, `${what} overlaps the canvas horizontally`).toBeGreaterThan(8);
  expect(y1 - y0, `${what} overlaps the canvas vertically`).toBeGreaterThan(8);
  const point = { x: (x0 + x1) / 2, y: (y0 + y1) / 2 };
  const inside = await overlay.evaluate((el, p) => {
    const hit = document.elementFromPoint(p.x, p.y);
    return hit !== null && el.contains(hit);
  }, point);
  expect(inside, `${what} is the topmost element at (${point.x}, ${point.y}), over the board iframe`).toBe(true);
}

test('smoke: setup to export (PLAN W7 steps 1-15, the review fixes, M1.1 board embeds)', async ({ page, context, request }, testInfo) => {
  test.setTimeout(15 * 60_000);
  const pageErrors: string[] = [];
  page.on('pageerror', (e) => pageErrors.push(e.message));
  // Every page of the context, its board iframe included: a CSP violation means something the production headers
  // block (§11), which happy-dom never sees.
  const cspViolations: string[] = [];
  context.on('console', (m) => {
    if (/Content Security Policy/i.test(m.text())) cspViolations.push(`${m.location().url}: ${m.text()}`);
  });

  let aUrl = '';
  let cUrl = '';
  let cRef = '';
  let cId = '';

  await test.step('1. / redirects to /setup; setup with the code; Home shows "Getting started"', async () => {
    await page.goto('/');
    await page.waitForURL('**/setup');
    await expect(page.getByRole('heading', { name: 'Set up Quire' })).toBeVisible();
    await page.getByLabel('Setup code').fill(SETUP_CODE);
    await page.getByLabel('Workspace name').fill(OWNER.workspace);
    await page.getByLabel('Your name').fill(OWNER.name);
    await page.getByLabel('Email').fill(OWNER.email);
    await page.getByLabel('Password', { exact: true }).fill(OWNER.password);
    await page.getByLabel('Repeat the password').fill(OWNER.password);
    const setup = page.waitForResponse((r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/v1/setup');
    await page.getByRole('button', { name: 'Create workspace' }).click();
    expect((await setup).status()).toBe(201);
    await page.waitForURL('**/home');
    await expect(page.getByRole('heading', { name: 'Home', level: 1 })).toBeVisible();
    await expect(treeItem(page, 'Getting started')).toBeVisible();
    await expect(sidebar(page).getByText(OWNER.workspace)).toBeVisible();
  });

  await test.step('2. Sign out, sign in with "Keep me signed in": the cookie has a Max-Age', async () => {
    await pick(page, page.getByRole('button', { name: 'Workspace menu' }), 'Sign out');
    await page.waitForURL('**/login**');
    expect((await context.cookies()).filter((c) => c.name === 'quire_session' && c.value !== '')).toEqual([]);
    await page.getByLabel('Email').fill(OWNER.email);
    await page.getByLabel('Password').fill(OWNER.password);
    const keep = page.getByLabel('Keep me signed in');
    await keep.check();
    await expect(keep).toBeChecked();
    const login = page.waitForResponse((r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/v1/auth/login');
    await page.getByRole('button', { name: 'Sign in' }).click();
    const res = await login;
    expect(res.status()).toBe(200);
    const setCookie = (await res.headerValue('set-cookie')) ?? '';
    expect(setCookie).toMatch(/(^|\n)quire_session=[^;\n]+;[^\n]*Max-Age=\d+/i);
    expect(setCookie).toMatch(/HttpOnly/i);
    await page.waitForURL('**/home');
    const cookie = (await context.cookies()).find((c) => c.name === 'quire_session');
    expect(cookie, 'the session cookie').toBeDefined();
    // A Max-Age cookie has an expiry (a session cookie has -1); "Keep me signed in" lasts days, not minutes.
    expect(cookie!.expires).toBeGreaterThan(Date.now() / 1000 + 24 * 3600);
  });

  await test.step('3. Eng / A / B / C; heading "Plan" and "quokka" in C; Saved; after a reload the tree, highlight and breadcrumbs', async () => {
    await sidebar(page).getByRole('button', { name: 'New collection…' }).click();
    const dialog = page.getByRole('dialog', { name: 'New collection' });
    await dialog.getByLabel('Name').fill('Eng');
    await dialog.getByRole('button', { name: 'Create', exact: true }).click();
    await page.waitForURL('**/collection/eng-*');
    const eng = treeItem(page, 'Eng');
    await expect(eng).toBeVisible();

    await eng.hover();
    await pick(page, eng.getByRole('button', { name: 'New in Eng' }), 'Document');
    await waitForNewItem(page, 'doc', null);
    await setTitle(page, page.locator('#item-title'), 'A');
    aUrl = page.url();
    await newNested(page, 'doc');
    await setTitle(page, page.locator('#item-title'), 'B');
    await newNested(page, 'doc');
    await setTitle(page, page.locator('#item-title'), 'C');
    cRef = urlIdOf(page);

    await expect(editor(page)).toBeVisible();
    await editor(page).click();
    const saved = page.waitForResponse((r) => isDocUpdate(r) && r.ok(), { timeout: SAVE_TIMEOUT });
    await page.keyboard.type('# Plan');
    await page.keyboard.press('Enter');
    await page.keyboard.type('quokka');
    await saved;
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    await expect(editor(page).locator('h1')).toHaveText('Plan');

    await page.reload();
    await expect(editor(page).locator('h1')).toHaveText('Plan');
    await expect(editor(page).locator('p', { hasText: 'quokka' })).toHaveCount(1);
    await expect(page.locator('#item-title')).toHaveValue('C');
    const c = treeItem(page, 'C');
    await expect(c).toHaveAttribute('aria-selected', 'true');
    await expect(c).toHaveAttribute('aria-current', 'page');
    await expect(c).toHaveAttribute('aria-level', '4');
    for (const name of ['Eng', 'A', 'B']) await expect(treeItem(page, name)).toHaveAttribute('aria-expanded', 'true');
    const crumbs = page.getByRole('navigation', { name: 'Breadcrumbs' });
    const links = crumbs.getByRole('link');
    await expect(links).toHaveCount(3);
    await expect(links.nth(0)).toHaveAccessibleName('Eng');
    await expect(links.nth(1)).toHaveAccessibleName('A');
    await expect(links.nth(2)).toHaveAccessibleName('B');
    await expect(crumbs.locator('[aria-current="page"]')).toHaveText('/ C');
    expect(urlIdOf(page)).toBe(cRef);
    cUrl = page.url();
    cId = (await detail(page, cRef)).item.id;
  });

  await test.step('4. Delete the body text of C: Saved, empty after a reload; type "quokka" again', async () => {
    // Selects the paragraph's text (a DOM range over it, as a drag across it would).
    await editor(page).locator('p', { hasText: 'quokka' }).selectText();
    await settle(page);
    const deleted = page.waitForResponse((r) => isDocUpdate(r) && r.ok(), { timeout: SAVE_TIMEOUT });
    await page.keyboard.press('Backspace');
    await deleted;
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    await expect(editor(page)).not.toContainText('quokka');

    await page.reload();
    await expect(editor(page).locator('h1')).toHaveText('Plan');
    await expect(editor(page)).not.toContainText('quokka');
    for (const p of await editor(page).locator('p').all()) await expect(p).toHaveText('');
    const text = await page.request.get(`/api/v1/items/${cId}/text`);
    expect(text.status()).toBe(200);
    const stored = await text.json();
    expect(stored.title).toBe('C');
    expect(stored.body).toContain('Plan');
    expect(stored.body).not.toContain('quokka');

    // The caret at the end of the heading (its text selected, then collapsed to the right), then a new paragraph.
    await editor(page).locator('h1').selectText();
    await settle(page);
    await page.keyboard.press('ArrowRight');
    await settle(page);
    await page.keyboard.press('Enter');
    const retyped = page.waitForResponse((r) => isDocUpdate(r) && r.ok(), { timeout: SAVE_TIMEOUT });
    await page.keyboard.type('quokka');
    await retyped;
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    await expect(editor(page).locator('p', { hasText: 'quokka' })).toHaveCount(1);
    await expect(editor(page).locator('h1')).toHaveText('Plan');
    await expect(editor(page).locator('h1 + p')).toHaveText('quokka');
  });

  await test.step('5. Search "quok": C first with a marked hit; "quok -zebra" still finds C; Mod+K, "C", Enter opens C', async () => {
    await sidebar(page).getByRole('link', { name: 'Search', exact: true }).click();
    await page.waitForURL('**/search**');
    const input = page.getByRole('searchbox', { name: 'Search' });
    const results = page.getByRole('list', { name: 'Results' });
    const first = results.locator('.search-result').first();

    let searched = page.waitForResponse((r) => new URL(r.url()).pathname === '/api/v1/search' && new URL(r.url()).searchParams.get('q') === 'quok');
    await input.fill('quok');
    expect((await searched).status()).toBe(200);
    await expect(first.locator('.search-result-title')).toHaveText('C');
    await expect(first.locator('.snippet mark').first()).toHaveText(/^quok/i);

    searched = page.waitForResponse((r) => new URL(r.url()).pathname === '/api/v1/search' && new URL(r.url()).searchParams.get('q') === 'quok -zebra');
    await input.fill('quok -zebra');
    expect((await searched).status()).toBe(200);
    await page.waitForURL((u) => u.searchParams.get('q') === 'quok -zebra');
    await expect(first.locator('.search-result-title')).toHaveText('C');

    await page.getByRole('heading', { name: 'Search', level: 1 }).click();
    await page.keyboard.press('ControlOrMeta+k');
    const switcher = page.getByRole('dialog', { name: 'Quick switcher' });
    await expect(switcher).toBeVisible();
    await switcher.getByRole('combobox', { name: 'Search titles' }).fill('C');
    const option = switcher.getByRole('option', { selected: true });
    await expect(option.locator('.switcher-title')).toHaveText('C');
    await page.keyboard.press('Enter');
    await page.waitForURL((u) => u.pathname.startsWith('/doc/') && u.pathname.endsWith(cRef));
    await expect(page.locator('#item-title')).toHaveValue('C');
  });

  await test.step('6. Star C: it is in STARRED; A, B and C are published; no Publish button, no Drafts entry', async () => {
    const star = page.locator('.item-header').getByRole('button', { name: 'Star', exact: true });
    await star.click();
    await expect(page.locator('.item-header').getByRole('button', { name: 'Unstar' })).toHaveAttribute('aria-pressed', 'true');
    const starred = sidebar(page).locator('section[aria-labelledby="sidebar-starred"]');
    await expect(starred.getByRole('link', { name: 'C', exact: true })).toBeVisible();
    const c = await detail(page, cId);
    expect(c.starred).toBe(true);
    expect(c.effectively_published).toBe(true);
    for (const crumb of c.breadcrumbs) expect((await detail(page, crumb.id)).effectively_published).toBe(true);
    expect(c.breadcrumbs.map((b: Json) => b.title)).toEqual(['A', 'B']);
    await expect(page.getByRole('button', { name: /publish/i })).toHaveCount(0);
    await expect(sidebar(page).getByText(/drafts/i)).toHaveCount(0);
    const side = await (await page.request.get('/api/v1/sidebar')).json();
    expect(side.drafts_count).toBe(0);
  });

  await test.step('7. Move B with its subtree to "Getting started" through the Move dialog: C\'s breadcrumbs update', async () => {
    const b = treeItem(page, 'B');
    await b.hover();
    await pick(page, b.getByRole('button', { name: 'B menu' }), 'Move…');
    const dialog = page.getByRole('dialog', { name: 'Move “B”' });
    await expect(dialog).toBeVisible();
    // B and C are not offered as new places.
    await expect(dialog.getByRole('option', { name: 'C', exact: true })).toHaveCount(0);
    await dialog.getByRole('option', { name: 'Getting started (top level)' }).click();
    const moved = page.waitForResponse((r) => r.request().method() === 'POST' && /\/move$/.test(new URL(r.url()).pathname));
    await dialog.getByRole('button', { name: 'Move', exact: true }).click();
    expect((await moved).status()).toBe(200);
    const links = page.getByRole('navigation', { name: 'Breadcrumbs' }).getByRole('link');
    await expect(links).toHaveCount(2);
    await expect(links.nth(0)).toHaveAccessibleName('Getting started');
    await expect(links.nth(1)).toHaveAccessibleName('B');
    const c = await detail(page, cId);
    expect(c.collection.name).toBe('Getting started');
    expect(c.breadcrumbs.map((x: Json) => x.title)).toEqual(['B']);
  });

  await test.step('8. Trash B: B and C leave the tree, Trash shows "+1 nested"; Restore puts both back in place', async () => {
    // A fresh load reveals C (its collection and B expanded), so the tree shows where B and C are.
    await page.goto(cUrl);
    await expect(treeItem(page, 'C')).toHaveAttribute('aria-selected', 'true');
    const before = await treeRows(page);
    expect(before).toContain('2:B');
    expect(before).toContain('3:C');

    const b = treeItem(page, 'B');
    await b.hover();
    await pick(page, b.getByRole('button', { name: 'B menu' }), 'Delete');
    const confirm = page.getByRole('alertdialog', { name: 'Move “B” to Trash?' });
    await confirm.getByRole('button', { name: 'Move to Trash' }).click();
    await expect(treeItem(page, 'B')).toHaveCount(0);
    await expect(treeItem(page, 'C')).toHaveCount(0);

    await sidebar(page).getByRole('link', { name: 'Trash', exact: true }).click();
    await page.waitForURL('**/trash');
    const row = page.getByRole('list', { name: 'Trashed items' }).locator('.item-list-row', { hasText: 'B' });
    await expect(row).toHaveCount(1);
    await expect(row.locator('.item-list-title')).toHaveText('B +1 nested');
    await row.getByRole('button', { name: 'Restore' }).click();
    await expect(page.getByText('Trash is empty.')).toBeVisible();
    await expect(treeItem(page, 'B')).toBeVisible();
    await expect.poll(() => treeRows(page)).toEqual(before);
    const c = await detail(page, cId);
    expect(c.trashed_at).toBeNull();
    expect(c.breadcrumbs.map((x: Json) => x.title)).toEqual(['B']);
  });

  let boardUrl = '';
  let boardId = '';
  let engine = true;

  await test.step('9. A board under A: the engine reports ready (fallback only without WebGL 2); menus over the canvas', async () => {
    const inkboard: { url: string; status: number }[] = [];
    page.on('response', (r) => {
      if (new URL(r.url()).pathname.startsWith('/inkboard/')) inkboard.push({ url: new URL(r.url()).pathname, status: r.status() });
    });
    await page.goto(aUrl);
    await expect(page.locator('#item-title')).toHaveValue('A');
    await newNested(page, 'board');
    await setTitle(page, page.getByRole('navigation', { name: 'Breadcrumbs' }).getByRole('textbox', { name: 'Title' }), BOARD_TITLE);
    boardUrl = page.url();
    boardId = (await detail(page, urlIdOf(page))).item.id;
    const board = await detail(page, boardId);
    expect(board.breadcrumbs.map((x: Json) => x.title)).toEqual(['A']);
    expect(board.board.objects).toBe(0);

    const gl = await page.evaluate(() => {
      const ctx = document.createElement('canvas').getContext('webgl2');
      if (!ctx) return null;
      const info = ctx.getExtension('WEBGL_debug_renderer_info');
      return String(info ? ctx.getParameter(info.UNMASKED_RENDERER_WEBGL) : ctx.getParameter(ctx.RENDERER));
    });
    const fallback = page.locator('.board-preview-reason');
    await expect(boardFrame(page).or(fallback)).toBeVisible({ timeout: ENGINE_TIMEOUT });
    if (await fallback.isVisible()) {
      const reason = await fallback.innerText();
      expect(gl, `the board fell back to the preview although WebGL 2 is available (${gl}): ${reason}`).toBeNull();
      expect(reason).toContain('This browser cannot run the whiteboard editor');
      const why = `WebGL 2 is unavailable in this browser: ${reason}`;
      testInfo.annotations.push({ type: 'skipped: steps 10-12 (board engine)', description: why });
      console.log(`smoke: steps 10-12 skipped: ${why}`);
      engine = false;
      return;
    }
    testInfo.annotations.push({ type: 'webgl2', description: gl ?? 'none' });
    await waitForEngine(page);
    console.log(`smoke: the board engine is ready (WebGL 2: ${gl}); steps 10-12 run on it`);
    // The frame page, its manifest and the engine came from /inkboard/{build}/ (merge.md item 1).
    const manifest = inkboard.find((r) => r.url.endsWith('/manifest.json'));
    expect(manifest, 'the frame fetched manifest.json').toBeDefined();
    expect(manifest!.status).toBe(200);
    expect(inkboard.filter((r) => r.status >= 400)).toEqual([]);
    expect(inkboard.some((r) => r.url.endsWith('/frame.html'))).toBe(true);
    expect(inkboard.some((r) => r.url.endsWith('.wasm'))).toBe(true);

    // The real-browser board host check (merge.md, W6 W3-3): menus and popovers paint above the board canvas.
    await page.getByRole('button', { name: 'Item menu' }).click();
    await expectAboveCanvas(page, page.getByRole('menu', { name: 'Item menu' }), 'the item menu');
    await page.keyboard.press('Escape');
    await expect(page.getByRole('menu', { name: 'Item menu' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Nested pages' }).click();
    await expectAboveCanvas(page, page.getByRole('dialog', { name: 'Nested pages' }), 'the Nested popover');
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog', { name: 'Nested pages' })).toHaveCount(0);
  });

  await test.step('10. Click the canvas, R and drag: Saved, and board.objects == 1', async (step) => {
    step.skip(!engine, 'WebGL 2 is unavailable (step 9)');
    const put = await drawRectangle(page, [0.55, 0.35, 0.7, 0.5]);
    expect(put.status()).toBe(200);
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    expect((await detail(page, boardId)).board.objects).toBe(1);
  });

  await test.step('11. Reload: the object is still there; the page SVG; the embed card in A loads no /inkboard/', async (step) => {
    step.skip(!engine, 'WebGL 2 is unavailable (step 9)');
    await page.reload();
    await waitForEngine(page);
    const reloaded = await detail(page, boardId);
    expect(reloaded.board.objects).toBe(1);
    const stored = await (await page.request.get(`/api/v1/items/${boardId}/board`)).json();
    expect(stored.document.pages.reduce((n: number, p: Json) => n + p.objects.length, 0)).toBe(1);
    // The engine opened the stored board: its next save holds that object and a new one.
    const put = await drawRectangle(page, [0.55, 0.6, 0.7, 0.75]);
    expect((await put.json()).meta.objects).toBe(2);
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    expect((await detail(page, boardId)).board.objects).toBe(2);

    // The SVG preview of page 0 at the current revision (rendered after the save).
    let rev = 0;
    await expect.poll(async () => {
      const d = await detail(page, boardId);
      rev = d.board.rev;
      return d.board.preview_rev === d.board.rev;
    }, { timeout: 60_000 }).toBe(true);
    const svg = await page.request.get(`/api/v1/items/${boardId}/board/pages/0?rev=${rev}`, { maxRedirects: 0 });
    expect(svg.status()).toBe(200);
    expect(svg.headers()['content-type']).toMatch(/^image\/svg\+xml/);
    expect(await svg.text()).toContain('<svg');

    // The embed card in A, inserted through slash → "Embed board…", shows the preview <img>; nothing under /inkboard/
    // is requested from the document page.
    const requested: string[] = [];
    page.on('request', (r) => requested.push(new URL(r.url()).pathname));
    await page.goto(aUrl);
    await expect(page.locator('#item-title')).toHaveValue('A');
    await editor(page).click();
    await page.keyboard.type('/embed');
    const slash = page.getByRole('listbox').locator('.suggest-item', { hasText: 'Embed board…' });
    await expect(slash).toBeVisible();
    await page.keyboard.press('Enter');
    const picker = page.getByRole('dialog', { name: 'Embed a board' });
    await picker.getByPlaceholder('Find a board').fill(BOARD_TITLE);
    const saved = page.waitForResponse((r) => isDocUpdate(r) && r.ok(), { timeout: SAVE_TIMEOUT });
    await picker.getByRole('button', { name: BOARD_TITLE, exact: true }).click();
    const card = editor(page).locator('.board-embed');
    await expect(card.locator('img.board-embed-preview')).toBeVisible();
    await expect(card.locator('.board-embed-title')).toHaveText(BOARD_TITLE);
    expect(await card.locator('img.board-embed-preview').getAttribute('src')).toContain(`/board/pages/0?rev=`);
    await expect.poll(() => card.locator('img.board-embed-preview').evaluate((i: HTMLImageElement) => i.complete && i.naturalWidth > 0)).toBe(true);
    await saved;
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    await page.waitForTimeout(500);
    expect(requested.filter((p) => p.startsWith('/inkboard/'))).toEqual([]);
    await expect(page.locator('iframe.board-frame')).toHaveCount(0);
  });

  await test.step('12. The board in two pages, changed in both: the second save shows the conflict banner', async (step) => {
    step.skip(!engine, 'WebGL 2 is unavailable (step 9)');
    const second = await context.newPage();
    try {
      await second.goto(boardUrl);
      await waitForEngine(second);
      await page.bringToFront();
      await page.goto(boardUrl);
      await waitForEngine(page);
      const first = await drawRectangle(page, [0.72, 0.35, 0.85, 0.5]);
      expect((await first.json()).meta.objects).toBe(3);
      await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
      await second.bringToFront();
      const late = await drawRectangle(second, [0.72, 0.6, 0.85, 0.75], 412);
      expect(late.status()).toBe(412);
      const banner = second.getByRole('alert').filter({ hasText: 'This board was changed somewhere else' });
      await expect(banner).toBeVisible();
      await expect(banner.getByRole('button', { name: 'Load latest' })).toBeVisible();
      await expect(banner.getByRole('button', { name: 'Keep mine as a copy' })).toBeVisible();
      await page.bringToFront();
      await expect(page.getByText('This board was changed somewhere else')).toHaveCount(0);
      expect((await detail(page, boardId)).board.objects).toBe(3);
    } finally {
      await second.close();
    }
  });

  await test.step('13. GET /api/v1/export is a tar with manifest.json', async () => {
    // The browser's own GET (cookie and fetch metadata, `Sec-Fetch-Site: same-origin`): a cookie request without
    // fetch metadata or a same-origin Referer is refused (§8, the export's CSRF rule).
    const bare = await page.request.get('/api/v1/export');
    expect(bare.status()).toBe(403);
    expect((await bare.json()).error.code).toBe('csrf_rejected');
    const res = await page.evaluate(async () => {
      const r = await fetch('/api/v1/export', { credentials: 'same-origin' });
      const bytes = new Uint8Array(await r.arrayBuffer());
      let bin = '';
      for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      return { status: r.status, type: r.headers.get('content-type') ?? '', body: btoa(bin) };
    });
    expect(res.status).toBe(200);
    expect(res.type).toMatch(/^application\/(x-)?tar/);
    const entries = tarEntries(Buffer.from(res.body, 'base64'));
    expect([...entries.keys()]).toContain('manifest.json');
    expect([...entries.keys()]).toContain('quire.db');
    const manifest = JSON.parse(entries.get('manifest.json')!.toString('utf8'));
    expect(typeof manifest).toBe('object');
  });

  await test.step('14. Settings → API tokens: a read token, shown once; Bearer works, POST 403, revoked 401', async () => {
    await sidebar(page).getByRole('link', { name: 'Settings', exact: true }).click();
    await page.waitForURL('**/settings/account');
    await page.getByRole('navigation', { name: 'Settings' }).getByRole('link', { name: 'API tokens' }).click();
    await page.waitForURL('**/settings/tokens');
    await page.getByRole('button', { name: 'New token' }).click();
    const create = page.getByRole('dialog', { name: 'New API token' });
    await create.getByLabel('Name').fill('smoke-read');
    await expect(create.getByRole('radio', { name: /^Read/ })).toBeChecked();
    await create.getByRole('button', { name: 'Create token' }).click();
    const shown = page.getByRole('dialog', { name: 'Your new token' });
    const token = await shown.getByRole('textbox', { name: 'Token' }).inputValue();
    expect(token).toMatch(/^qpat_[0-9A-Za-z_-]{46}$/);
    await shown.getByRole('button', { name: 'Done' }).click();
    await expect(shown).toHaveCount(0);
    // Shown once: the page keeps only the prefix.
    const row = page.locator('.token-table tbody tr', { hasText: 'smoke-read' });
    await expect(row).toHaveCount(1);
    expect(token.startsWith((await row.locator('code').innerText()).replace(/…$/, ''))).toBe(true);
    expect(await page.content()).not.toContain(token);

    const bearer = { Authorization: `Bearer ${token}` };
    const me = await request.get('/api/v1/auth/me', { headers: bearer });
    expect(me.status()).toBe(200);
    const meBody = await me.json();
    expect(meBody.auth.method).toBe('token');
    expect(meBody.auth.token.scopes).toEqual(['read']);
    const post = await request.post('/api/v1/collections', { headers: bearer, data: { name: 'Not allowed' } });
    expect(post.status()).toBe(403);
    expect((await post.json()).error.code).toBe('insufficient_scope');

    await row.getByRole('button', { name: 'Revoke smoke-read' }).click();
    await page.getByRole('alertdialog', { name: 'Revoke “smoke-read”?' }).getByRole('button', { name: 'Revoke', exact: true }).click();
    await expect(page.locator('.token-table tbody tr', { hasText: 'smoke-read' })).toHaveCount(0);
    const after = await request.get('/api/v1/auth/me', { headers: bearer });
    expect(after.status()).toBe(401);
  });

  await test.step('15. Document ⋯ → Export Markdown downloads a .md whose first heading is "Plan"', async () => {
    await page.goto(cUrl);
    await expect(page.locator('#item-title')).toHaveValue('C');
    await page.getByRole('button', { name: 'Item menu' }).click();
    const download = page.waitForEvent('download');
    await page.getByRole('menu', { name: 'Item menu' }).getByRole('menuitem', { name: 'Export Markdown' }).click();
    const file = await download;
    expect(file.suggestedFilename()).toBe('c.md');
    const path = await file.path();
    const md = await readFile(path, 'utf8');
    const heading = md.split('\n').find((l) => /^#{1,6} /.test(l));
    expect(heading).toBe('# Plan');
    expect(md).toContain('quokka');
  });

  await test.step('16. Review fixes: Recently viewed, item links that open, the Welcome board preview, a low row menu', async () => {
    // Recently viewed fills from the pages' POST …/view: C was opened above.
    await page.goto('/home');
    const viewed = page.getByRole('tabpanel', { name: 'Recently viewed' });
    await expect(viewed.locator('.item-list-title', { hasText: /^C$/ })).toBeVisible();
    // A document with an item link to C, written as an agent would (quire://item/<uuid>), from the page (cookie + Origin).
    const aRef = /\/doc\/(?:[^/]*-)?([0-9A-Za-z]{10})\/?$/.exec(new URL(aUrl).pathname)?.[1] ?? '';
    const a = await detail(page, aRef);
    const written = await page.evaluate(async ({ collection, target }) => {
      const created = await fetch('/api/v1/items', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ collection_id: collection, kind: 'doc', title: 'Links' }),
      });
      const item = await created.json();
      const put = await fetch(`/api/v1/items/${item.id}/doc/markdown`, {
        method: 'PUT',
        headers: { 'Content-Type': 'text/markdown', 'If-Match': '*' },
        body: `Go to [the plan](quire://item/${target}).\n`,
      });
      return { url_id: item.url_id as string, created: created.status, put: put.status };
    }, { collection: a.item.collection_id, target: cId });
    expect(written).toMatchObject({ created: 201, put: 200 });
    await page.goto(`/doc/${written.url_id}`);
    const link = editor(page).getByRole('link', { name: 'the plan' });
    await expect(link).toHaveAttribute('href', `/item/${cId}`);
    // Editable: a plain click places the caret; Ctrl+click follows the link.
    await link.click();
    await page.waitForTimeout(300);
    expect(currentRef(page)).toBe(written.url_id);
    await link.click({ modifiers: ['Control'] });
    await page.waitForURL((u) => u.pathname === new URL(cUrl).pathname);
    await expect(page.locator('#item-title')).toHaveValue('C');
    // The /item/<uuid> step was replaced: Back returns to the Links document.
    await page.goBack();
    await page.waitForURL(`**/doc/*${written.url_id}`);
    // The Welcome document embeds the seeded board, whose preview is rendered on demand (no stored preview yet).
    const hits = await (await page.request.get('/api/v1/search/titles?q=Welcome')).json();
    const welcome = (hits as Json[]).find((h) => h.item.title === 'Welcome to Quire');
    expect(welcome, 'the Welcome document').toBeTruthy();
    await page.goto(`/doc/${welcome.item.url_id}`);
    const preview = page.locator('.board-embed img.board-embed-preview');
    await expect(preview).toHaveCount(1, { timeout: SAVE_TIMEOUT });
    await preview.scrollIntoViewIfNeeded({ timeout: SAVE_TIMEOUT });
    // evaluateAll never waits: a failed load replaces the <img> with the placeholder, which fails the poll in time.
    await expect.poll(
      () => preview.evaluateAll((els) => els.length === 1 && (els[0] as HTMLImageElement).complete && (els[0] as HTMLImageElement).naturalWidth > 0),
      { timeout: SAVE_TIMEOUT, message: 'the Welcome board card loads its preview' },
    ).toBe(true);
    // A row menu near the bottom of the window opens upward: every item, Delete included, stays inside the window.
    // The window is cut 24 px below the lowest sidebar row that has room for the menu above it.
    const viewport = page.viewportSize();
    if (!viewport) throw new Error('no viewport');
    const rows = sidebar(page).getByRole('treeitem');
    const boxes = await rows.evaluateAll((els) =>
      els.map((el) => ({ level: Number(el.getAttribute('aria-level')), ...el.getBoundingClientRect().toJSON() })));
    // An item row (level 2 and deeper: its menu is the item menu of §9.6.2), low enough for that menu to fit above it.
    const low = boxes
      .map((b, i) => ({ top: b.top as number, bottom: b.bottom as number, level: b.level, i }))
      .filter((b) => b.level >= 2 && b.top >= 300 && b.bottom <= viewport.height)
      .pop();
    expect(low, `an item row with its top at 300 px or lower (rows: ${JSON.stringify(boxes)})`).toBeTruthy();
    const height = Math.ceil((low?.bottom ?? 0) + 24);
    await page.setViewportSize({ width: viewport.width, height });
    const row = rows.nth(low?.i ?? 0);
    await row.hover();
    await row.getByRole('button', { name: / menu$/ }).first().click();
    const menu = page.getByRole('menu');
    const del = menu.getByRole('menuitem', { name: /^Delete/ }).first();
    const [menuBox, delBox] = [await menu.boundingBox(), await del.boundingBox()];
    if (!menuBox || !delBox) throw new Error('the row menu has no box');
    expect(menuBox.y + menuBox.height, 'the menu opened above its row').toBeLessThanOrEqual((low?.top ?? 0) + 1);
    expect(delBox.y).toBeGreaterThanOrEqual(0);
    expect(delBox.y + delBox.height).toBeLessThanOrEqual(height);
    await page.keyboard.press('Escape');
    await page.setViewportSize(viewport);
  });

  await test.step('17. Board document embeds: Text doc, Embed page…, a sidebar drop, double-click editing, autosave, reload', async (step) => {
    step.skip(!engine, 'WebGL 2 is unavailable (step 9)');
    // A larger window than the default 1280 × 720, so that three 480 × 360 cards at zoom 1 leave empty canvas around them
    // (docs/handoff/E-merge.md); restored at the end.
    const viewport = page.viewportSize();
    if (!viewport) throw new Error('no viewport');
    await page.setViewportSize({ width: 1600, height: 1000 });
    const aRef = /\/doc\/(?:[^/]*-)?([0-9A-Za-z]{10})\/?$/.exec(new URL(aUrl).pathname)?.[1] ?? '';
    const aId = (await detail(page, aRef)).item.id as string;

    // 1. A new board "Embed board" under A, its engine ready.
    await page.goto(aUrl);
    await expect(page.locator('#item-title')).toHaveValue('A');
    await newNested(page, 'board');
    await setTitle(page, page.getByRole('navigation', { name: 'Breadcrumbs' }).getByRole('textbox', { name: 'Title' }), EMBED_BOARD);
    const embedBoardUrl = page.url();
    const embedBoardId = (await detail(page, urlIdOf(page))).item.id as string;
    await waitForEngine(page);
    const textDoc = page.getByRole('button', { name: 'Text doc', exact: true });
    const embedPage = page.getByRole('button', { name: 'Embed page…', exact: true });
    await expect(textDoc).toBeEnabled();
    await expect(embedPage).toBeEnabled();
    const frameBox = async () => {
      const b = await boardFrame(page).boundingBox();
      if (!b) throw new Error('the board frame has no box');
      return b;
    };
    const frame = await frameBox();
    const centre = { x: frame.x + frame.width / 2, y: frame.y + frame.height / 2 };

    // 2. "Text doc": a child of the board, a card editing at the centre, typing autosaves, the title input renames it.
    const created = page.waitForResponse((r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/v1/items');
    await textDoc.click();
    const post = await created;
    expect(post.status()).toBe(201);
    expect(post.request().postDataJSON()).toMatchObject({ parent_id: embedBoardId, kind: 'doc' });
    const docId = (await post.json()).id as string;
    const boardRow = treeItem(page, EMBED_BOARD);
    await expect(boardRow).toHaveAttribute('aria-expanded', 'true');
    const boardLevel = Number(await boardRow.getAttribute('aria-level'));
    const child = (name: string) => treeItem(page, name).and(page.locator(`[aria-level="${boardLevel + 1}"]`));
    await expect(child('Untitled')).toBeVisible();
    const docCard = cardOf(page, docId);
    await expect(docCard).toHaveAttribute('data-state', 'editing');
    await expectCentredOn(docCard, centre, 'the Text doc card');
    const docEditor = docCard.locator('.board-embed-editor .ProseMirror[contenteditable="true"]');
    await expect(docEditor).toBeFocused();
    let saved = page.waitForResponse((r) => isDocUpdateOf(r, docId) && r.ok(), { timeout: SAVE_TIMEOUT });
    await page.keyboard.type('wombat');
    await saved;
    await expect(docCard.locator('.board-embed-status')).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    expect((await (await page.request.get(`/api/v1/items/${docId}/text`)).json()).body).toContain('wombat');
    const titleInput = docCard.getByRole('textbox', { name: 'Page title' });
    await expect(titleInput).toHaveValue('');
    await expect(titleInput).toHaveAttribute('placeholder', 'Untitled');
    const renamed = page.waitForResponse((r) => isItemPatch(r) && r.ok());
    await titleInput.click();
    await page.keyboard.type('Wombat notes');
    await renamed;
    await expect(child('Wombat notes')).toBeVisible();

    // 3. Esc in the title input: the card is static and still shows "wombat", under its new title.
    await page.keyboard.press('Escape');
    await expect(docCard).toHaveAttribute('data-state', 'static');
    await expect(docCard.locator('.board-embed-static .ProseMirror')).toContainText('wombat');
    await expect(docCard.locator('.board-embed-card-title')).toHaveText('Wombat notes');

    // 4. "Embed page…": find C and choose it: a second card near the centre, static, showing "quokka".
    await embedPage.click();
    const picker = page.getByRole('dialog', { name: 'Embed a page' });
    await picker.getByPlaceholder('Find a page').fill('C');
    await picker.getByRole('button', { name: 'C', exact: true }).click();
    await expect(picker).toHaveCount(0);
    const cCard = cardOf(page, cId);
    await expect(cCard).toHaveAttribute('data-state', 'static');
    await expectCentredOn(cCard, centre, 'the card of C');
    await expect(cCard.locator('.board-embed-static .ProseMirror')).toContainText('quokka');

    // 5. A real drag of A's sidebar row onto empty canvas at P: a third card centred on P.
    const p = { x: frame.x + frame.width * 0.86, y: centre.y };
    const aRow = treeItem(page, 'A');
    await expect(aRow).toBeVisible();
    const rowBox = await aRow.locator('.row-label').boundingBox();
    if (!rowBox) throw new Error('the sidebar row of A has no box');
    await page.mouse.move(rowBox.x + Math.min(20, rowBox.width / 2), rowBox.y + rowBox.height / 2);
    await page.mouse.down();
    for (let i = 1; i <= 20; i++) {
      const from = { x: rowBox.x + Math.min(20, rowBox.width / 2), y: rowBox.y + rowBox.height / 2 };
      await page.mouse.move(from.x + ((p.x - from.x) * i) / 20, from.y + ((p.y - from.y) * i) / 20);
      await page.waitForTimeout(30);
    }
    await page.mouse.up();
    const aCard = cardOf(page, aId);
    await expect(aCard).toHaveAttribute('data-state', 'static', { timeout: SAVE_TIMEOUT });
    await expectCentredOn(aCard, p, 'the card of A');

    // 6. After the board's save: three cards (the new document, C, A), the first labelled with its new title.
    await expect(saveStatus(page)).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    await expect.poll(async () => (await embedCards(page, embedBoardId)).map((c) => c.item), { timeout: SAVE_TIMEOUT }).toEqual([docId, cId, aId]);
    expect((await embedCards(page, embedBoardId))[0].label).toBe('Wombat notes');

    // 7. A single click selects C's card (no editing); a double-click on the word "quokka" edits it; End, " numbat",
    //    saved; a click on empty canvas leaves it (no new object). Zoomed in until the card is taller than the canvas,
    //    the caret at the end and ten Enters: nothing scrolls the overlay and the card stays on its box.
    const word = await wordBox(cCard.locator('.board-embed-static .ProseMirror'), 'quokka');
    await page.mouse.click(word.x, word.y);
    await expect(cCard.locator('.board-embed-hint')).toHaveText('Double-click to edit');
    await page.waitForTimeout(600);
    await expect(cCard).toHaveAttribute('data-state', 'static');
    await page.mouse.dblclick(word.x, word.y);
    await expect(cCard).toHaveAttribute('data-state', 'editing');
    const cEditor = cCard.locator('.board-embed-editor .ProseMirror[contenteditable="true"]');
    await expect(cEditor).toBeFocused();
    await settle(page);
    await page.keyboard.press('End');
    saved = page.waitForResponse((r) => isDocUpdateOf(r, cId) && r.ok(), { timeout: SAVE_TIMEOUT });
    await page.keyboard.type(' numbat');
    await saved;
    await expect(cCard.locator('.board-embed-status')).toHaveText('Saved', { timeout: SAVE_TIMEOUT });
    const cText = async () => (await (await page.request.get(`/api/v1/items/${cId}/text`)).json()).body as string;
    //    While C is edited (the keyboard focus in its editor, so the frame never hears of Shift or Alt), the board still pans
    //    from empty canvas above the card: Shift + wheel sideways, Alt + wheel up and down, a middle-button drag; the
    //    card stays editing, and the caret is back in it after the drag (E review, finding 2).
    const boxOf = async () => {
      const b = await cCard.boundingBox();
      if (!b) throw new Error("C's card has no box");
      return b;
    };
    const at0 = await boxOf();
    const above = { x: at0.x + at0.width / 2, y: at0.y - 40 };
    await page.mouse.move(above.x, above.y);
    await page.keyboard.down('Shift');
    await page.mouse.wheel(0, -120);
    await page.keyboard.up('Shift');
    await expect.poll(async () => Math.round((await boxOf()).x - at0.x), { message: 'Shift + wheel pans sideways' }).not.toBe(0);
    const at1 = await boxOf();
    expect(at1.width, 'Shift + wheel does not zoom').toBeCloseTo(at0.width, 0);
    expect(at1.y).toBeCloseTo(at0.y, 0);
    await page.keyboard.down('Alt');
    await page.mouse.wheel(0, -120);
    await page.keyboard.up('Alt');
    await expect.poll(async () => Math.round((await boxOf()).y - at1.y), { message: 'Alt + wheel pans up and down' }).toBeGreaterThan(0);
    const at2 = await boxOf();
    expect(at2.width, 'Alt + wheel does not zoom').toBeCloseTo(at0.width, 0);
    expect(at2.x).toBeCloseTo(at1.x, 0);
    await page.mouse.down({ button: 'middle' });
    for (let i = 1; i <= 10; i++) {
      await page.mouse.move(above.x + 6 * i, above.y + 3 * i);
      await page.waitForTimeout(30);
    }
    await page.mouse.up({ button: 'middle' });
    await expect.poll(async () => Math.round((await boxOf()).x - at2.x), { message: 'the middle button pans' }).toBe(60);
    expect(Math.round((await boxOf()).y - at2.y)).toBe(30);
    await expect(cCard).toHaveAttribute('data-state', 'editing');
    await expect(cEditor).toBeFocused();
    saved = page.waitForResponse((r) => isDocUpdateOf(r, cId) && r.ok(), { timeout: SAVE_TIMEOUT });
    await page.keyboard.type(' wallaby');
    await saved;
    await expect.poll(cText, { timeout: SAVE_TIMEOUT }).toContain('quokka numbat wallaby');
    const typed = await cText();
    expect(typed).toContain('quokka numbat');
    // Empty canvas: above the two cards at the centre, clear of the engine's floating cards at the top.
    const cBox = await cCard.boundingBox();
    if (!cBox) throw new Error("C's card has no box");
    const empty = { x: cBox.x + cBox.width / 2, y: cBox.y - 40 };
    await page.mouse.click(empty.x, empty.y);
    await expect(cCard).toHaveAttribute('data-state', 'static');
    await page.waitForTimeout(3000);
    expect((await embedCards(page, embedBoardId)).length).toBe(3);
    expect((await detail(page, embedBoardId)).board.objects).toBe(3);

    await page.mouse.move(empty.x, empty.y);
    for (let i = 0; i < 40; i++) {
      const [card, f] = [await cCard.boundingBox(), await frameBox()];
      if (card && card.height > f.height) break;
      await page.mouse.wheel(0, -120);
      await page.waitForTimeout(80);
    }
    const tall = await cCard.boundingBox();
    if (!tall) throw new Error("C's card has no box");
    expect(tall.height, "C's card is taller than the canvas").toBeGreaterThan((await frameBox()).height);
    const header = await cCard.locator('.board-embed-header').boundingBox();
    if (!header) throw new Error("C's card has no header box");
    await page.mouse.dblclick(tall.x + tall.width / 2, header.y + header.height + 20);
    await expect(cCard).toHaveAttribute('data-state', 'editing');
    await expect(cEditor).toBeFocused();
    await settle(page);
    const before = await cCard.evaluate((el) => el.getBoundingClientRect().toJSON());
    await page.keyboard.press('ControlOrMeta+End');
    for (let i = 0; i < 10; i++) await page.keyboard.press('Enter');
    await page.waitForTimeout(300);
    const scrolled = await page.locator('.board-embeds').evaluate((el) => ({ top: el.scrollTop, left: el.scrollLeft }));
    expect(scrolled, '.board-embeds never scrolls (overflow: clip)').toEqual({ top: 0, left: 0 });
    const afterKeys = await cCard.evaluate((el) => el.getBoundingClientRect().toJSON());
    for (const k of ['x', 'y', 'width', 'height'] as const) {
      expect(Math.abs(afterKeys[k] - before[k]), `the card's ${k} after the keys`).toBeLessThanOrEqual(2);
    }
    for (let i = 0; i < 10; i++) await page.keyboard.press('ControlOrMeta+z');
    await page.mouse.click(empty.x, empty.y);
    await expect(cCard).toHaveAttribute('data-state', 'static');
    await expect.poll(cText, { timeout: SAVE_TIMEOUT, message: 'ten undos took the ten Enters back' }).toBe(typed);
    await expect(cCard.locator('.board-embed-static .ProseMirror')).toContainText('quokka numbat');
    //    Esc in the body (not in the title input) leaves too: ProseMirror prevents the default of every Esc, so only the
    //    editor's own key handling can tell (E review, finding 1). The card stays selected and the frame has the focus.
    await page.mouse.dblclick(tall.x + tall.width / 2, header.y + header.height + 20);
    await expect(cCard).toHaveAttribute('data-state', 'editing');
    await expect(cEditor).toBeFocused();
    await settle(page);
    await page.keyboard.press('Escape');
    await expect(cCard).toHaveAttribute('data-state', 'static');
    await expect(boardFrame(page)).toBeFocused();
    await expect(cCard.locator('.board-embed-hint')).toHaveText('Double-click to edit');

    // 8. Reload: the three cards are static with their documents' text; C's page holds "quokka numbat".
    expect(page.url()).toBe(embedBoardUrl);
    await page.reload();
    await waitForEngine(page);
    for (const [id, text] of [[docId, 'wombat'], [cId, 'quokka numbat'], [aId, BOARD_TITLE]] as const) {
      const card = cardOf(page, id);
      await expect(card).toHaveAttribute('data-state', 'static', { timeout: SAVE_TIMEOUT });
      await expect(card.locator('.board-embed-static')).toContainText(text);
    }
    await page.goto(cUrl);
    await expect(page.locator('#item-title')).toHaveValue('C');
    await expect(editor(page)).toContainText('quokka numbat');
    await page.setViewportSize(viewport);
    console.log('smoke: step 17 ran on the engine: Text doc, Embed page…, a real sidebar drag, double-click editing, reload');
  });

  if (pageErrors.length > 0) testInfo.annotations.push({ type: 'page errors', description: pageErrors.join('\n') });
  await test.step('No CSP violation on any page of the run', async () => {
    expect(cspViolations).toEqual([]);
  });
});
