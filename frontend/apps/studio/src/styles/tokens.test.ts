import { readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { describe, expect, it } from 'vitest';
import { sagatTheme } from '../ui/theme/sagatTheme';

const root = process.cwd();
const directory = `${root}/src/styles`;
const files = readdirSync(directory).filter((name) => name.endsWith('.css'));
const read = (name: string) => readFileSync(`${directory}/${name}`, 'utf8');
const studio = files.map(read).join('\n');
const designSystem = readFileSync(createRequire(import.meta.url).resolve('@astryxdesign/core/astryx.css'), 'utf8');

function declared(source: string): Set<string> {
  return new Set([...source.matchAll(/(--[\w-]+)\s*:/g)].map((match) => match[1] ?? ''));
}

describe('the Studio stylesheets (Success criterion 007-005, Architecture decision 0101 rule 5)', () => {
  it('hold no literal colour: every colour is a token of the theme', () => {
    for (const file of files) {
      expect(read(file), file).not.toMatch(/#[\da-f]{3,8}\b|\b(?:rgb|hsl|oklch|oklab|lab|lch|color-mix)a?\(/i);
    }
  });

  it('reference only custom properties that the theme, the design system or the Studio declares', () => {
    const theme = sagatTheme as unknown as { tokens: Record<string, unknown>; localTokens?: Record<string, unknown> };
    const known = new Set([...declared(studio), ...declared(designSystem), ...Object.keys(theme.tokens), ...Object.keys(theme.localTokens ?? {})]);
    for (const file of files) {
      for (const match of read(file).matchAll(/var\((--[\w-]+)/g)) {
        expect(known.has(match[1] ?? ''), `${file}: undefined custom property ${match[1]}`).toBe(true);
      }
    }
  });

  it('declare no colour, type, spacing or radius token of their own', () => {
    // What the Studio declares is layout only; a parallel token system is what Architecture decision 0101 removes.
    for (const name of declared(studio)) {
      expect(name, name).not.toMatch(/^--(color|font|text|spacing|radius|shadow|focus|size-element)-/);
    }
  });

  it('do not style a bare control, heading, table or link: those belong to the design system', () => {
    for (const file of files) {
      const selectors = [...read(file).matchAll(/(^|\})\s*([^{}@]+)\{/g)].map((match) => match[2] ?? '');
      for (const selector of selectors) {
        for (const part of selector.split(',')) {
          expect(part.trim(), `${file}: ${part.trim()}`).not.toMatch(/^(button|input|select|textarea|table|a|h[1-6])(\s*$|:)/);
        }
      }
    }
  });
});
