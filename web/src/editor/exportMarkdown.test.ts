// Export → Markdown downloads the server's `GET …/doc/markdown?download=true` (docs/ARCHITECTURE.md §9.6.4, D49);
// the client has no Markdown code of its own.
import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { docExports } from './exports';

afterEach(() => vi.restoreAllMocks());

describe('Export Markdown', () => {
  it('downloads GET /api/v1/items/{id}/doc/markdown?download=true', async () => {
    const clicked: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      clicked.push(this.getAttribute('href') ?? '');
    });
    const fetchSpy = vi.fn();
    vi.stubGlobal('fetch', fetchSpy);
    const [md, html] = docExports('0192f0a0-1111-7000-8000-00000000000a', () => 'T', () => null);
    expect(md.label).toBe('Markdown');
    expect(html.label).toBe('HTML');
    await md.run();
    expect(clicked).toEqual(['/api/v1/items/0192f0a0-1111-7000-8000-00000000000a/doc/markdown?download=true']);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('has no client-side Markdown code', () => {
    const pkg = JSON.parse(readFileSync('package.json', 'utf8')) as { dependencies: Record<string, string>; devDependencies: Record<string, string> };
    const deps = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies });
    expect(deps.filter((d) => /markdown|remark|marked|turndown|showdown/i.test(d))).toEqual([]);
  });
});
