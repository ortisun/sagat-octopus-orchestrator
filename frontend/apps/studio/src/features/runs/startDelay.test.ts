import { describe, expect, it } from 'vitest';
import { MAX_START_DELAY_MS, startDelayFields, startDelayProblem } from './startDelay';

describe('the delay of a start', () => {
  it('needs none: an empty amount sends nothing', () => {
    expect(startDelayProblem('', 'hours')).toBeUndefined();
    expect(startDelayFields('', 'hours')).toEqual({});
    expect(startDelayFields('   ', 'days')).toEqual({});
  });

  it('is a whole number of minutes, hours or days, in milliseconds', () => {
    expect(startDelayFields('90', 'minutes')).toEqual({ startDelayMs: 5_400_000 });
    expect(startDelayFields('2', 'hours')).toEqual({ startDelayMs: 7_200_000 });
    expect(startDelayFields(' 2 ', 'days')).toEqual({ startDelayMs: 172_800_000 });
    expect(startDelayProblem('2', 'days')).toBeUndefined();
  });

  it('is at most 366 days, and at least a minute', () => {
    expect(MAX_START_DELAY_MS).toBe(366 * 86_400_000);
    expect(startDelayProblem('366', 'days')).toBeUndefined();
    expect(startDelayProblem('367', 'days')).toMatch(/at most 366 days/);
    expect(startDelayProblem('9000000', 'hours')).toMatch(/at most 366 days/);
    expect(startDelayProblem('0', 'minutes')).toMatch(/whole number/);
  });

  it('refuses what is not a whole number, in words', () => {
    for (const text of ['-1', '1.5', '1e3', 'soon', '0x10', '١٢']) {
      expect(startDelayProblem(text, 'hours'), text).toMatch(/whole number/);
    }
  });
});
