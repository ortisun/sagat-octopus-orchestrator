/**
 * What the runner hands a TypeScript script, as a declaration file for the editor's autocomplete.
 * It describes exactly what the runner provides (`crates/infra-sandbox/shims/runtime.mjs`, and the
 * description of `createScript` in the OpenAPI contract) and nothing else: the script is called as
 * `(input, octopus)`, so there is no `octopus.input`.
 */
export const OCTOPUS_DTS_PATH = 'file:///octopus.d.ts';

export const OCTOPUS_DTS = `
/** The answer to an \`octopus.fetch\`. */
interface OctopusResponse {
  readonly status: number;
  readonly headers: Record<string, string>;
  text(): Promise<string>;
  json(): Promise<unknown>;
}

interface OctopusFetchInit {
  method?: string;
  headers?: Record<string, string>;
  body?: string | null;
}

/** The second argument of the default function of a script: what the platform lets it ask for. */
interface Octopus {
  /**
   * Asks the platform to make an HTTP request. The script has no network of its own: the request
   * is made under the egress policy of the namespace, and only if the script's policy says
   * \`egress: namespace\`. Otherwise it is refused (and audited).
   */
  fetch(url: string, init?: OctopusFetchInit): Promise<OctopusResponse>;
  /**
   * The value of a secret the step declared. A test run has no secrets: this is always refused
   * there, so a production credential never reaches a script that is still being written.
   */
  secret(name: string): Promise<string>;
  /** Writes to the log of the run (so does \`console.log\`). */
  log(...args: unknown[]): void;
  /**
   * The same on every attempt of the same step: put it in the request of an external effect so the
   * receiver can tell a repeat from a new request. \`null\` in a test run.
   */
  readonly idempotencyKey: string | null;
}

declare const console: {
  log(...args: unknown[]): void;
  info(...args: unknown[]): void;
  debug(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
};
`;
