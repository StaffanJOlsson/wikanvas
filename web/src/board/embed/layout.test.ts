// The overlay's geometry (docs/ARCHITECTURE.md §18.5): coordinates, which cards are drawn, inner boxes, the clip path
// and the caret of a double-click.
import { describe, expect, it } from 'vitest';
import type { EmbedBox, EmbedLayout, FrameRect } from '../bridge';
import {
  bodyBox,
  caretFor,
  cardBox,
  chooseCards,
  clipHoles,
  clipPath,
  frameToViewport,
  frameToWorld,
  HEADER,
  INSET,
  MAX_STATIC_CARDS,
  screenBounds,
  viewportToFrame,
  worldToFrame,
} from './layout';

function card(objectId: string, over: Partial<EmbedBox> = {}): EmbedBox {
  return { objectId, item: 'x', x: 0, y: 0, w: 480, h: 360, angle: 0, label: '', selected: false, locked: false, ...over };
}

function layout(over: Partial<EmbedLayout> = {}): EmbedLayout {
  return {
    itemId: 'b',
    page: 'p',
    zoom: 1,
    originX: 0,
    originY: 0,
    canvas: { x: 0, y: 0, width: 1000, height: 800 },
    occluders: [],
    embeds: [],
    items: [],
    editing: null,
    ...over,
  };
}

const area = (rs: FrameRect[]) => rs.reduce((a, r) => a + r.width * r.height, 0);

function disjoint(rs: FrameRect[]): boolean {
  for (let i = 0; i < rs.length; i++) {
    for (let j = i + 1; j < rs.length; j++) {
      const a = rs[i];
      const b = rs[j];
      if (a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height) return false;
    }
  }
  return true;
}

/** The union's area by sampling a grid (the shapes are on integer coordinates). */
function unionArea(rs: FrameRect[], within: FrameRect): number {
  let n = 0;
  for (let x = within.x; x < within.x + within.width; x++) {
    for (let y = within.y; y < within.y + within.height; y++) {
      if (rs.some((r) => x + 0.5 > r.x && x + 0.5 < r.x + r.width && y + 0.5 > r.y && y + 0.5 < r.y + r.height)) n++;
    }
  }
  return n;
}

describe('coordinates', () => {
  it('world → frame → viewport and back, with an iframe offset', () => {
    const l = layout({ zoom: 2, originX: 100, originY: 50 });
    const f = worldToFrame(l, { x: 10, y: -5 });
    expect(f).toEqual({ x: 120, y: 40 });
    const iframe = { left: 260, top: 52 };
    const v = frameToViewport(iframe, f);
    expect(v).toEqual({ x: 380, y: 92 });
    expect(viewportToFrame(iframe, v)).toEqual(f);
    expect(frameToWorld(l, f)).toEqual({ x: 10, y: -5 });
  });

  it('a card box is inset by 12 world units; a 20 × 20 card clamps at 0', () => {
    expect(INSET).toBe(12);
    expect(cardBox({ x: -240, y: -180, w: 480, h: 360 })).toEqual({ left: -228, top: -168, width: 456, height: 336 });
    expect(cardBox({ x: 5, y: 5, w: 20, h: 20 })).toEqual({ left: 17, top: 17, width: 0, height: 0 });
    expect(bodyBox({ w: 20, h: 20 })).toEqual({ left: 0, top: 0, width: 0, height: 0 });
    expect(bodyBox({ w: 480, h: 360 })).toEqual({ left: 0, top: HEADER, width: 456, height: 336 - HEADER });
  });

  it('rotated bounds grow to the rotated corners', () => {
    const b = screenBounds(layout(), card('a', { x: 0, y: 0, w: 100, h: 100, angle: Math.PI / 4 }));
    const d = 100 * Math.SQRT2;
    expect(b.width).toBeCloseTo(d);
    expect(b.height).toBeCloseTo(d);
    expect(b.x + b.width / 2).toBeCloseTo(50);
    expect(b.y + b.height / 2).toBeCloseTo(50);
  });
});

