import { describe, expect, it } from 'vitest';
import {
  attachmentUrl,
  boardPreviewUrl,
  collectionPath,
  inkboardBase,
  inkboardFrameUrl,
  itemPath,
  safeNext,
  slugify,
  urlIdFromSlugId,
} from './urls';

describe('urls', () => {
  it('boardPreviewUrl is /api/v1/items/<id>/board/pages/<n>?rev=<r>, without .svg', () => {
    expect(boardPreviewUrl('0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b', 7, 2)).toBe(
      '/api/v1/items/0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b/board/pages/2?rev=7',
    );
  });

  it('attachmentUrl is /api/v1/attachments/<id>', () => {
    expect(attachmentUrl('abc')).toBe('/api/v1/attachments/abc');
  });

  it('itemPath and collectionPath are <slug>-<url_id> under the kind prefix', () => {
    expect(itemPath({ kind: 'doc', title: 'Hello, World!', url_id: 'AbCdEfGh12' })).toBe('/doc/hello-world-AbCdEfGh12');
    expect(itemPath({ kind: 'board', title: '', url_id: 'AbCdEfGh12' })).toBe('/board/untitled-AbCdEfGh12');
    expect(collectionPath({ name: 'Café déjà vu', url_id: 'Zz09Zz09Zz' })).toBe('/collection/cafe-deja-vu-Zz09Zz09Zz');
  });

  it('slugify folds accents, collapses separators and caps the length', () => {
    expect(slugify('  --Ünïcödé   & more--  ')).toBe('unicode-more');
    expect(slugify('日本語')).toBe('untitled');
    const long = slugify('word '.repeat(40));
    expect(long.length).toBeLessThanOrEqual(60);
    expect(long.endsWith('-')).toBe(false);
  });

  it('urlIdFromSlugId takes the last 10 characters', () => {
    expect(urlIdFromSlugId('hello-world-AbCdEfGh12')).toBe('AbCdEfGh12');
    expect(urlIdFromSlugId('AbCdEfGh12')).toBe('AbCdEfGh12');
    expect(urlIdFromSlugId('a-b-c-d-e-f-g')).toBeNull();
    expect(urlIdFromSlugId('helloAbCdEfGh12')).toBeNull();
    expect(urlIdFromSlugId('short')).toBeNull();
    expect(urlIdFromSlugId(undefined)).toBeNull();
    expect(urlIdFromSlugId(itemPath({ kind: 'doc', title: 'x', url_id: 'Q1w2E3r4T5' }).split('/')[2])).toBe('Q1w2E3r4T5');
  });

  it('inkboardBase reads config.inkboard.base, and null without an engine', () => {
    const inkboard = {
      base: '/inkboard/0123456789ab/',
      engine_version: '0.2.0-beta.3+embed.wikanvas.1',
      format_version: 2,
      wasm_bytes: { raw: 1, br: 1, gz: 1 },
    };
    expect(inkboardBase({ inkboard })).toBe('/inkboard/0123456789ab/');
    expect(inkboardFrameUrl({ inkboard })).toBe('/inkboard/0123456789ab/frame.html');
    expect(inkboardBase({ inkboard: null })).toBeNull();
    expect(inkboardFrameUrl(undefined)).toBeNull();
  });

  it('safeNext keeps same-origin paths only', () => {
    expect(safeNext('/doc/x-AbCdEfGh12?a=1#h')).toBe('/doc/x-AbCdEfGh12?a=1#h');
    expect(safeNext(null)).toBe('/home');
    expect(safeNext('https://evil.example/')).toBe('/home');
    expect(safeNext('//evil.example/')).toBe('/home');
    expect(safeNext('/\\evil.example')).toBe('/home');
    expect(safeNext('/login?next=/x')).toBe('/home');
    expect(safeNext('/login#x')).toBe('/home');
    expect(safeNext('/setup')).toBe('/home');
  });

  it('safeNext refuses control characters the URL parser would drop (no open redirect)', () => {
    // `?next=/%09/evil.example` decodes to "/\t/evil.example", which `new URL` reads as //evil.example.
    expect(new URL('/\t/evil.example', 'http://x.invalid').host).toBe('evil.example');
    expect(safeNext('/\t/evil.example')).toBe('/home');
    expect(safeNext('/\n/evil.example')).toBe('/home');
    expect(safeNext('/\r/evil.example')).toBe('/home');
    expect(safeNext('/\u0000x')).toBe('/home');
    // Dot segments that collapse to `//host` after parsing.
    expect(safeNext('/a/..//evil.example')).toBe('/home');
    expect(safeNext('/.//evil.example')).toBe('/home');
    // Ordinary targets come back parsed, the same path.
    expect(safeNext('/search?q=a%20b&kind=doc')).toBe('/search?q=a%20b&kind=doc');
    expect(safeNext('/settings/tokens')).toBe('/settings/tokens');
  });
});
