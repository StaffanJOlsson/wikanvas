// Placeholder for the Inkboard frame test (W0b; W5 replaces it with every item of the docs/ARCHITECTURE.md §14
// "Inkboard patch" row and the two extra checks of PLAN W5). It only checks that serve-static.mjs serves the dist of
// `scripts/inkboard.sh build` or `stub` and that Chromium opens the frame page.
import { expect, test } from '@playwright/test';

test('serves the Inkboard dist', async ({ request }) => {
  const manifest = await request.get('/manifest.json');
  expect(manifest.status()).toBe(200);
  expect(manifest.headers()['content-type']).toBe('application/json');
  expect(typeof (await manifest.json()).format_version).toBe('number');

  const frame = await request.get('/frame.html');
  expect(frame.status()).toBe(200);
  expect(frame.headers()['content-type']).toContain('text/html');
});

test('opens the frame page', async ({ page }) => {
  const response = await page.goto('/frame.html');
  expect(response?.status()).toBe(200);
  await expect(page.locator('body')).not.toBeEmpty();
});
