// The overlay's geometry (docs/ARCHITECTURE.md §18.5), pure functions only, so the coordinate math is tested without
// a browser or an engine:
// - world units → frame px → viewport px (`screen = origin + world × zoom`; the iframe's client rect adds the rest);
// - a card's inset box, in world units, and the box of its header and body;
// - `chooseCards`: which cards the host draws (the editing one always, others from zoom 0.5 and 120 × 80 px on
//   screen, at most 24, the largest first, with hysteresis so a threshold does not mount and unmount viewers);
// - `clipHoles` / `clipPath`: the canvas minus the engine's own interface, as disjoint rectangles (evenodd);
// - `caretFor`: where the caret goes when a double-click opens a card.
import type { EmbedBox, EmbedLayout, FramePoint, FrameRect } from '../bridge';

/** The margin between the engine's box and the host's content, in world units (6 px at the minimum zoom). */
export const INSET = 12;
/** The card header's height, in world units. */
export const HEADER = 40;
/** Static cards are drawn from this zoom on… */
export const MIN_ZOOM = 0.5;
/** …and at least this size on screen (CSS px)… */
export const MIN_WIDTH = 120;
export const MIN_HEIGHT = 80;
/** …when their screen bounds meet the canvas grown by this margin (px). */
export const MARGIN = 200;
/** A card drawn in the previous layout stays drawn down to these (hysteresis). */
export const KEEP_ZOOM = 0.45;
export const KEEP_WIDTH = 108;
export const KEEP_HEIGHT = 72;
export const KEEP_MARGIN = 400;
/** At most this many static cards (the editing card comes on top). */
export const MAX_STATIC_CARDS = 24;
/** A static card shows at most this many top-level blocks. */
export const MAX_STATIC_BLOCKS = 200;

export interface Point {
  x: number;
  y: number;
}

/** A box in world units, as CSS `left`/`top`/`width`/`height`. */
export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

type Camera = Pick<EmbedLayout, 'zoom' | 'originX' | 'originY'>;

/** World units → frame px. */
export function worldToFrame(l: Camera, p: Point): FramePoint {
  return { x: l.originX + p.x * l.zoom, y: l.originY + p.y * l.zoom };
}

/** Frame px → world units. */
export function frameToWorld(l: Camera, p: FramePoint): Point {
  const z = l.zoom === 0 ? 1 : l.zoom;
  return { x: (p.x - l.originX) / z, y: (p.y - l.originY) / z };
}

/** Frame px → viewport px (`r`: the iframe's client rect; the iframe has no border or padding). */
export function frameToViewport(r: { left: number; top: number }, p: FramePoint): Point {
  return { x: r.left + p.x, y: r.top + p.y };
}

/** Viewport px → frame px. */
export function viewportToFrame(r: { left: number; top: number }, p: Point): FramePoint {
  return { x: p.x - r.left, y: p.y - r.top };
}

/** The host's box of a card, in world units: the frame inset by INSET, never negative. */
export function cardBox(e: Pick<EmbedBox, 'x' | 'y' | 'w' | 'h'>): Box {
  return {
    left: e.x + INSET,
    top: e.y + INSET,
    width: Math.max(0, e.w - 2 * INSET),
    height: Math.max(0, e.h - 2 * INSET),
  };
}

/** The body inside a card's box (below the header), in world units relative to the box. */
export function bodyBox(e: Pick<EmbedBox, 'w' | 'h'>): Box {
  const b = cardBox({ x: 0, y: 0, ...e });
  const top = Math.min(HEADER, b.height);
  return { left: 0, top, width: b.width, height: Math.max(0, b.height - top) };
}

/** The axis-aligned bounds of a card's rotated frame, in frame px. */
export function screenBounds(l: Camera, e: Pick<EmbedBox, 'x' | 'y' | 'w' | 'h' | 'angle'>): FrameRect {
  const cx = e.x + e.w / 2;
  const cy = e.y + e.h / 2;
  const cos = Math.cos(e.angle);
  const sin = Math.sin(e.angle);
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const [dx, dy] of [[-1, -1], [1, -1], [1, 1], [-1, 1]] as const) {
    const lx = (dx * e.w) / 2;
    const ly = (dy * e.h) / 2;
    // Positive angles turn clockwise on screen (y down), as CSS `rotate()` does.
    const p = worldToFrame(l, { x: cx + lx * cos - ly * sin, y: cy + lx * sin + ly * cos });
    minX = Math.min(minX, p.x);
    minY = Math.min(minY, p.y);
    maxX = Math.max(maxX, p.x);
    maxY = Math.max(maxY, p.y);
  }
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

function grow(r: FrameRect, by: number): FrameRect {
  return { x: r.x - by, y: r.y - by, width: r.width + 2 * by, height: r.height + 2 * by };
}

function intersect(a: FrameRect, b: FrameRect): FrameRect | null {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  const r = Math.min(a.x + a.width, b.x + b.width);
  const bt = Math.min(a.y + a.height, b.y + b.height);
  return r > x && bt > y ? { x, y, width: r - x, height: bt - y } : null;
}

