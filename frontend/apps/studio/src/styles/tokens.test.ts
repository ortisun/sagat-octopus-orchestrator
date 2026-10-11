import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const directory = `${process.cwd()}/src/styles`;
const css = readFileSync(`${directory}/base.css`, 'utf8');
function declarations(source: string): Record<string, string> {
  const tokens: Record<string, string> = {};
  for (const match of source.matchAll(/--([\w-]+):\s*([^;]+);/g)) {
    if (match[1] && match[2]) tokens[match[1]] = match[2].trim();
  }
  return tokens;
}
const light = declarations(css.slice(css.indexOf(':root {'), css.indexOf('@media')));
const dark = { ...light, ...declarations(css.slice(css.indexOf(":root[data-theme='dark']"), css.indexOf('@media (forced-colors: active)'))) };
function luminance(hex: string): number {
  expect(hex).toMatch(/^#[\da-f]{6}$/i);
  const channels = [1, 3, 5].map((offset) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels.reduce((sum, value, index) => sum + value * ([0.2126, 0.7152, 0.0722][index] ?? 0), 0);
}
function contrast(a: string | undefined, b: string | undefined): number {
  if (!a || !b) throw new Error('Missing contrast token');
  const first = luminance(a), second = luminance(b);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

describe('design token accessibility (FR-007-009)', () => {
  it('uses the same dark tokens for system preference and explicit selection', () => {
    const system = declarations(css.slice(css.indexOf('@media (prefers-color-scheme: dark)'), css.indexOf(":root[data-theme='dark']")));
    for (const [name, value] of Object.entries(system)) expect(dark[name], name).toBe(value);
  });
  it('uses the recorded dark surfaces and accent', () => {
    expect(dark.bg).toBe('#0a0b0e');
    expect(dark.accent).toBe('#5ed99e');
  });
  for (const [name, theme] of Object.entries({ light, dark })) {
    for (const background of ['bg', 'bg-subtle', 'bg-raised']) {
      for (const foreground of ['fg', 'fg-muted', 'accent', 'danger', 'warn', 'ok', 'info', 'status-idle', 'status-running', 'status-waiting', 'status-ok', 'status-bad', 'status-skip']) {
        it(`${name}: ${foreground} on ${background} meets AA text contrast`, () => {
          expect(contrast(theme[foreground], theme[background])).toBeGreaterThanOrEqual(4.5);
        });
      }
      for (const foreground of ['border', 'focus']) {
        it(`${name}: ${foreground} on ${background} meets control contrast`, () => {
          expect(contrast(theme[foreground], theme[background])).toBeGreaterThanOrEqual(3);
        });
      }
    }
    it(`${name}: primary action and banners meet text contrast`, () => {
      expect(contrast(theme['accent-fg'], theme.accent)).toBeGreaterThanOrEqual(4.5);
      for (const tone of ['danger', 'warn', 'ok', 'info']) {
        expect(contrast(theme.fg, theme[`${tone}-bg`])).toBeGreaterThanOrEqual(4.5);
      }
    });
  }
  it('keeps literal colours in the token stylesheet', () => {
    for (const file of readdirSync(directory).filter((f) => f.endsWith('.css') && f !== 'base.css')) {
      expect(readFileSync(`${directory}/${file}`, 'utf8'), file).not.toMatch(/#[\da-f]{3,8}\b|\b(?:rgb|hsl|oklch)a?\(/i);
    }
  });
});
