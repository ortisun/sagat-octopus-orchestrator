import { defineTheme } from '@astryxdesign/core/theme';
import type { TokenValue } from '@astryxdesign/core/theme';
import { roles } from './roles';

/**
 * The Studio's Astryx theme (Architecture decision 0101, Decision 2402). It carries the visual direction recorded in
 * `specs/007-workflow-studio/design/README.md`: the dark roles are the owner's export, the light
 * roles are the accessible derivation of Feature 007. Every value is `[light, dark]`.
 *
 * The colours themselves are in `roles.ts`, the only file where the Studio writes a literal colour.
 * The theme is applied as it is defined here, when the application starts: there is no generated
 * stylesheet to keep in step. `theme.test.ts` fails when a text, boundary or focus pair falls under its
 * contrast floor or when a role is not mapped.
 */
export const colorModes = ['light', 'dark'] as const;

const pair = (role: readonly [string, string]): TokenValue => [role[0], role[1]];

const sans = { family: 'Geist', fallbacks: "system-ui, -apple-system, 'Segoe UI', sans-serif" };
const mono = { family: 'Geist Mono', fallbacks: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' };

export const sagatTheme = defineTheme({
  name: 'sagat',

  // The recorded body size is 13px; the ratio gives 16, 19 and 22px for the titles.
  typography: { scale: { base: 13, ratio: 1.2 }, body: sans, heading: sans, code: mono },
  // Command 4px, control 8px, panel 12px: the recorded radii.
  radius: { base: 4, multiplier: 1 },

  // What Astryx has no role for: the Studio's own surfaces and the states of a run.
  localTokens: {
    '--sagat-color-sidebar': pair(roles.sidebar),
    '--sagat-color-canvas': pair(roles.canvas),
    '--sagat-color-grid': pair(roles.linePanel),
    '--sagat-color-minimap-mask': ['#00000026', '#00000026'],
    '--sagat-status-idle': pair(roles.statusIdle),
    '--sagat-status-running': pair(roles.info),
    '--sagat-status-waiting': pair(roles.warning),
    '--sagat-status-ok': pair(roles.success),
    '--sagat-status-bad': pair(roles.error),
    '--sagat-status-skip': pair(roles.statusSkip),
  },

  tokens: {
    '--color-background-body': pair(roles.sidebar),
    '--color-background-surface': pair(roles.page),
    '--color-background-card': pair(roles.raised),
    '--color-background-popover': pair(roles.raised),
    '--color-background-muted': pair(roles.field),
    '--color-background-inverted': [roles.text[0], roles.text[1]],

    '--color-accent': pair(roles.accent),
    '--color-accent-muted': pair(roles.successFill),
    '--color-on-accent': pair(roles.onAccent),
    '--color-text-accent': pair(roles.accent),
    '--color-icon-accent': pair(roles.accent),

    '--color-text-primary': pair(roles.text),
    '--color-text-secondary': pair(roles.textMuted),
    '--color-text-disabled': pair(roles.textDisabled),
    '--color-icon-primary': pair(roles.text),
    '--color-icon-secondary': pair(roles.textMuted),
    '--color-icon-disabled': pair(roles.textDisabled),

    '--color-border': pair(roles.linePanel),
    '--color-border-emphasized': pair(roles.lineControl),
    '--color-overlay': ['#00000080', '#00000080'],

    '--color-error': pair(roles.error),
    '--color-on-error': pair(roles.onStrongFill),
    '--color-error-muted': pair(roles.errorFill),
    '--color-on-error-muted': pair(roles.text),
    '--color-text-error': pair(roles.error),

    '--color-warning': pair(roles.warning),
    '--color-on-warning': pair(roles.onStrongFill),
    '--color-warning-muted': pair(roles.warningFill),
    '--color-on-warning-muted': pair(roles.text),
    '--color-text-warning': pair(roles.warning),

    '--color-success': pair(roles.success),
    '--color-on-success': pair(roles.onStrongFill),
    '--color-success-muted': pair(roles.successFill),
    '--color-on-success-muted': pair(roles.text),
    '--color-text-success': pair(roles.success),

    '--color-info': pair(roles.info),
    '--color-on-info': pair(roles.onStrongFill),
    '--color-info-muted': pair(roles.infoFill),
    '--color-on-info-muted': pair(roles.text),
    '--color-text-info': pair(roles.info),

    '--focus-outline-color': pair(roles.accent),
  },
});
