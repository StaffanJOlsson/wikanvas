// Auto-expand while dragging (docs/ARCHITECTURE.md §9.6.3): hovering a collapsed row that has children for 700 ms,
// with the drop going inside it, expands it. CollectionsTree's drag monitor feeds every drag update through
// `autoExpandKey` into an `AutoExpandTimer`; both are plain code, so they test without native drag events (W3-D14).
import type { ResolvedDrop } from '../tree/drop';
import type { Row } from '../tree/visibleRows';

/** Hovering "inside" a collapsed row for this long expands it (§9.6.3). */
export const AUTO_EXPAND_MS = 700;

/**
 * The row a drag over `row` (resolved to `resolved`) should expand after AUTO_EXPAND_MS, or null: a collapsed row with
 * children whose drop goes inside it ("child" on an item row, "collection" on a collection row).
 */
export function autoExpandKey(row: Row, resolved: ResolvedDrop | null): string | null {
  if (!resolved || !row.hasChildren || row.expanded) return null;
  const into = resolved.instruction.type === 'child' || resolved.instruction.type === 'collection';
  return into && resolved.indicator.rowKey === row.key ? row.key : null;
}

/**
 * Calls `expand(key)` once the same key has been hovered for `ms`. Hovering another key restarts the wait; null (or
 * `cancel`) stops it. A key that already fired does not fire again while it stays hovered.
 */
export class AutoExpandTimer {
  private current: { key: string; timer: ReturnType<typeof setTimeout> } | null = null;
  private readonly expand: (key: string) => void;
  private readonly ms: number;

  constructor(expand: (key: string) => void, ms: number = AUTO_EXPAND_MS) {
    this.expand = expand;
    this.ms = ms;
  }

  hover(key: string | null): void {
    if (key !== null && this.current?.key === key) return;
    this.cancel();
    if (key === null) return;
    this.current = { key, timer: setTimeout(() => this.expand(key), this.ms) };
  }

  cancel(): void {
    if (this.current) clearTimeout(this.current.timer);
    this.current = null;
  }
}
