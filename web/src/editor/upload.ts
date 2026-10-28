// Image uploads (docs/ARCHITECTURE.md §7.1, §9.2, §9.6.5): `POST /api/v1/attachments?item_id&filename` with the raw
// bytes as `application/octet-stream`, whatever the file's own type (the server sniffs it; a browser's `text/plain`
// would get 415). XMLHttpRequest, because fetch reports no upload progress.
import { ApiError, apiUrl } from '../api/client';
import type { Attachment } from '../api/gen/Attachment';
import type { ErrorBody } from '../api/gen/ErrorBody';

export interface UploadOptions {
  /** The document the image is for (context only, D36). */
  itemId: string;
  /** 0…1 as the bytes go out. */
  onProgress?(fraction: number): void;
  signal?: AbortSignal;
}

/** The upload URL of a file. */
export function uploadUrl(itemId: string, filename: string): string {
  const q = new URLSearchParams({ item_id: itemId, filename: filename || 'file' });
  return apiUrl(`/attachments?${q.toString()}`);
}

/** Uploads one file; resolves with the attachment, or rejects with an `ApiError`. */
export function uploadFile(file: Blob, filename: string, opts: UploadOptions): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', uploadUrl(opts.itemId, filename));
    xhr.withCredentials = false;
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.responseType = 'text';
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && opts.onProgress) opts.onProgress(e.loaded / e.total);
    };
    xhr.onload = () => {
      const requestId = xhr.getResponseHeader('X-Request-Id');
      let body: unknown = null;
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        // Not JSON: handled below.
      }
      if (xhr.status === 201 || xhr.status === 200) {
        if (body && typeof body === 'object' && 'id' in body) resolve(body as Attachment);
        else reject(new ApiError(xhr.status, 'bad_response', 'Unreadable upload response', undefined, requestId));
        return;
      }
      const envelope = body as Partial<ErrorBody> | null;
      const info = envelope?.error;
      reject(
        info && typeof info.code === 'string'
          ? new ApiError(xhr.status, info.code, info.message ?? info.code, info.details ?? undefined, envelope?.request_id ?? requestId)
          : new ApiError(xhr.status, 'bad_response', `HTTP ${xhr.status}`, undefined, requestId),
      );
    };
    xhr.onerror = () => reject(new ApiError(0, 'network', 'Network error'));
    xhr.onabort = () => reject(new DOMException('Aborted', 'AbortError'));
    opts.signal?.addEventListener('abort', () => xhr.abort());
    xhr.send(file);
  });
}

/** Whether a file is an image the editor embeds (by its declared type; the server sniffs the real one). */
export function isImageFile(file: File): boolean {
  return file.type.startsWith('image/') || /\.(png|jpe?g|gif|webp|svg|avif)$/i.test(file.name);
}
