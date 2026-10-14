import type { StatusView } from '../../model/status';
import type { ScriptTestRunSummary } from './api';
import { clipLabel } from './text';

/** How the states of a test run look: always a word and a shape, never only a colour. */
const TEST_RUN: Record<string, StatusView> = {
  QUEUED: { label: 'Queued', symbol: '◔', tone: 'idle', help: 'Waiting for a sandbox runner to take it.' },
  RUNNING: { label: 'Running', symbol: '▶', tone: 'running', help: 'A runner is building the draft and running it.' },
  SUCCEEDED: { label: 'Succeeded', symbol: '✓', tone: 'ok', help: 'The script ran and returned a result.' },
  FAILED: { label: 'Failed', symbol: '✕', tone: 'bad', help: 'The script did not produce a result; the failure code says why.' },
};

const VERSION: Record<string, StatusView> = {
  BUILDING: { label: 'Building', symbol: '↻', tone: 'running', help: 'A runner is building this version in isolation.' },
  READY: { label: 'Ready', symbol: '✓', tone: 'ok', help: 'Built. A workflow can pin its artifact digest.' },
  FAILED: { label: 'Build failed', symbol: '✕', tone: 'bad', help: 'The build made nothing; the reason is kept with the version.' },
};

const UNKNOWN: StatusView = { label: 'Unknown', symbol: '?', tone: 'idle', help: 'A state this version of the Studio does not know.' };

export function testRunStatus(status: string): StatusView {
  return TEST_RUN[status] ?? { ...UNKNOWN, label: clipLabel(status, 24) };
}

export function versionStatus(status: string): StatusView {
  return VERSION[status] ?? { ...UNKNOWN, label: clipLabel(status, 24) };
}

export function isTestRunFinal(status: string): boolean {
  return status === 'SUCCEEDED' || status === 'FAILED';
}

export function isVersionFinal(status: string): boolean {
  return status === 'READY' || status === 'FAILED';
}

/** What each failure code of a test run means (the list in `getScriptTestRun`). */
const FAILURE: Record<string, string> = {
  STORAGE_QUOTA_EXCEEDED: 'The tenant retained-content limit was reached. Free capacity before starting new work.',
  SCRIPT_ERROR: 'The script threw an error.',
  SCRIPT_TIMEOUT: 'The script ran past its time limit and was stopped.',
  SCRIPT_OUT_OF_MEMORY: 'The script used more memory than its limit and was killed.',
  SCRIPT_OUTPUT_LIMIT: 'The script wrote more than its output limit and was stopped.',
  SCRIPT_DENIED: 'The script asked for something its policy does not allow: the network or a secret.',
  SCRIPT_SCHEMA_VIOLATION: 'The input or the result did not satisfy the schema of the script (the rule and the place are in the log, never the value).',
  SCRIPT_NO_RESULT: 'The script ended without returning a result.',
  SCRIPT_BUILD_REJECTED: 'The platform could not build the source: it does not parse, or it does not export what it must.',
  SCRIPT_SANDBOX_UNAVAILABLE: 'No sandbox runner took the run in time, or the sandbox was not available. Nothing about the script is known.',
};

export function failureHelp(code: string | undefined): string {
  if (code === undefined) return '';
  return FAILURE[code] ?? `A failure code this version of the Studio does not know (${clipLabel(code, 40)}).`;
}

/** What the prefix of the `failure` of a version says about where the build stopped. */
const BUILD_FAILURE: [string, string][] = [
  ['STORAGE_QUOTA_EXCEEDED', 'The tenant retained-content limit was reached. Free capacity before publishing again.'],
  ['SCRIPT_SANDBOX_UNAVAILABLE', 'No runner could complete the build.'],
  ['SCRIPT_BUILD_REJECTED', 'The build was rejected; retained diagnostics may have been truncated.'],
  ['rejected:', 'The toolchain rejected the source.'],
  ['lockfile:', 'The lockfile pins a package; installing packages is not available yet.'],
  ['limit:', 'The build ran into a limit.'],
  ['unsupported:', 'This is not supported by the build.'],
  ['unavailable:', 'No runner could build it; trying again later, or publishing the same source again, may work.'],
];

export function buildFailureHelp(failure: string | undefined): string {
  if (failure === undefined) return '';
  return BUILD_FAILURE.find(([prefix]) => failure.startsWith(prefix))?.[1] ?? '';
}

export type Testedness =
  /** A test run of the current draft succeeded. */
  | { kind: 'passed'; run: ScriptTestRunSummary }
  | { kind: 'failed'; run: ScriptTestRunSummary }
  | { kind: 'pending'; run: ScriptTestRunSummary }
  | { kind: 'untested'; reason: 'no-run' | 'draft-changed' | 'unsaved' };

/**
 * Whether the draft as it is on the platform has been tried. A test run runs the draft as it was
 * when the run was asked for, and a draft change moves `updatedAt`, so a run asked for at or after
 * the last change tried this very draft. (The platform does not require a test run before a
 * version is published: this is advice, not a gate.)
 */
export function testedness(runs: readonly ScriptTestRunSummary[], draftUpdatedAt: string, unsaved: boolean): Testedness {
  if (unsaved) return { kind: 'untested', reason: 'unsaved' };
  const since = Date.parse(draftUpdatedAt);
  const current = runs.filter((run) => {
    const at = Date.parse(run.createdAt);
    return Number.isFinite(at) && Number.isFinite(since) && at >= since;
  });
  const newest = (list: ScriptTestRunSummary[]) => [...list].sort((x, y) => Date.parse(y.createdAt) - Date.parse(x.createdAt))[0];
  const passed = newest(current.filter((run) => run.status === 'SUCCEEDED'));
  if (passed) return { kind: 'passed', run: passed };
  const pending = newest(current.filter((run) => run.status === 'QUEUED' || run.status === 'RUNNING'));
  if (pending) return { kind: 'pending', run: pending };
  const failed = newest(current.filter((run) => run.status === 'FAILED'));
  if (failed) return { kind: 'failed', run: failed };
  return { kind: 'untested', reason: runs.length === 0 ? 'no-run' : 'draft-changed' };
}
