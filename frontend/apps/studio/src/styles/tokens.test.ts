import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const directory = `${process.cwd()}/src/styles`;
const css = readFileSync(`${directory}/base.css`, 'utf8');
const entrypoint = readFileSync(`${process.cwd()}/src/main.tsx`, 'utf8');
const manifest = JSON.parse(readFileSync(`${process.cwd()}/package.json`, 'utf8')) as { dependencies: Record<string, string> };
const distributedFontLicense = readFileSync(`${process.cwd()}/public/licenses/geist-OFL.txt`, 'utf8');
function declarations(source: string): Record<string, string> {
  const tokens: Record<string, string> = {};
  for (const match of source.matchAll(/--([\w-]+):\s*([^;]+);/g)) {
    if (match[1] && match[2]) tokens[match[1]] = match[2].trim();
  }
  return tokens;
}
const light = declarations(css.slice(css.indexOf(':root {'), css.indexOf('@media')));
const dark = { ...light, ...declarations(css.slice(css.indexOf(":root[data-theme='dark']"), css.indexOf('@media (forced-colors: active)'))) };
const forced = declarations(css.slice(css.indexOf('@media (forced-colors: active)')));
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
    expect(dark['bg-sidebar']).toBe('#06070a');
    expect(dark['bg-canvas']).toBe('#08090c');
    expect(dark.accent).toBe('#5ed99e');
  });
  it('defines the complete recorded type, spacing, radius and layout scales', () => {
    expect(light['font-sans']).toContain('Geist');
    expect(light['font-mono']).toContain('Geist Mono');
    expect(light).toMatchObject({
      'font-size-page-title': '1.25rem',
      'font-size-list-title': '0.9375rem',
      'font-size-brand': '0.875rem',
      'font-size-body': '0.8125rem',
      'font-size-mono': '0.75rem',
      'font-size-micro': '0.6875rem',
      'font-size-tagline': '0.65625rem',
      'letter-spacing-title': '-0.02em',
      'letter-spacing-label': '0.05em',
      'radius-command': '0.25rem',
      'radius-nav': '0.4375rem',
      'radius-control': '0.5rem',
      'radius-card': '0.625rem',
      'radius-panel': '0.75rem',
      'radius-pill': '999px',
      'space-1': '0.125rem',
      'space-2': '0.25rem',
      'space-3': '0.375rem',
      'space-4': '0.5rem',
      'space-5': '0.625rem',
      'space-6': '0.75rem',
      'space-7': '0.875rem',
      'space-8': '1rem',
      'space-page-y': '1.125rem',
      'space-10': '1.25rem',
      'space-page-x': '1.5rem',
      'sidebar-width': '13.5rem',
      'topbar-height': '3.5rem',
      'runs-master-width': '24.5rem',
      'graph-node-width': '11.5rem',
      'graph-node-height': '3.875rem',
      'graph-grid-size': '1.125rem',
      'graph-dot-size': '1px',
      'graph-edge-width': '1.5px',
      'graph-edge-pending-dash': '4 4',
      'graph-edge-omitted-dash': '2 5',
    });
  });
  it('maps every meaningful colour to a system colour in forced-colours mode', () => {
    for (const name of ['bg', 'bg-subtle', 'bg-raised', 'bg-sidebar', 'bg-canvas']) expect(forced[name], name).toBe('Canvas');
    for (const name of ['fg', 'fg-muted', 'border', 'border-subtle', 'graph-grid-color']) expect(forced[name], name).toBe('CanvasText');
    for (const name of ['accent', 'focus']) expect(forced[name], name).toBe('Highlight');
    expect(forced['accent-fg']).toBe('HighlightText');
    for (const name of ['danger', 'warn', 'ok', 'info', 'status-idle', 'status-running', 'status-waiting', 'status-ok', 'status-bad', 'status-skip']) {
      expect(forced[name], name).toBe('CanvasText');
    }
  });
  for (const [name, theme] of Object.entries({ light, dark })) {
    for (const background of ['bg', 'bg-subtle', 'bg-raised', 'bg-sidebar', 'bg-canvas']) {
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
  it('resolves every custom property referenced by the Studio stylesheets', () => {
    const stylesheets = readdirSync(directory).filter((name) => name.endsWith('.css'))
      .map((name) => readFileSync(`${directory}/${name}`, 'utf8')).join('\n');
    const defined = declarations(stylesheets);
    for (const match of stylesheets.matchAll(/var\((--[\w-]+)\s*\)/g)) {
      const name = match[1]?.slice(2) ?? '';
      expect(defined[name], `Undefined CSS variable --${name}`).toBeDefined();
    }
  });
  it('pins and loads the approved Geist faces from the application bundle', () => {
    expect(manifest.dependencies).toMatchObject({ '@fontsource/geist': '5.3.0', '@fontsource/geist-mono': '5.3.0' });
    for (const stylesheet of [
      '@fontsource/geist/latin-400.css',
      '@fontsource/geist/latin-500.css',
      '@fontsource/geist/latin-600.css',
      '@fontsource/geist-mono/latin-400.css',
      '@fontsource/geist-mono/latin-500.css',
    ]) {
      expect(entrypoint).toContain(`import '${stylesheet}';`);
    }
    for (const family of ['geist', 'geist-mono']) {
      const license = readFileSync(`${process.cwd()}/../../node_modules/@fontsource/${family}/LICENSE`, 'utf8');
      expect(license).toContain('SIL OPEN FONT LICENSE Version 1.1');
    }
    expect(distributedFontLicense).toContain('GeistMono-Italic[wght].ttf: Copyright 2024 The Geist Project Authors');
    expect(distributedFontLicense).toContain('SIL OPEN FONT LICENSE Version 1.1');
  });
});
