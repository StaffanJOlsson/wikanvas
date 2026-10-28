// The link and image scheme allow-list of docs/ARCHITECTURE.md §6.4 (also TipTap Link's `protocols` and
// `isAllowedUri`, §9.6.5): http, https, mailto (links only), quire, same-origin paths `/…` and fragments `#…`.
// Everything else (`javascript:`, `data:`, `vbscript:`, protocol-relative `//host`, …) stays text.

/** Schemes a link may use, besides same-origin paths and fragments. */
export const LINK_SCHEMES = ['http', 'https', 'mailto', 'quire'] as const;
/** Schemes an external image may use. */
export const IMAGE_SCHEMES = ['http', 'https', 'quire'] as const;

/** Item links: `quire://item/<uuid>` (§6.4). */
export const QUIRE_ITEM_PREFIX = 'quire://item/';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// Control characters and whitespace that browsers strip before parsing a scheme ("java\tscript:").
// eslint-disable-next-line no-control-regex
const STRIPPED = /[\u0000- \u007f-\u009f]/g;

function allowed(href: string, schemes: readonly string[]): boolean {
  const s = href.replace(STRIPPED, '');
  if (s === '') return false;
  if (s.startsWith('#')) return true;
  if (s.startsWith('/')) return !s.startsWith('//') && !s.startsWith('/\\');
  const m = /^([a-z][a-z0-9+.-]*):/i.exec(s);
  if (!m) return false;
  return schemes.includes(m[1].toLowerCase());
}

/** Whether a link may point at `href` (§6.4 allow-list). */
export function isAllowedLink(href: string | null | undefined): boolean {
  return typeof href === 'string' && allowed(href, LINK_SCHEMES);
}

/** Whether an external image may load from `src` (§6.4 allow-list; never `mailto:` or `data:`). */
export function isAllowedImageSrc(src: string | null | undefined): boolean {
  return typeof src === 'string' && allowed(src, IMAGE_SCHEMES);
}

/** The item id of a `quire://item/<uuid>` URL, or null. */
export function quireItemId(href: string | null | undefined): string | null {
  if (typeof href !== 'string' || !href.startsWith(QUIRE_ITEM_PREFIX)) return null;
  const id = href.slice(QUIRE_ITEM_PREFIX.length);
  return UUID.test(id) ? id.toLowerCase() : null;
}

/** `quire://item/<id>`. */
export function quireItemUrl(id: string): string {
  return `${QUIRE_ITEM_PREFIX}${id}`;
}

/**
 * The url_id of a same-instance item URL (`/doc/<slug>-<url_id>` or `/board/<slug>-<url_id>`, relative or under
 * `origin`), or null. A pasted Quire URL becomes `link{itemId}` once the id is resolved (§9.6.5).
 */
export function sameInstanceUrlId(href: string, origin: string): string | null {
  let path = href;
  if (/^https?:\/\//i.test(href)) {
    let u: URL;
    try {
      u = new URL(href);
    } catch {
      return null;
    }
    if (u.origin !== origin) return null;
    path = u.pathname;
  }
  const m = /^\/(?:doc|board)\/(?:[^/?#]*-)?([0-9A-Za-z]{10})\/?(?:[?#].*)?$/.exec(path);
  return m ? m[1] : null;
}
