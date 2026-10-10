/** Times of a run, told to a person. Everything here is a pure function of its arguments: no clock is read. */

/** Milliseconds since the epoch for an ISO date-time the platform sent, or nothing when it is not one. */
export function parseTime(value: string | null | undefined): number | undefined {
  if (value === null || value === undefined || value === '') return undefined;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : undefined;
}

/** `2026-10-10 14:03:22 UTC`: unambiguous for people in different time zones, the text of a tooltip. */
export function formatAbsolute(value: string | null | undefined): string {
  const ms = parseTime(value);
  if (ms === undefined) return 'unknown time';
  return `${new Date(ms).toISOString().slice(0, 19).replace('T', ' ')} UTC`;
}

const LOCAL = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium' });

/** The time in the reader's own zone. */
export function formatLocal(value: string | null | undefined): string {
  const ms = parseTime(value);
  return ms === undefined ? 'unknown time' : LOCAL.format(ms);
}

/** `850 ms`, `2.5 s`, `12 s`, `3 min 05 s`, `2 h 10 min`, `1 d 3 h`. */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms)) return 'unknown';
  const total = Math.max(0, Math.round(ms));
  if (total < 1000) return `${total} ms`;
  const seconds = total / 1000;
  if (seconds < 10) return `${seconds.toFixed(1)} s`;
  const whole = Math.floor(seconds);
  if (whole < 60) return `${whole} s`;
  const minutes = Math.floor(whole / 60);
  if (minutes < 60) return `${minutes} min ${String(whole % 60).padStart(2, '0')} s`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ${String(minutes % 60).padStart(2, '0')} min`;
  return `${Math.floor(hours / 24)} d ${hours % 24} h`;
}

/** `in 12 s`, `5 min ago`, `now`: a moment relative to `nowMs`. */
export function formatRelative(value: string | null | undefined, nowMs: number): string {
  const at = parseTime(value);
  if (at === undefined) return 'at an unknown time';
  const diff = at - nowMs;
  const abs = Math.abs(diff);
  if (abs < 5000) return 'now';
  const text = unit(abs);
  return diff > 0 ? `in ${text}` : `${text} ago`;
}

function unit(abs: number): string {
  const seconds = Math.floor(abs / 1000);
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h`;
  return `${Math.floor(hours / 24)} d`;
}

/**
 * How long something lasted: from `start` to `end`, or to `nowMs` while it has no end. Nothing for
 * a time that cannot be read, and never negative (the browser's clock may be behind the server's).
 */
export function durationBetween(start: string | null | undefined, end: string | null | undefined, nowMs: number): number | undefined {
  const from = parseTime(start);
  if (from === undefined) return undefined;
  const to = end === null || end === undefined ? nowMs : parseTime(end);
  if (to === undefined) return undefined;
  return Math.max(0, to - from);
}

/** A string clipped to `max` characters with an ellipsis, for text the platform or a user wrote. */
export function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}
