import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { describe, expect, it } from 'vitest';
import { roles, themeColor } from './roles';
import { sagatTheme } from './sagatTheme';

const root = process.cwd();
const defined = sagatTheme as unknown as { name: string; tokens: Record<string, unknown>; localTokens?: Record<string, unknown> };
const applied = JSON.stringify([defined.tokens, defined.localTokens]).toLowerCase();
const entrypoint = readFileSync(`${root}/src/main.tsx`, 'utf8');
const provider = readFileSync(`${root}/src/ui/provider.tsx`, 'utf8');
const manifest = JSON.parse(readFileSync(`${root}/package.json`, 'utf8')) as { dependencies: Record<string, string>; devDependencies: Record<string, string> };

function luminance(hex: string): number {
  expect(hex).toMatch(/^#[\da-f]{6}$/i);
  const channels = [1, 3, 5].map((offset) => {
    const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels.reduce((sum, value, index) => sum + value * ([0.2126, 0.7152, 0.0722][index] ?? 0), 0);
}
function contrast(a: string, b: string): number {
  const first = luminance(a);
  const second = luminance(b);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

type Role = keyof typeof roles;
const MODES = { light: 0, dark: 1 } as const;
const SURFACES: Role[] = ['page', 'field', 'raised', 'sidebar', 'canvas'];
const TEXT: Role[] = ['text', 'textMuted', 'accent', 'error', 'warning', 'success', 'info', 'statusIdle', 'statusSkip'];
const FILLS: Role[] = ['errorFill', 'warningFill', 'successFill', 'infoFill'];
const STRONG: Role[] = ['error', 'warning', 'success', 'info'];

describe('the sagat theme (FR-007-009, ADR-0101, D2402)', () => {
  it('keeps the dark surfaces and the accent of the recorded direction', () => {
    expect(roles.page[1]).toBe('#0a0b0e');
    expect(roles.sidebar[1]).toBe('#06070a');
    expect(roles.canvas[1]).toBe('#08090c');
    expect(roles.raised[1]).toBe('#121417');
    expect(roles.accent[1]).toBe('#5ed99e');
  });

  for (const [mode, index] of Object.entries(MODES)) {
    for (const surface of SURFACES) {
      for (const text of TEXT) {
        it(`${mode}: ${text} on ${surface} meets AA text contrast`, () => {
          expect(contrast(roles[text][index], roles[surface][index])).toBeGreaterThanOrEqual(4.5);
        });
      }
      it(`${mode}: the control boundary and the focus ring on ${surface} meet 3:1`, () => {
        expect(contrast(roles.lineControl[index], roles[surface][index])).toBeGreaterThanOrEqual(3);
        expect(contrast(roles.accent[index], roles[surface][index])).toBeGreaterThanOrEqual(3);
      });
    }
    it(`${mode}: the primary action, the banners and the strong status fills meet text contrast`, () => {
      expect(contrast(roles.onAccent[index], roles.accent[index])).toBeGreaterThanOrEqual(4.5);
      for (const fill of FILLS) expect(contrast(roles.text[index], roles[fill][index]), fill).toBeGreaterThanOrEqual(4.5);
      for (const fill of STRONG) expect(contrast(roles.onStrongFill[index], roles[fill][index]), fill).toBeGreaterThanOrEqual(4.5);
    });
  }

  it('gives a surface that cannot read a token the same colour the stylesheet has', () => {
    expect(themeColor('textMuted', 'dark')).toBe(roles.textMuted[1]);
    expect(themeColor('textMuted', 'light')).toBe(roles.textMuted[0]);
    expect(themeColor('textMuted', 'light')).toMatch(/^#[\da-f]{6}$/i);
  });

  it('maps every recorded role to a token of the theme that the application applies', () => {
    expect(defined.name).toBe('sagat');
    for (const [name, pair] of Object.entries(roles)) {
      for (const value of pair) expect(applied, `${name} ${value}`).toContain(value.toLowerCase());
    }
    expect(applied).toContain('geist');
    expect(applied).toContain('geist mono');
    expect(String(defined.tokens['--color-accent']).toLowerCase()).toBe(`light-dark(${roles.accent[0]}, ${roles.accent[1]})`);
    expect(String(defined.tokens['--color-background-surface']).toLowerCase()).toBe(`light-dark(${roles.page[0]}, ${roles.page[1]})`);
  });

  it('applies the theme from its source: no generated stylesheet or command line tool to keep in step', () => {
    expect(provider).toContain("import { sagatTheme } from './theme/sagatTheme';");
    expect(Object.keys({ ...manifest.dependencies, ...manifest.devDependencies }).filter((name) => name.startsWith('@astryxdesign/'))).toEqual(['@astryxdesign/core']);
  });

  it('loads the design system in its layer order: reset, then components', () => {
    const reset = provider.indexOf("import '@astryxdesign/core/reset.css';");
    const components = provider.indexOf("import '@astryxdesign/core/astryx.css';");
    expect(reset).toBeGreaterThanOrEqual(0);
    expect(components).toBeGreaterThan(reset);
  });

  it('pins the design system to one exact version (ADR-0101, rule 7)', () => {
    const core = manifest.dependencies['@astryxdesign/core'];
    expect(core).toMatch(/^\d+\.\d+\.\d+$/);
    expect(manifest.dependencies['@stylexjs/stylex']).toMatch(/^\d+\.\d+\.\d+$/);
    expect(Object.keys(manifest.dependencies).filter((name) => name.startsWith('@astryxdesign/theme-'))).toEqual([]);
  });

  it('pins and loads the approved Geist faces from the application bundle (D159)', () => {
    expect(manifest.dependencies).toMatchObject({ '@fontsource/geist': '5.3.0', '@fontsource/geist-mono': '5.3.0' });
    for (const stylesheet of ['@fontsource/geist/latin-400.css', '@fontsource/geist/latin-500.css', '@fontsource/geist/latin-600.css', '@fontsource/geist-mono/latin-400.css', '@fontsource/geist-mono/latin-500.css']) {
      expect(entrypoint).toContain(`import '${stylesheet}';`);
    }
    for (const family of ['geist', 'geist-mono']) {
      expect(readFileSync(createRequire(import.meta.url).resolve(`@fontsource/${family}/LICENSE`), 'utf8')).toContain('SIL OPEN FONT LICENSE Version 1.1');
    }
    const distributed = readFileSync(`${root}/public/licenses/geist-OFL.txt`, 'utf8');
    expect(distributed).toContain('GeistMono-Italic[wght].ttf: Copyright 2024 The Geist Project Authors');
    expect(distributed).toContain('SIL OPEN FONT LICENSE Version 1.1');
  });
});
