// Relative times of the item meta line (docs/ARCHITECTURE.md §9.6.4).
import type { Timestamp } from '../api/gen/Timestamp';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

/**
 * "just now" (under a minute), "N minutes ago", "N hours ago" (same or previous day, under 24 hours), "yesterday",
 * then a date ("Oct 3", with the year when it is not the current one).
 */
export function relativeTime(at: Timestamp | Date | number, now: Date | number = Date.now()): string {
  const t = at instanceof Date ? at.getTime() : typeof at === 'number' ? at : Date.parse(at);
  const n = now instanceof Date ? now.getTime() : now;
  const diff = n - t;
  if (Number.isNaN(t)) return '';
  if (diff < MINUTE) return 'just now';
  if (diff < HOUR) {
    const m = Math.floor(diff / MINUTE);
    return m === 1 ? '1 minute ago' : `${m} minutes ago`;
  }
  if (diff < 24 * HOUR) {
    const h = Math.floor(diff / HOUR);
    return h === 1 ? '1 hour ago' : `${h} hours ago`;
  }
  const day = new Date(n);
  day.setHours(0, 0, 0, 0);
  const yesterday = day.getTime() - 24 * HOUR;
  if (t >= yesterday) return 'yesterday';
  const date = new Date(t);
  const sameYear = date.getFullYear() === new Date(n).getFullYear();
  return date.toLocaleDateString('en', sameYear
    ? { month: 'short', day: 'numeric' }
    : { year: 'numeric', month: 'short', day: 'numeric' });
}
