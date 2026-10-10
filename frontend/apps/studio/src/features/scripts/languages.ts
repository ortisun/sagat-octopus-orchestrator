/**
 * The languages a script can be written in. The workflow schema names five; the platform builds and
 * runs two (G96 in `specs/005-inline-code-sandbox/research.md`): the others are refused when a
 * script is written, with `details.reason` `LANGUAGE_NOT_BUILT`. The Studio shows them, disabled
 * and saying so, instead of hiding what the platform will one day do or pretending it does it now.
 */
export type ScriptLanguage = 'typescript' | 'python' | 'csharp' | 'go' | 'java';

export interface LanguageInfo {
  id: string;
  label: string;
  /** Whether the platform builds and runs it today. */
  built: boolean;
  /** The Monaco language of the source. */
  monaco: 'typescript' | 'python' | 'csharp' | 'go' | 'java' | 'plaintext';
  /** What the browser can check while the author types. */
  checks: string;
  /** Whether the browser has a language service that reports problems in it. */
  lint: boolean;
}

export const LANGUAGES: readonly LanguageInfo[] = [
  {
    id: 'typescript',
    label: 'TypeScript',
    built: true,
    monaco: 'typescript',
    lint: true,
    checks: 'Syntax and types are checked in the browser as advice only: the platform erases the types and checks that the source parses and exports a default function when it builds.',
  },
  {
    id: 'python',
    label: 'Python',
    built: true,
    monaco: 'python',
    lint: false,
    checks: 'Syntax highlighting only; the platform checks the source when it builds.',
  },
  { id: 'csharp', label: 'C#', built: false, monaco: 'csharp', lint: false, checks: 'Syntax highlighting only; the platform checks the source when it builds.' },
  { id: 'go', label: 'Go', built: false, monaco: 'go', lint: false, checks: 'Syntax highlighting only; the platform checks the source when it builds.' },
  { id: 'java', label: 'Java', built: false, monaco: 'java', lint: false, checks: 'Syntax highlighting only; the platform checks the source when it builds.' },
];

export const NOT_BUILT = 'not built by this platform yet';

export function languageInfo(id: string): LanguageInfo {
  return (
    LANGUAGES.find((language) => language.id === id) ?? {
      id,
      label: id.length > 24 ? `${id.slice(0, 24)}…` : id,
      built: false,
      monaco: 'plaintext',
      lint: false,
      checks: 'Syntax highlighting only; the platform checks the source when it builds.',
    }
  );
}

/** The Monaco language id for the source of a script (unknown languages are shown as plain text). */
export function monacoLanguage(id: string): string {
  return languageInfo(id).monaco;
}

/**
 * What a new script starts with: the shape the runner expects (`export default function (input,
 * octopus)` / `def run(input, octopus)`), using only what the runner provides.
 */
export const SAMPLE_SOURCE: Readonly<Record<'typescript' | 'python', string>> = {
  typescript: `// The platform calls the default export with the input and the \`octopus\` object, and
// takes what it returns as the result. console.log and octopus.log write to the log.
export default async function (input: { n?: number }, octopus: Octopus) {
  octopus.log('n =', input.n);
  return { double: (input.n ?? 0) * 2 };
}
`,
  python: `# The platform calls run(input, octopus) and takes what it returns as the result.
# print and octopus.log write to the log.
def run(input, octopus):
    octopus.log("n =", input.get("n"))
    return {"double": input.get("n", 0) * 2}
`,
};

export function sampleSource(language: string): string {
  return language === 'python' ? SAMPLE_SOURCE.python : SAMPLE_SOURCE.typescript;
}

/** The input a test run starts with. */
export const SAMPLE_INPUT = '{\n  "n": 21\n}\n';

/** Limits of the platform that the editor shows and respects (`crates/domain/src/scripts.rs`, openapi). */
export const LIMITS = {
  /** Bytes of source in a draft. */
  sourceBytes: 256 * 1024,
  /** Bytes of one schema. */
  schemaBytes: 64 * 1024,
  /** Bytes of the input of a test run (240 KiB). */
  inputBytes: 240 * 1024,
} as const;

/** The name of a script: a slug of 3 to 63 characters (`^[a-z][a-z0-9-]{1,61}[a-z0-9]$`). */
export const SCRIPT_NAME = /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/;