describe('chooseCards', () => {
  const none = new Set<string>();

  it('draws a card from zoom 0.5 and 120 × 80 px on screen', () => {
    const embeds = [card('a', { w: 240, h: 160 })];
    expect(chooseCards(layout({ zoom: 0.5, embeds }), null, null, none)).toEqual(['a']);
    expect(chooseCards(layout({ zoom: 0.49, embeds }), null, null, none)).toEqual([]);
    // 239 × 160 at 0.5 is 119.5 px wide.
    expect(chooseCards(layout({ zoom: 0.5, embeds: [card('a', { w: 239, h: 160 })] }), null, null, none)).toEqual([]);
    expect(chooseCards(layout({ zoom: 0.5, embeds: [card('a', { w: 240, h: 159 })] }), null, null, none)).toEqual([]);
  });

  it('keeps a drawn card down to 0.45 and 108 × 72 px, while a new one needs 0.5 and 120 × 80', () => {
    const embeds = [card('a', { w: 240, h: 160 }), card('b', { x: 300, w: 240, h: 160 })];
    const was = new Set(['a']);
    expect(chooseCards(layout({ zoom: 0.46, embeds }), null, null, was)).toEqual(['a']);
    expect(chooseCards(layout({ zoom: 0.45, embeds }), null, null, was)).toEqual(['a']);
    expect(chooseCards(layout({ zoom: 0.44, embeds }), null, null, was)).toEqual([]);
    // Sizes: 216 × 144 at 0.5 is exactly 108 × 72.
    const small = [card('a', { w: 216, h: 144 }), card('b', { x: 300, w: 216, h: 144 })];
    expect(chooseCards(layout({ zoom: 0.5, embeds: small }), null, null, was)).toEqual(['a']);
    const smaller = [card('a', { w: 214, h: 144 })];
    expect(chooseCards(layout({ zoom: 0.5, embeds: smaller }), null, null, was)).toEqual([]);
  });

  it('draws only cards whose rotated bounds meet the canvas grown by 200 px (400 px for a drawn one)', () => {
    const near = card('near', { x: 1150, y: 0 }); // 150 px right of the canvas
    const far = card('far', { x: 1300, y: 0 }); // 300 px right
    const l = layout({ embeds: [near, far] });
    expect(chooseCards(l, null, null, none)).toEqual(['near']);
    expect(chooseCards(l, null, null, new Set(['far']))).toEqual(['near', 'far']);
    // A rotated card whose unrotated box is outside but whose rotated corner reaches into the margin.
    const rotated = card('rot', { x: 1210, y: 100, w: 130, h: 600, angle: Math.PI / 2 });
    expect(chooseCards(layout({ embeds: [{ ...rotated, angle: 0 }] }), null, null, none)).toEqual([]);
    expect(chooseCards(layout({ embeds: [rotated] }), null, null, none)).toEqual(['rot']);
    const off = card('off', { x: -2000, y: -2000 });
    expect(chooseCards(layout({ embeds: [off] }), null, null, none)).toEqual([]);
  });

  it('the editing card is always drawn, whatever its zoom or place', () => {
    const embeds = [card('a', { x: -5000, y: -5000, w: 10, h: 10 })];
    expect(chooseCards(layout({ zoom: 0.1, embeds }), null, 'a', none)).toEqual(['a']);
  });

  it('draws at most 24 cards, the largest first, in drawing order', () => {
    const embeds = Array.from({ length: 30 }, (_, i) => card(`c${i}`, { x: (i % 6) * 20, y: Math.floor(i / 6) * 20, w: 200 + i, h: 160 }));
    const drawn = chooseCards(layout({ embeds }), null, null, none);
    expect(drawn).toHaveLength(MAX_STATIC_CARDS);
    // The six smallest (c0–c5) are left out; the rest stay in drawing order.
    expect(drawn).toEqual(embeds.slice(6).map((e) => e.objectId));
    // The editing card comes on top of the 24.
    expect(chooseCards(layout({ embeds }), null, 'c0', none)).toHaveLength(MAX_STATIC_CARDS + 1);
    // Ties keep drawing order.
    const same = Array.from({ length: 26 }, (_, i) => card(`s${i}`));
    expect(chooseCards(layout({ embeds: same }), null, null, none)).toEqual(same.slice(0, 24).map((e) => e.objectId));
  });

  it('the overlay size limits the visible region when known', () => {
    const embeds = [card('a', { x: 900, y: 0 })];
    expect(chooseCards(layout({ embeds }), { width: 600, height: 800 }, null, none)).toEqual([]);
    expect(chooseCards(layout({ embeds }), null, null, none)).toEqual(['a']);
  });
});

