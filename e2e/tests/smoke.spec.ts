// Placeholder for the Playwright smoke (W0b; W7 replaces it with the "Smoke spec" steps of docs/PLAN.md W7). Quire runs
// from the production image in the compose e2e project and is reached directly at http://quire:8080 over plain HTTP
// (docs/ARCHITECTURE.md §13.3, §14).
import { expect, test } from '@playwright/test';

test('quire answers /healthz', async ({ request }) => {
  const response = await request.get('/healthz');
  expect(response.status()).toBe(200);
});