function overlaps(a: FrameRect, b: FrameRect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/**
 * The cards the host draws, as object ids in drawing order (`layout.embeds` order, so a later card paints above an
 * earlier one): the editing card always; every other card when the zoom, its size on screen and its rotated bounds
 * pass the thresholds (the looser KEEP_* ones for a card in `previous`, the set drawn last time), at most
 * MAX_STATIC_CARDS of them, the largest on screen first (ties: drawing order). `overlay` (the overlay's size, when
 * known) further limits the visible region to the frame.
 */
export function chooseCards(
  layout: EmbedLayout,
  overlay: { width: number; height: number } | null,
  editing: string | null,
  previous: ReadonlySet<string>,
): string[] {
  let region: FrameRect | null = layout.canvas;
  if (overlay) region = intersect(region, { x: 0, y: 0, width: overlay.width, height: overlay.height });
  const candidates: { index: number; area: number }[] = [];
  const chosen = new Set<string>();
  layout.embeds.forEach((e, index) => {
    if (e.objectId === editing) {
      chosen.add(e.objectId);
      return;
    }
    if (region === null) return;
    const kept = previous.has(e.objectId);
    if (layout.zoom < (kept ? KEEP_ZOOM : MIN_ZOOM)) return;
    const w = e.w * layout.zoom;
    const h = e.h * layout.zoom;
    if (w < (kept ? KEEP_WIDTH : MIN_WIDTH) || h < (kept ? KEEP_HEIGHT : MIN_HEIGHT)) return;
    if (!overlaps(screenBounds(layout, e), grow(region, kept ? KEEP_MARGIN : MARGIN))) return;
    candidates.push({ index, area: w * h });
  });
  candidates.sort((a, b) => b.area - a.area || a.index - b.index);
  for (const c of candidates.slice(0, MAX_STATIC_CARDS)) chosen.add(layout.embeds[c.index].objectId);
  return layout.embeds.filter((e) => chosen.has(e.objectId)).map((e) => e.objectId);
}

/**
 * The engine's interface over the canvas as disjoint rectangles: each occluder intersected with `canvas`, then cut
 * into horizontal slabs at every distinct top and bottom edge, the x-intervals within a slab merged (touching ones
 * too). Their union is the union of the occluders inside the canvas, and no two of them overlap, so `evenodd` never
 * cancels a hole out.
 */
export function clipHoles(canvas: FrameRect, occluders: readonly FrameRect[]): FrameRect[] {
  const rects = occluders.map((o) => intersect(o, canvas)).filter((r): r is FrameRect => r !== null);
  if (rects.length === 0) return [];
  const ys = [...new Set(rects.flatMap((r) => [r.y, r.y + r.height]))].sort((a, b) => a - b);
  const out: FrameRect[] = [];
  for (let i = 0; i + 1 < ys.length; i++) {
    const y0 = ys[i];
    const y1 = ys[i + 1];
    const spans = rects
      .filter((r) => r.y <= y0 && r.y + r.height >= y1)
      .map((r) => [r.x, r.x + r.width] as [number, number])
      .sort((a, b) => a[0] - b[0]);
    let cur: [number, number] | null = null;
    for (const s of spans) {
      if (cur && s[0] <= cur[1]) cur[1] = Math.max(cur[1], s[1]);
      else {
        if (cur) out.push({ x: cur[0], y: y0, width: cur[1] - cur[0], height: y1 - y0 });
        cur = [s[0], s[1]];
      }
    }
    if (cur) out.push({ x: cur[0], y: y0, width: cur[1] - cur[0], height: y1 - y0 });
  }
  return out;
}

const n = (v: number) => String(Math.round(v * 100) / 100);

function rectPath(r: FrameRect): string {
  return `M${n(r.x)} ${n(r.y)}H${n(r.x + r.width)}V${n(r.y + r.height)}H${n(r.x)}Z`;
}

/** The overlay's `clip-path`: the canvas with the occluders cut out, in overlay-local px. */
export function clipPath(canvas: FrameRect, occluders: readonly FrameRect[]): string {
  const c = { x: canvas.x, y: canvas.y, width: Math.max(0, canvas.width), height: Math.max(0, canvas.height) };
  const d = [c, ...clipHoles(c, occluders)].map(rectPath).join('');
  return `path(evenodd, "${d}")`;
}

/** Where the caret goes when a card opens for editing (§18.5 Editing step 3). */
export type Caret = { kind: 'end' } | { kind: 'header' } | { kind: 'body'; left: number; top: number };

/**
 * The caret for a double-click at `point` (frame px of `openLayout`, the layout when the open was reported): the
 * point goes to world units with that camera and comes back through `layout` (the camera may have moved while the
 * document loaded), as viewport px (`frame`: the iframe's client rect). A point in the header gives `header`; Enter
 * and the menu (no point), a rotated card or a point outside the box give `end`.
 */
export function caretFor(
  point: FramePoint | null,
  openLayout: EmbedLayout | null,
  layout: EmbedLayout | null,
  objectId: string,
  frame: { left: number; top: number },
): Caret {
  if (!point || !openLayout || !layout) return { kind: 'end' };
  const then = openLayout.embeds.find((e) => e.objectId === objectId);
  const now = layout.embeds.find((e) => e.objectId === objectId);
  if (!then || !now || then.angle !== 0 || now.angle !== 0) return { kind: 'end' };
  const w = frameToWorld(openLayout, point);
  const box = cardBox(then);
  const local = { x: w.x - box.left, y: w.y - box.top };
  if (local.x < 0 || local.y < 0 || local.x > box.width || local.y > box.height) return { kind: 'end' };
  if (local.y < HEADER) return { kind: 'header' };
  const at = cardBox(now);
  const v = frameToViewport(frame, worldToFrame(layout, { x: at.left + local.x, y: at.top + local.y }));
  return { kind: 'body', left: v.x, top: v.y };
}