describe('clip path', () => {
  const canvas = { x: 0, y: 0, width: 100, height: 80 };

  it('overlapping occluders give disjoint holes whose area is the union', () => {
    const occ = [
      { x: 10, y: 10, width: 30, height: 30 },
      { x: 25, y: 20, width: 30, height: 30 },
      { x: 30, y: 15, width: 5, height: 5 },
    ];
    const holes = clipHoles(canvas, occ);
    expect(disjoint(holes)).toBe(true);
    expect(area(holes)).toBe(unionArea(occ, canvas));
  });

  it('edge-touching occluders merge; occluders are cut to the canvas; empty ones vanish', () => {
    const occ = [
      { x: 0, y: 0, width: 50, height: 10 },
      { x: 50, y: 0, width: 50, height: 10 },
      { x: 90, y: 70, width: 40, height: 40 },
      { x: 200, y: 200, width: 10, height: 10 },
    ];
    const holes = clipHoles(canvas, occ);
    expect(disjoint(holes)).toBe(true);
    expect(holes).toContainEqual({ x: 0, y: 0, width: 100, height: 10 });
    expect(holes).toContainEqual({ x: 90, y: 70, width: 10, height: 10 });
    expect(area(holes)).toBe(1000 + 100);
  });

  it('is an evenodd path of the canvas and the holes', () => {
    expect(clipPath(canvas, [])).toBe('path(evenodd, "M0 0H100V80H0Z")');
    expect(clipPath(canvas, [{ x: 10, y: 10, width: 20, height: 20 }])).toBe(
      'path(evenodd, "M0 0H100V80H0ZM10 10H30V30H10Z")',
    );
  });
});

describe('caretFor', () => {
  const frame = { left: 300, top: 60 };
  const c = card('a', { x: 0, y: 0, w: 480, h: 360 });

  it('maps a body point through world units, with the camera of the open and of now', () => {
    const then = layout({ zoom: 1, originX: 100, originY: 100, embeds: [c] });
    const now = layout({ zoom: 2, originX: 50, originY: 0, embeds: [c] });
    // Frame (212, 212) at the open is world (112, 112): local (100, 100) in the box, below the header.
    expect(caretFor({ x: 212, y: 212 }, then, now, 'a', frame)).toEqual({ kind: 'body', left: 300 + 50 + 112 * 2, top: 60 + 112 * 2 });
  });

  it('a point in the header focuses the title; no point, a rotated card or a point outside go to the end', () => {
    const l = layout({ embeds: [c] });
    expect(caretFor({ x: 30, y: 20 }, l, l, 'a', frame)).toEqual({ kind: 'header' });
    expect(caretFor(null, l, l, 'a', frame)).toEqual({ kind: 'end' });
    expect(caretFor({ x: 5, y: 5 }, l, l, 'a', frame)).toEqual({ kind: 'end' });
    const r = layout({ embeds: [card('a', { angle: 0.3 })] });
    expect(caretFor({ x: 100, y: 100 }, r, r, 'a', frame)).toEqual({ kind: 'end' });
  });
});
