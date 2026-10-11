/**
 * The parts of the Monaco setup that are plain functions, so they are tested without a browser: which
 * worker serves which language, what the editor's environment is, which theme matches the page.
 * Nothing here imports Monaco; `monaco.ts` does, and is loaded on demand.
 */

/** Monaco asks for one worker per language service; the Studio bundles three. */
export type WorkerKind = 'editor' | 'json' | 'typescript';

/**
 * The worker for a Monaco `label`. Only JSON and TypeScript/JavaScript have language services of
 * their own; every other label (the editor's own `editorWorkerService`, anything unknown) gets the
 * generic editor worker, which is always safe.
 */
export function workerKind(label: string): WorkerKind {
  if (label === 'json') return 'json';
  if (label === 'typescript' || label === 'javascript') return 'typescript';
  return 'editor';
}

export type WorkerFactories = Record<WorkerKind, new () => Worker>;

export interface MonacoEnvironmentShape {
  getWorker: (workerId: string, label: string) => Worker;
}

/**
 * The environment Monaco reads from `self.MonacoEnvironment`: workers come from the bundle (Vite's
 * `?worker` constructors, which load from the page's own origin or a `blob:`, both allowed by the
 * CSP) and never from a URL Monaco is told about, so there is no `getWorkerUrl` and no CDN.
 */
export function monacoEnvironment(factories: WorkerFactories): MonacoEnvironmentShape {
  return {
    getWorker: (_workerId, label) => {
      const Factory = factories[workerKind(label)];
      return new Factory();
    },
  };
}

/** Installs the environment once; a second call leaves the first in place. */
export function installMonacoEnvironment(target: { MonacoEnvironment?: unknown }, factories: WorkerFactories): MonacoEnvironmentShape {
  const existing = target.MonacoEnvironment as MonacoEnvironmentShape | undefined;
  if (existing !== undefined && typeof existing.getWorker === 'function') return existing;
  const environment = monacoEnvironment(factories);
  target.MonacoEnvironment = environment;
  return environment;
}

export type MonacoTheme = 'vs' | 'vs-dark' | 'hc-black' | 'hc-light';

export type StudioMonacoTheme = 'sagat-light' | 'sagat-dark' | 'hc-black' | 'hc-light';

export interface StudioMonacoThemeData {
  base: 'vs' | 'vs-dark';
  inherit: true;
  rules: { token: 'comment'; foreground: string }[];
  colors: Record<string, string>;
}

/** The theme that matches the page: the explicit choice (`data-theme`) wins over the system's. */
export function resolveTheme(dataTheme: string | null | undefined, prefersDark: boolean, forcedColors = false): MonacoTheme {
  const dark = dataTheme === 'dark' || (dataTheme !== 'light' && prefersDark);
  if (forcedColors) return dark ? 'hc-black' : 'hc-light';
  return dark ? 'vs-dark' : 'vs';
}

/** The Studio only customizes normal themes; forced-colors mode keeps Monaco's high-contrast themes. */
export function studioThemeName(base: MonacoTheme): StudioMonacoTheme {
  if (base === 'vs-dark') return 'sagat-dark';
  if (base === 'vs') return 'sagat-light';
  return base;
}

/**
 * Monaco token theme colors are six-digit hex without `#`. The single override uses the page's
 * semantic muted-text token, whose contrast is checked with the rest of the Studio palette.
 */
export function studioThemeData(base: 'vs' | 'vs-dark', mutedText: string): StudioMonacoThemeData {
  const match = /^#([\da-f]{6})$/i.exec(mutedText.trim());
  if (match?.[1] === undefined) throw new Error('The Monaco comment token must be a six-digit hex color');
  return {
    base,
    inherit: true,
    rules: [{ token: 'comment', foreground: match[1].toUpperCase() }],
    colors: {},
  };
}

/** The kind of model, which decides its language service and where it lives. */
export type ModelRole = 'source' | 'inputSchema' | 'outputSchema' | 'testInput' | 'version' | 'versionInputSchema' | 'versionOutputSchema';

const EXTENSION: Record<string, string> = { typescript: 'ts', python: 'py', json: 'json', csharp: 'cs', go: 'go', java: 'java' };

/**
 * The address of a model. It is in memory (nothing is fetched from it), fixed per role so a
 * schema can be associated with it, and TypeScript sees the source as `file:///main.ts`, a module.
 */
export function modelAddress(role: ModelRole, language: string): string {
  const extension = EXTENSION[language] ?? 'txt';
  if (role === 'source' && language === 'typescript') return 'file:///main.ts';
  return `inmemory://sagat/${role}.${extension}`;
}

/** A severity as the problems list says it. */
export type ProblemSeverity = 'error' | 'warning' | 'info';

export interface Problem {
  severity: ProblemSeverity;
  line: number;
  column: number;
  message: string;
  /** Which language service said it (`ts`, `json`), when it says. */
  source?: string | undefined;
}

/** Monaco's numeric severities: 8 error, 4 warning, 2 info, 1 hint. */
export function severityOf(monacoSeverity: number): ProblemSeverity {
  if (monacoSeverity >= 8) return 'error';
  if (monacoSeverity >= 4) return 'warning';
  return 'info';
}

const MAX_PROBLEMS = 50;
const MAX_MESSAGE = 300;

/** Problems as the page lists them: worst first, then by position, bounded, messages clipped. */
export function tidyProblems(problems: readonly Problem[]): Problem[] {
  const rank: Record<ProblemSeverity, number> = { error: 0, warning: 1, info: 2 };
  return [...problems]
    .map((problem) => ({ ...problem, message: problem.message.length > MAX_MESSAGE ? `${problem.message.slice(0, MAX_MESSAGE)}…` : problem.message }))
    .sort((a, b) => rank[a.severity] - rank[b.severity] || a.line - b.line || a.column - b.column)
    .slice(0, MAX_PROBLEMS);
}
