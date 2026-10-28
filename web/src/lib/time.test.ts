import { describe, expect, it } from 'vitest';
import { relativeTime } from './time';

describe('relativeTime', () => {
  const now = new Date(2026, 9, 28, 15, 0, 0).getTime();
  const ago = (ms: number) => new Date(now - ms).toISOString();

  it('follows the §9.6.4 steps', () => {
    expect(relativeTime(ago(10_000), now)).toBe('just now');
    expect(relativeTime(ago(60_000), now)).toBe('1 minute ago');
    expect(relativeTime(ago(59 * 60_000), now)).toBe('59 minutes ago');
    expect(relativeTime(ago(3 * 3_600_000), now)).toBe('3 hours ago');
    expect(relativeTime(ago(30 * 3_600_000), now)).toBe('yesterday');
    expect(relativeTime(new Date(2026, 9, 3, 12).getTime(), now)).toBe('Oct 3');
    expect(relativeTime(new Date(2025, 0, 5, 12).getTime(), now)).toBe('Jan 5, 2025');
  });

  it('treats a time in the future as just now', () => {
    expect(relativeTime(now + 5_000, now)).toBe('just now');
  });
});
