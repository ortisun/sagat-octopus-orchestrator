import { describe, expect, it } from 'vitest';
import { lineDiff } from './diff';

describe('lineDiff', () => {
  it('finds nothing when the texts are the same', () => {
    const out = lineDiff('a\nb\nc', 'a\nb\nc');
    expect(out).toMatchObject({ added: 0, removed: 0, exact: true, lines: [] });
  });

  it('tells a changed line from the lines around it', () => {
    const out = lineDiff('one\ntwo\nthree', 'one\n2\nthree');
    expect(out.removed).toBe(1);
    expect(out.added).toBe(1);
    expect(out.lines.map((line) => (line.kind === 'gap' ? 'gap' : `${line.kind}:${line.text}`))).toEqual(['same:one', 'removed:two', 'added:2', 'same:three']);
  });

  it('finds lines added and removed in the middle of a longer text', () => {
    const before = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j'].join('\n');
    const after = ['a', 'b', 'c', 'X', 'd', 'e', 'f', 'g', 'h', 'j'].join('\n');
    const out = lineDiff(before, after);
    expect(out.added).toBe(1);
    expect(out.removed).toBe(1);
    expect(out.lines.filter((line) => line.kind === 'added').map((line) => (line.kind === 'added' ? line.text : ''))).toEqual(['X']);
    expect(out.lines.filter((line) => line.kind === 'removed').map((line) => (line.kind === 'removed' ? line.text : ''))).toEqual(['i']);
  });

  it('keeps a little context around a change and hides the rest behind a gap', () => {
    const lines = Array.from({ length: 40 }, (_, i) => `line ${i}`);
    const changed = [...lines];
    changed[20] = 'CHANGED';
    const out = lineDiff(lines.join('\n'), changed.join('\n'));
    const shown = out.lines.filter((line) => line.kind === 'same').length;
    expect(shown).toBe(6);
    expect(out.lines[0]).toEqual({ kind: 'gap', hidden: 17 });
    expect(out.lines[out.lines.length - 1]).toEqual({ kind: 'gap', hidden: 16 });
    // A change at the far end leaves a gap behind.
    const edge = [...lines];
    edge[39] = 'END';
    const far = lineDiff(lines.join('\n'), edge.join('\n'));
    expect(far.lines[0]).toEqual({ kind: 'gap', hidden: 36 });
  });

  it('compares from nothing and to nothing', () => {
    expect(lineDiff('', 'a\nb')).toMatchObject({ added: 2, removed: 0 });
    expect(lineDiff('a\nb', '')).toMatchObject({ added: 0, removed: 2 });
  });

  it('does not build a huge table for two very different texts: it says they were replaced', () => {
    const left = Array.from({ length: 1500 }, (_, i) => `left ${i}`).join('\n');
    const right = Array.from({ length: 1500 }, (_, i) => `right ${i}`).join('\n');
    const started = performance.now();
    const out = lineDiff(left, right);
    expect(performance.now() - started).toBeLessThan(1000);
    expect(out.exact).toBe(false);
    expect(out.removed).toBe(1500);
    expect(out.added).toBe(1500);
    // What is shown is bounded whatever the size.
    expect(out.lines.length).toBeLessThanOrEqual(400);
  });
});
