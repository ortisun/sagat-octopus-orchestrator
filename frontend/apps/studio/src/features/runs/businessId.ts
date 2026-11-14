/** The business identifier of a start (feature 018): a person's own name for a run, 1 to 200 visible ASCII characters. */

export const MAX_BUSINESS_ID = 200;

export type Conflict = 'FAIL' | 'USE_EXISTING';
export type Reuse = 'ALLOW_DUPLICATE' | 'ALLOW_DUPLICATE_FAILED_ONLY' | 'REJECT_DUPLICATE';

export const CONFLICTS: readonly { value: Conflict; label: string }[] = [
  { value: 'FAIL', label: 'Refuse the start' },
  { value: 'USE_EXISTING', label: 'Open that run instead' },
];

export const REUSES: readonly { value: Reuse; label: string }[] = [
  { value: 'ALLOW_DUPLICATE', label: 'Start a new run' },
  { value: 'ALLOW_DUPLICATE_FAILED_ONLY', label: 'Start a new run unless it succeeded' },
  { value: 'REJECT_DUPLICATE', label: 'Refuse the start' },
];

const VISIBLE_ASCII = /^[!-~]+$/;

/**
 * Why `text` cannot be an identifier, or nothing when it can (and when it is empty: a start needs none). `max` is the ceiling of
 * the field: 200 for an identifier and for the filter of the run list, 160 for the prefix of a schedule.
 */
export function businessIdProblem(text: string, max: number = MAX_BUSINESS_ID): string | undefined {
  if (text === '') return undefined;
  if (text.length > max) return `At most ${max} characters.`;
  if (!VISIBLE_ASCII.test(text)) return 'Letters, digits and punctuation only: no space, no accent, no control character.';
  return undefined;
}

/** What a start sends about its identifier (already trimmed): nothing when there is none, and the defaults are not sent. */
export function businessFields(id: string, conflict: Conflict, reuse: Reuse): Record<string, string> {
  if (id === '') return {};
  return {
    businessId: id,
    ...(conflict === 'FAIL' ? {} : { businessIdConflictPolicy: conflict }),
    ...(reuse === 'ALLOW_DUPLICATE' ? {} : { businessIdReusePolicy: reuse }),
  };
}

/** The identifier of the filter, as the address keeps it: only what could be a prefix. */
export function readBusinessPrefix(value: string | null): string {
  return value !== null && value.length <= MAX_BUSINESS_ID && VISIBLE_ASCII.test(value) ? value : '';
}
