import { describe, expect, it } from 'vitest';
import { formatDuration, splitDuration, toMs } from './duration';

describe('duration', () => {
  it('reads a number of milliseconds in the largest unit that holds it exactly', () => {
    expect(splitDuration(3_600_000)).toEqual({ amount: 1, unit: 'h' });
    expect(splitDuration(90_000)).toEqual({ amount: 90, unit: 's' });
    expect(splitDuration(120_000)).toEqual({ amount: 2, unit: 'min' });
    expect(splitDuration(1_500)).toEqual({ amount: 1_500, unit: 'ms' });
    expect(formatDuration(30_000)).toBe('30 s');
  });

  it('turns an amount and a unit into milliseconds, and refuses what is not a whole number of them', () => {
    expect(toMs(2, 'min')).toBe(120_000);
    expect(toMs(5, 'ms')).toBe(5);
    expect(toMs(1.5, 's')).toBe(1_500);
    expect(toMs(0.0005, 's')).toBeUndefined();
    expect(toMs(-1, 's')).toBeUndefined();
    expect(toMs(Number.NaN, 's')).toBeUndefined();
    expect(toMs(Number.MAX_SAFE_INTEGER, 'h')).toBeUndefined();
  });

  it('round-trips what it splits', () => {
    for (const ms of [1, 999, 1_000, 61_000, 3_600_000, 86_400_000, 90_061_001]) {
      const { amount, unit } = splitDuration(ms);
      expect(toMs(amount, unit)).toBe(ms);
    }
  });
});
