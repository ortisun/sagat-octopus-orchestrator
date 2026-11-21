/**
 * The colour roles of the Studio's theme (Architecture decision 0101, Decision 2402), each as `[light, dark]`. The dark values are
 * the owner's export recorded in `specs/007-workflow-studio/design/README.md`; the light values are the
 * accessible derivation of Feature 007. This is the only file where the Studio writes a literal colour:
 * `sagatTheme.ts` maps the roles to the design system's tokens, and `theme.test.ts` measures them.
 */
export const roles = {
  page: ['#ffffff', '#0a0b0e'],
  field: ['#f4f5f7', '#0e0f13'],
  raised: ['#ffffff', '#121417'],
  sidebar: ['#eef1f4', '#06070a'],
  canvas: ['#f4f5f7', '#08090c'],
  text: ['#1b1f24', '#eceff2'],
  textMuted: ['#4a5360', '#8f9298'],
  textDisabled: ['#78818d', '#686b70'],
  lineControl: ['#78818d', '#686b70'],
  linePanel: ['#d4d8de', '#1d1f23'],
  accent: ['#12643f', '#5ed99e'],
  onAccent: ['#ffffff', '#05160d'],
  error: ['#b3261e', '#f75d59'],
  errorFill: ['#fdecea', '#291315'],
  warning: ['#8a4b00', '#edb345'],
  warningFill: ['#fff4e0', '#252015'],
  success: ['#1b6b2f', '#52cd86'],
  successFill: ['#e7f5ea', '#14251c'],
  info: ['#0b57d0', '#56acf0'],
  infoFill: ['#e8f0fe', '#14212c'],
  onStrongFill: ['#ffffff', '#0a0b0e'],
  statusIdle: ['#4a5360', '#83868c'],
  statusSkip: ['#626975', '#83868c'],
} as const satisfies Record<string, readonly [string, string]>;


export type ColorRole = keyof typeof roles;

/**
 * The colour of a role as a six-digit hexadecimal value, for a surface that cannot read a stylesheet
 * token (the code editor's own theme takes literal colours).
 */
export function themeColor(role: ColorRole, mode: 'light' | 'dark'): string {
  return roles[role][mode === 'dark' ? 1 : 0];
}
