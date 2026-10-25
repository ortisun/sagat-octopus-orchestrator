/**
 * Text that came from a script or from the platform is shown as text and nothing else, but it can
 * still be hostile to the eye: terminal escapes that redraw the screen, control characters, and
 * characters that reorder or hide what is shown (the "trojan source" family). These helpers make a
 * log safe to put in a `<pre>` and bound how much of it is put there.
 */

import { formatLocal, parseTime } from '../runs/time';

/**
 * CSI (`ESC [ ... final`, also the one-byte form), OSC (`ESC ] ... BEL|ST`), and the two-byte
 * escapes. An escape that was cut at the end of the text (the platform cuts a log at its limit) is
 * removed too.
 */
// eslint-disable-next-line no-control-regex -- terminal escape sequences are made of control characters
const ESCAPES = /(?:\u001b\[[0-?]*[ -/]*(?:[@-~]|$)|\u009b[0-?]*[ -/]*(?:[@-~]|$)|\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\|$)|\u001b[@-Z\\-_0-?`-~])/g;

/** C0 controls other than tab and newline, DEL and C1 controls. */
// eslint-disable-next-line no-control-regex -- removing control characters is the point
const CONTROLS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g;

/** Characters that reorder text or are invisible: shown as `<U+202E>` instead of acting. */
const INVISIBLE_RANGES: readonly (readonly [number, number])[] = [
  [0x061c, 0x061c],
  [0x200b, 0x200f],
  [0x202a, 0x202e],
  [0x2060, 0x2064],
  [0x2066, 0x2069],
  [0xfeff, 0xfeff],
];
const INVISIBLE = new RegExp(`[${INVISIBLE_RANGES.map(([from, to]) => `${String.fromCharCode(from)}-${String.fromCharCode(to)}`).join('')}]`, 'g');

export function stripAnsi(text: string): string {
  return text.replace(ESCAPES, '');
}

/** Makes the characters that hide or reorder text visible, by name. */
export function revealInvisible(text: string): string {
  return text.replace(INVISIBLE, (char) => `<U+${char.charCodeAt(0).toString(16).toUpperCase().padStart(4, '0')}>`);
}

/**
 * A log or a result as it may be shown: line ends normalised (a carriage return alone, as a
 * progress bar writes it, starts a new line), terminal escapes and control characters removed,
 * invisible and reordering characters made visible.
 */
export function sanitizeLog(text: string): string {
  return revealInvisible(stripAnsi(text.replace(/\r\n?/g, '\n')).replace(CONTROLS, ''));
}

export interface ClipLimits {
  maxChars: number;
  maxLines: number;
  maxLineChars: number;
}

export interface Clipped {
  text: string;
  /** Whether anything was left out. */
  clipped: boolean;
  totalChars: number;
  totalLines: number;
}

/** Keeps the start of `text` within the limits, line by line, and says whether it had to cut. */
export function clipText(text: string, limits: ClipLimits): Clipped {
  const lines = text.split('\n');
  let clipped = false;
  const kept: string[] = [];
  let chars = 0;
  for (const line of lines) {
    if (kept.length >= limits.maxLines) {
      clipped = true;
      break;
    }
    let shown = line;
    if (shown.length > limits.maxLineChars) {
      // Not in the middle of a surrogate pair.
      let end = limits.maxLineChars;
      if (isHighSurrogate(shown.charCodeAt(end - 1))) end -= 1;
      shown = `${shown.slice(0, end)} … [${line.length - end} more characters on this line]`;
      clipped = true;
    }
    if (chars + shown.length + 1 > limits.maxChars) {
      const room = Math.max(0, limits.maxChars - chars);
      let end = room;
      if (end > 0 && isHighSurrogate(shown.charCodeAt(end - 1))) end -= 1;
      kept.push(shown.slice(0, end));
      clipped = true;
      break;
    }
    kept.push(shown);
    chars += shown.length + 1;
  }
  return { text: kept.join('\n'), clipped, totalChars: text.length, totalLines: lines.length };
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

/** How many bytes `text` is in UTF-8, without making the bytes. */
export function utf8Length(text: string): number {
  let bytes = 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code < 0x80) bytes += 1;
    else if (code < 0x800) bytes += 2;
    else if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length && isLowSurrogate(text.charCodeAt(i + 1))) {
      bytes += 4;
      i += 1;
    } else bytes += 3;
  }
  return bytes;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${trim(bytes / 1024)} KiB`;
  return `${trim(bytes / (1024 * 1024))} MiB`;
}

function trim(value: number): string {
  return value >= 100 ? value.toFixed(0) : value.toFixed(1).replace(/\.0$/, '');
}

export function formatDuration(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms) || ms < 0) return '—';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${trim(ms / 1000)} s`;
  return `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function isUuid(value: string | undefined): value is string {
  return value !== undefined && UUID.test(value);
}

/** The start of a digest, for a table; the whole digest is always one click away. */
export function shortDigest(digest: string): string {
  const [algorithm, hex] = digest.split(':');
  return hex === undefined ? digest.slice(0, 20) : `${algorithm}:${hex.slice(0, 12)}…`;
}

/** A person's name or an id as it may appear in a table: clipped, never trusted for its length. */
export function clipLabel(text: string, max = 60): string {
  const safe = revealInvisible(text.replace(CONTROLS, ''));
  return safe.length > max ? `${safe.slice(0, max)}…` : safe;
}

/** A time from the platform for people: local, in the one format the whole Studio uses. An unreadable one is shown as unknown, not as an error. */
export function formatTime(iso: string | undefined): string {
  return parseTime(iso) === undefined ? '—' : formatLocal(iso);
}
