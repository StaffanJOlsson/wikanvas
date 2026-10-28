// Menus open below their button, flip above it near the bottom of the window, and are capped to the roomier side when
// they fit neither (docs/ARCHITECTURE.md §9.6.2: a row's Archive and Delete must stay reachable).
import { describe, expect, it } from 'vitest';
import { placeMenu } from './Menu';

const viewport = { width: 1440, height: 900 };
const at = (top: number, left = 200) => ({ top, bottom: top + 24, left, right: left + 24 });

describe('placeMenu', () => {
  it('opens below the button when the menu fits there', () => {
    expect(placeMenu(at(100), 257, viewport, 'start')).toEqual({ top: 128, left: 200 });
  });

  it('opens above a button near the bottom of the window (a low sidebar row)', () => {
    // The review's case: the row at y ≈ 770, a 257 px menu, a 900 px window.
    const pos = placeMenu(at(770), 257, viewport, 'start');
    expect(pos).toEqual({ bottom: 900 - 770 + 4, left: 200 });
    // Its top edge (900 - bottom - 257) is inside the window.
    expect(900 - (pos.bottom ?? 0) - 257).toBeGreaterThanOrEqual(0);
  });

  it('caps the height on the roomier side when it fits neither', () => {
    const small = { width: 800, height: 300 };
    expect(placeMenu(at(100), 480, small, 'start')).toEqual({ top: 128, left: 200, maxHeight: 300 - 124 - 12 });
    expect(placeMenu(at(200), 480, small, 'start')).toEqual({ bottom: 104, left: 200, maxHeight: 200 - 12 });
  });

  it('keeps the horizontal alignment', () => {
    expect(placeMenu(at(100, 1300), 100, viewport, 'end')).toEqual({ top: 128, right: 1440 - 1324 });
    expect(placeMenu(at(100, 0), 100, viewport, 'start')).toEqual({ top: 128, left: 4 });
  });
});
