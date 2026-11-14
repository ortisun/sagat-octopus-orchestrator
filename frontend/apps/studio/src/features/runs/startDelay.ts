/** The delay of a start (feature 018): a run that is created now and releases its first step later. */

export type DelayUnit = 'minutes' | 'hours' | 'days';

export const DELAY_UNITS: readonly { value: DelayUnit; label: string; ms: number }[] = [
  { value: 'minutes', label: 'minutes', ms: 60_000 },
  { value: 'hours', label: 'hours', ms: 3_600_000 },
  { value: 'days', label: 'days', ms: 86_400_000 },
];

/** The longest delay the platform takes: 366 days, in milliseconds. */
export const MAX_START_DELAY_MS = 366 * 86_400_000;

const WHOLE = /^[0-9]{1,9}$/;

/** What a delay amount comes to in milliseconds, or why it cannot be one; an empty amount is no delay. */
function parse(amount: string, unit: DelayUnit): { ms: number } | { problem: string } | undefined {
  const text = amount.trim();
  if (text === '') return undefined;
  const per = DELAY_UNITS.find((u) => u.value === unit)?.ms ?? 60_000;
  if (!WHOLE.test(text) || Number(text) < 1) return { problem: `Enter a whole number of ${unit}, at least 1.` };
  const ms = Number(text) * per;
  if (ms > MAX_START_DELAY_MS) return { problem: 'The delay is at most 366 days.' };
  return { ms };
}

/** Why the amount cannot be a delay, in words; nothing for none and for a good one. */
export function startDelayProblem(amount: string, unit: DelayUnit): string | undefined {
  const parsed = parse(amount, unit);
  return parsed !== undefined && 'problem' in parsed ? parsed.problem : undefined;
}

/** What a start sends about its delay: nothing without one. */
export function startDelayFields(amount: string, unit: DelayUnit): { startDelayMs?: number } {
  const parsed = parse(amount, unit);
  return parsed !== undefined && 'ms' in parsed ? { startDelayMs: parsed.ms } : {};
}

/** "1 hour", "90 minutes": the delay as a person says it. */
export function describeDelay(amount: string, unit: DelayUnit): string {
  const count = Number(amount.trim());
  return `${count} ${count === 1 ? unit.replace(/s$/, '') : unit}`;
}
