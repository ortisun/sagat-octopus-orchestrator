/** Durations are stored in milliseconds; people think in seconds, minutes and hours. */
export type DurationUnit = 'ms' | 's' | 'min' | 'h';

export const DURATION_UNITS: readonly { unit: DurationUnit; ms: number }[] = [
  { unit: 'ms', ms: 1 },
  { unit: 's', ms: 1_000 },
  { unit: 'min', ms: 60_000 },
  { unit: 'h', ms: 3_600_000 },
];

export function unitFactor(unit: DurationUnit): number {
  return DURATION_UNITS.find((u) => u.unit === unit)?.ms ?? 1;
}

/** The largest unit that divides `ms` evenly, so 90000 reads "90 s" and 3600000 reads "1 h". */
export function splitDuration(ms: number): { amount: number; unit: DurationUnit } {
  if (!Number.isSafeInteger(ms) || ms <= 0) return { amount: Number.isFinite(ms) ? ms : 0, unit: 'ms' };
  for (let i = DURATION_UNITS.length - 1; i >= 0; i--) {
    const entry = DURATION_UNITS[i];
    if (entry && ms % entry.ms === 0) return { amount: ms / entry.ms, unit: entry.unit };
  }
  return { amount: ms, unit: 'ms' };
}

/** Milliseconds for an amount in a unit, or nothing if it is not a whole, non-negative number of them. */
export function toMs(amount: number, unit: DurationUnit): number | undefined {
  if (!Number.isFinite(amount) || amount < 0) return undefined;
  const ms = amount * unitFactor(unit);
  return Number.isSafeInteger(ms) ? ms : undefined;
}

export function formatDuration(ms: number): string {
  const { amount, unit } = splitDuration(ms);
  return `${amount} ${unit}`;
}
