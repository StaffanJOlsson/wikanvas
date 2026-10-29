// URLs of the SPA's routes (docs/ARCHITECTURE.md §9.1) and of the API resources pages link to (§7.2, §11).
import { API_BASE } from '../api/client';
import type { Config } from '../api/gen/Config';
import type { ItemKind } from '../api/gen/ItemKind';

/** Length of a `url_id` (10 base62 characters, D4): the last characters of a `:slugId`. */
export const URL_ID_LENGTH = 10;

const MAX_SLUG = 60;

/**
 * The slug part of `<slug>-<url_id>`: lowercase ASCII letters and digits separated by single dashes, at most 60
 * characters, accents folded ("Café déjà vu" → "cafe-deja-vu"), "untitled" when nothing is left. Only cosmetic: routes
 * resolve by the url_id, and pages `history.replace` to the canonical slug after loading.
 */
export function slugify(title: string): string {
  const slug = title
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, MAX_SLUG)
    .replace(/-+$/, '');
  return slug || 'untitled';
}

/** The `url_id` of a `:slugId` route parameter, or null when it cannot hold one. */
export function urlIdFromSlugId(slugId: string | undefined): string | null {
  if (!slugId || slugId.length < URL_ID_LENGTH) return null;
  const id = slugId.slice(-URL_ID_LENGTH);
  if (!/^[0-9A-Za-z]+$/.test(id)) return null;
  if (slugId.length > URL_ID_LENGTH && slugId[slugId.length - URL_ID_LENGTH - 1] !== '-') return null;
  return id;
}

/** `/doc/<slug>-<url_id>` or `/board/<slug>-<url_id>`, for an `ItemSummary`, a `Crumb` or a `SearchHit.item`. */
export function itemPath(item: { kind: ItemKind; title: string; url_id: string }): string {
  return `/${item.kind}/${slugify(item.title)}-${item.url_id}`;
}

/** `/collection/<slug>-<url_id>`. */
export function collectionPath(collection: { name: string; url_id: string }): string {
  return `/collection/${slugify(collection.name)}-${collection.url_id}`;
}

/** One page's SVG preview of a board at a revision: `/api/v1/items/{id}/board/pages/{page}?rev={rev}` (no `.svg`). */
export function boardPreviewUrl(id: string, rev: number, page: number): string {
  return `${API_BASE}/items/${encodeURIComponent(id)}/board/pages/${page}?rev=${rev}`;
}

/** An attachment's bytes: `/api/v1/attachments/{id}`. */
export function attachmentUrl(id: string): string {
  return `${API_BASE}/attachments/${encodeURIComponent(id)}`;
}

/** The engine build's base path, `/inkboard/<build>/` (ends with `/`), or null when no engine is embedded. */
export function inkboardBase(config: Pick<Config, 'inkboard'> | null | undefined): string | null {
  return config?.inkboard?.base ?? null;
}

/** The board frame page of the embedded engine, or null when there is none (boards open in BoardPreview, §9.5). */
export function inkboardFrameUrl(config: Pick<Config, 'inkboard'> | null | undefined): string | null {
  const base = inkboardBase(config);
  return base === null ? null : `${base}frame.html`;
}

/** The origin `safeNext` resolves against; any other origin after parsing means the target left the app. */
const NEXT_BASE = 'http://wikanvas.invalid';

/**
 * The `?next=` target after signing in: a same-origin path only, else `/home`. Refused: a scheme, `//host`, a
 * backslash, and any control character (the URL parser drops tab and newline, so `/\t/evil.example` would become
 * `//evil.example`). The rest is parsed as a URL against a fixed origin, which must stay the same; the result is the
 * parsed path, query and hash. `/login` and `/setup` also give `/home`.
 */
export function safeNext(next: string | null | undefined): string {
  if (!next || !next.startsWith('/') || next.startsWith('//') || /[\\\u0000-\u001f\u007f]/.test(next)) return '/home';
  let url: URL;
  try {
    url = new URL(next, NEXT_BASE);
  } catch {
    return '/home';
  }
  if (url.origin !== NEXT_BASE || url.pathname.startsWith('//')) return '/home';
  if (url.pathname === '/login' || url.pathname.startsWith('/setup')) return '/home';
  return url.pathname + url.search + url.hash;
}
