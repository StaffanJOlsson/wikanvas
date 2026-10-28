// Image uploads go out as application/octet-stream, whatever the file's type (docs/ARCHITECTURE.md §7.1, §9.2).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { uploadFile, uploadUrl } from './upload';

class FakeXhr {
  static last: FakeXhr | null = null;
  method = '';
  url = '';
  headers: Record<string, string> = {};
  body: unknown = null;
  status = 0;
  responseText = '';
  responseType = '';
  withCredentials = true;
  upload: { onprogress: ((e: { lengthComputable: boolean; loaded: number; total: number }) => void) | null } = {
    onprogress: null,
  };
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onabort: (() => void) | null = null;
  constructor() {
    FakeXhr.last = this;
  }
  open(method: string, url: string) {
    this.method = method;
    this.url = url;
  }
  setRequestHeader(k: string, v: string) {
    this.headers[k] = v;
  }
  getResponseHeader() {
    return 'req-1';
  }
  send(body: unknown) {
    this.body = body;
  }
  abort() {
    this.onabort?.();
  }
}

afterEach(() => {
  FakeXhr.last = null;
});

describe('uploadFile', () => {
  for (const type of ['image/png', 'text/plain', '', 'application/pdf']) {
    it(`sends a ${type || 'typeless'} file as application/octet-stream`, async () => {
      vi.stubGlobal('XMLHttpRequest', FakeXhr);
      const file = new File([new Uint8Array([137, 80, 78, 71])], 'photo.png', { type });
      const progress: number[] = [];
      const done = uploadFile(file, file.name, { itemId: 'doc-1', onProgress: (f) => progress.push(f) });
      const xhr = FakeXhr.last as FakeXhr;
      expect(xhr.method).toBe('POST');
      expect(xhr.url).toBe(uploadUrl('doc-1', 'photo.png'));
      expect(xhr.url).toBe('/api/v1/attachments?item_id=doc-1&filename=photo.png');
      expect(xhr.headers['Content-Type']).toBe('application/octet-stream');
      expect(xhr.body).toBe(file);
      xhr.upload.onprogress?.({ lengthComputable: true, loaded: 2, total: 4 });
      xhr.status = 201;
      xhr.responseText = JSON.stringify({ id: 'att-1', url: '/api/v1/attachments/att-1', sha256: '', mime: 'image/png', size: 4, width: 1, height: 1 });
      xhr.onload?.();
      await expect(done).resolves.toMatchObject({ id: 'att-1' });
      expect(progress).toEqual([0.5]);
    });
  }

  it('rejects with the error envelope', async () => {
    vi.stubGlobal('XMLHttpRequest', FakeXhr);
    const done = uploadFile(new Blob(['x']), 'a.txt', { itemId: 'doc-1' });
    const xhr = FakeXhr.last as FakeXhr;
    xhr.status = 507;
    xhr.responseText = JSON.stringify({ error: { code: 'insufficient_storage', message: 'full' }, request_id: 'r' });
    xhr.onload?.();
    await expect(done).rejects.toMatchObject({ status: 507, code: 'insufficient_storage', requestId: 'r' });
  });
});
