/**
 * A line diff for the one place the Studio needs it: two people changed the same draft, and the
 * author must see what differs before anything is overwritten (D146). Small and bounded on
 * purpose: common start and end are skipped, the middle is compared line by line while that stays
 * cheap, and past that it is shown as "everything here was replaced".
 */
export type DiffLine = { kind: 'same' | 'removed' | 'added'; text: string } | { kind: 'gap'; hidden: number };

export interface LineDiff {
  lines: DiffLine[];
  added: number;
  removed: number;
  /** Whether the middle was compared line by line (false: shown as one block replaced by another). */
  exact: boolean;
}

/** The most cells of the comparison table we are willing to fill. */
const MAX_CELLS = 1_000_000;
const CONTEXT = 3;
/** The most lines a diff shows; the rest is summarised by the counts. */
const MAX_SHOWN = 400;

export function lineDiff(before: string, after: string): LineDiff {
  const a = before === '' ? [] : before.split('\n');
  const b = after === '' ? [] : after.split('\n');
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start += 1;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA -= 1;
    endB -= 1;
  }
  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  const { ops, exact } = compare(midA, midB);
  const all: DiffLine[] = [
    ...a.slice(0, start).map((text): DiffLine => ({ kind: 'same', text })),
    ...ops,
    ...a.slice(endA).map((text): DiffLine => ({ kind: 'same', text })),
  ];
  const added = ops.filter((op) => op.kind === 'added').length;
  const removed = ops.filter((op) => op.kind === 'removed').length;
  return { lines: withContext(all).slice(0, MAX_SHOWN), added, removed, exact };
}

function compare(a: string[], b: string[]): { ops: DiffLine[]; exact: boolean } {
  if (a.length === 0) return { ops: b.map((text) => ({ kind: 'added', text })), exact: true };
  if (b.length === 0) return { ops: a.map((text) => ({ kind: 'removed', text })), exact: true };
  if (a.length * b.length > MAX_CELLS) {
    return { ops: [...a.map((text): DiffLine => ({ kind: 'removed', text })), ...b.map((text): DiffLine => ({ kind: 'added', text }))], exact: false };
  }
  // Longest common subsequence of lines, by length tables over suffixes.
  const width = b.length + 1;
  const table = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      table[i * width + j] = a[i] === b[j] ? (table[(i + 1) * width + j + 1] ?? 0) + 1 : Math.max(table[(i + 1) * width + j] ?? 0, table[i * width + j + 1] ?? 0);
    }
  }
  const ops: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      ops.push({ kind: 'same', text: a[i] ?? '' });
      i += 1;
      j += 1;
    } else if ((table[(i + 1) * width + j] ?? 0) >= (table[i * width + j + 1] ?? 0)) {
      ops.push({ kind: 'removed', text: a[i] ?? '' });
      i += 1;
    } else {
      ops.push({ kind: 'added', text: b[j] ?? '' });
      j += 1;
    }
  }
  for (; i < a.length; i += 1) ops.push({ kind: 'removed', text: a[i] ?? '' });
  for (; j < b.length; j += 1) ops.push({ kind: 'added', text: b[j] ?? '' });
  return { ops, exact: true };
}

/** Keeps a few unchanged lines around each change and replaces the rest by a gap. */
function withContext(lines: DiffLine[]): DiffLine[] {
  const keep = new Array<boolean>(lines.length).fill(false);
  lines.forEach((line, index) => {
    if (line.kind === 'same' || line.kind === 'gap') return;
    for (let k = Math.max(0, index - CONTEXT); k <= Math.min(lines.length - 1, index + CONTEXT); k += 1) keep[k] = true;
  });
  const out: DiffLine[] = [];
  let hidden = 0;
  lines.forEach((line, index) => {
    if (keep[index]) {
      if (hidden > 0) out.push({ kind: 'gap', hidden });
      hidden = 0;
      out.push(line);
    } else hidden += 1;
  });
  if (hidden > 0 && out.length > 0) out.push({ kind: 'gap', hidden });
  return out;
}
