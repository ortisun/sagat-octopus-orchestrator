import { describe, expect, it } from 'vitest';
import { clip, durationBetween, formatAbsolute, formatDuration, formatRelative, parseTime } from './time';

describe('parseTime', () => {
  it('reads a date-time and refuses what is not one', () => {
    expect(parseTime('2026-10-10T10:00:00Z')).toBe(Date.UTC(2026, 9, 10, 10));
    for (const bad of [undefined, null, '', 'yesterday', '<script>']) expect(parseTime(bad)).toBeUndefined();
  });
});

describe('formatDuration', () => {
  it('uses the unit a person reads', () => {
    expect(formatDuration(0)).toBe('0 ms');
    expect(formatDuration(850)).toBe('850 ms');
    expect(formatDuration(2500)).toBe('2.5 s');
    expect(formatDuration(12_400)).toBe('12 s');
    expect(formatDuration(185_000)).toBe('3 min 05 s');
    expect(formatDuration(2 * 3_600_000 + 10 * 60_000)).toBe('2 h 10 min');
    expect(formatDuration(27 * 3_600_000)).toBe('1 d 3 h');
  });
  it('never shows a negative or a broken number', () => {
    expect(formatDuration(-5)).toBe('0 ms');
    expect(formatDuration(Number.NaN)).toBe('unknown');
  });
});

describe('formatRelative', () => {
  const now = Date.UTC(2026, 9, 10, 10, 0, 0);
  it('says whether a moment is ahead or behind', () => {
    expect(formatRelative('2026-10-10T10:00:12Z', now)).toBe('in 12 s');
    expect(formatRelative('2026-10-10T09:55:00Z', now)).toBe('5 min ago');
    expect(formatRelative('2026-10-10T13:00:00Z', now)).toBe('in 3 h');
    expect(formatRelative('2026-10-13T10:00:00Z', now)).toBe('in 3 d');
    expect(formatRelative('2026-10-10T10:00:02Z', now)).toBe('now');
  });
  it('does not pretend to know a time it cannot read', () => {
    expect(formatRelative('garbage', now)).toBe('at an unknown time');
  });
});

describe('formatAbsolute', () => {
  it('is the same for everybody: UTC', () => {
    expect(formatAbsolute('2026-10-10T14:03:22.917Z')).toBe('2026-10-10 14:03:22 UTC');
    expect(formatAbsolute(null)).toBe('unknown time');
  });
});

describe('durationBetween', () => {
  const now = Date.UTC(2026, 9, 10, 10, 0, 30);
  it('measures a finished span and a span still going', () => {
    expect(durationBetween('2026-10-10T10:00:00Z', '2026-10-10T10:00:05Z', now)).toBe(5000);
    expect(durationBetween('2026-10-10T10:00:00Z', null, now)).toBe(30_000);
    expect(durationBetween('2026-10-10T10:00:00Z', undefined, now)).toBe(30_000);
  });
  it('is never negative when the browser clock is behind the platform, and never invents a span', () => {
    expect(durationBetween('2026-10-10T10:05:00Z', null, now)).toBe(0);
    expect(durationBetween('nope', null, now)).toBeUndefined();
    expect(durationBetween('2026-10-10T10:00:00Z', 'nope', now)).toBeUndefined();
  });
});

describe('clip', () => {
  it('clips long text', () => {
    expect(clip('abcdef', 4)).toBe('abcd…');
    expect(clip('abc', 4)).toBe('abc');
  });
});
