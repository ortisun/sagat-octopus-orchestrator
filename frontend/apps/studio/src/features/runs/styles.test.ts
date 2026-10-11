import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// (Read from disk: the test runner turns stylesheets into nothing.)
const css = readFileSync(resolve(process.cwd(), 'src/styles/runs.css'), 'utf8');

/** The parts of the stylesheet that carry a promise about accessibility or about the CSP, which no jsdom test can see. */
function block(text: string, header: string): string {
  const start = text.indexOf(header);
  if (start < 0) return '';
  let depth = 0;
  for (let i = text.indexOf('{', start); i < text.length; i++) {
    if (text[i] === '{') depth += 1;
    if (text[i] === '}') {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  return '';
}

describe('runs.css', () => {
  it('is there to be checked', () => {
    expect(css.length).toBeGreaterThan(2000);
  });

  it('switches the highlight of a changed step off for people who ask for less motion', () => {
    expect(css).toMatch(/\.rn-flash\s*{[^}]*animation:\s*rn-flash/);
    const reduced = block(css, '@media (prefers-reduced-motion: reduce)');
    expect(reduced).toMatch(/\.rn-flash\s*{[^}]*animation:\s*none/);
    expect(reduced).toMatch(/\.rn-flash\s*{[^}]*display:\s*none/);
  });

  it('draws a link that was taken, one not decided and one omitted differently by pattern, not only by colour', () => {
    expect(block(css, '.react-flow__edge.edge-taken .react-flow__edge-path')).toMatch(/stroke:\s*var\(--status-ok\)/);
    expect(block(css, '.react-flow__edge.edge-taken .react-flow__edge-path')).not.toMatch(/dasharray/);
    expect(block(css, '.react-flow__edge.edge-pending .react-flow__edge-path')).toMatch(/stroke-dasharray:\s*var\(--graph-edge-pending-dash\)/);
    const omitted = block(css, '.react-flow__edge.edge-not-taken .react-flow__edge-path');
    expect(omitted).toMatch(/stroke:\s*var\(--border\)/);
    expect(omitted).toMatch(/stroke-dasharray:\s*var\(--graph-edge-omitted-dash\)/);
    expect(omitted).toMatch(/opacity:\s*1/);
  });

  it('draws the graph on the semantic dotted canvas from the prototype', () => {
    const canvas = block(css, '.rn-canvas');
    expect(canvas).toMatch(/background-color:\s*var\(--bg-canvas\)/);
    expect(canvas).toMatch(/radial-gradient\(var\(--graph-grid-color\)/);
    expect(canvas).toMatch(/var\(--graph-dot-size\)/);
    expect(canvas).toMatch(/background-size:\s*var\(--graph-grid-size\) var\(--graph-grid-size\)/);
  });

  it('draws an undo or a lookup with a dashed border', () => {
    expect(css).toMatch(/\.rn-card\[data-variant='undo'\][^{]*{[^}]*border-style:\s*dashed/);
    expect(css).toMatch(/\.rn-card\[data-variant='lookup'\]/);
  });

  it('keeps a visible focus ring on a card and has rules for forced colours', () => {
    expect(block(css, '.react-flow__node:focus-visible > .rn-card')).toMatch(/outline:\s*3px solid var\(--focus\)/);
    const forced = block(css, '@media (forced-colors: active)');
    expect(forced).toMatch(/edge-taken/);
    expect(forced).toMatch(/Highlight/);
    expect(forced).toMatch(/\.rn-canvas[^}]*background-image:\s*none/);
  });

  it('asks for nothing outside the page: no remote URL, no import (the CSP allows the platform origin only)', () => {
    expect(css).not.toMatch(/https?:\/\//);
    expect(css).not.toMatch(/@import/);
    expect(css).not.toMatch(/url\(/);
  });

  it('keeps every target in the tables at least 24 pixels', () => {
    expect(block(css, '.link')).toMatch(/min-height:\s*24px/);
    expect(block(css, '.btn.small')).toMatch(/min-height:\s*28px/);
  });

  it('places the narrow selected-step override after its two-column base rule', () => {
    const base = css.indexOf('.rn-selected-step {');
    const narrow = css.indexOf('@media (max-width: 48rem)');
    expect(base).toBeGreaterThanOrEqual(0);
    expect(narrow).toBeGreaterThan(base);
    expect(block(css, '@media (max-width: 48rem)')).toMatch(/\.rn-selected-step\s*{[^}]*grid-template-columns:\s*minmax\(0, 1fr\)/);
  });

  it('compacts the run summary on wide review screens without changing the narrow layout', () => {
    const review = block(css, '@media (min-width: 88rem)');
    expect(review).toMatch(/\.rn-title\s*{[^}]*display:\s*grid/);
    expect(review).toMatch(/\.rn-title\s*>\s*\.toolbar\s*{[^}]*grid-row:\s*1\s*\/\s*3/);
    expect(review).toMatch(/\.rn-facts\.rn-grid\s*{[^}]*grid-template-columns:\s*repeat\(4, minmax\(0, 1fr\)\)/);
    expect(css.indexOf('@media (min-width: 88rem)')).toBeGreaterThan(css.indexOf('.rn-facts.rn-grid {'));
  });
});
