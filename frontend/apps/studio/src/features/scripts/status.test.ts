import { describe, expect, it } from 'vitest';
import type { ScriptTestRunSummary } from './api';
import { buildFailureHelp, failureHelp, isTestRunFinal, isVersionFinal, testedness, testRunStatus, versionStatus } from './status';
import { aRunSummary } from './testing';

const at = (minutes: number) => new Date(Date.UTC(2026, 9, 10, 9, minutes)).toISOString();
const run = (minutes: number, status: ScriptTestRunSummary['status'], extra: Partial<ScriptTestRunSummary> = {}) =>
  aRunSummary({ id: `00000000-0000-4000-8000-${String(minutes).padStart(12, '0')}`, createdAt: at(minutes), status, ...extra });

describe('statuses', () => {
  it('say every state with a word and a shape, never a colour only', () => {
    for (const status of ['QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED']) {
      const view = testRunStatus(status);
      expect(view.label.length).toBeGreaterThan(2);
      expect(view.symbol.length).toBeGreaterThan(0);
    }
    for (const status of ['BUILDING', 'READY', 'FAILED']) {
      const view = versionStatus(status);
      expect(view.label.length).toBeGreaterThan(2);
      expect(view.symbol.length).toBeGreaterThan(0);
    }
    expect(versionStatus('FAILED').label).toBe('Build failed');
  });

  it('shows a state it does not know as unknown, clipped, instead of failing', () => {
    expect(testRunStatus('SOMETHING_NEW').label).toBe('SOMETHING_NEW');
    expect(testRunStatus('X'.repeat(100)).label.length).toBeLessThanOrEqual(25);
    expect(versionStatus('WEIRD').tone).toBe('idle');
  });

  it('knows which states are final', () => {
    expect(['QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED'].filter(isTestRunFinal)).toEqual(['SUCCEEDED', 'FAILED']);
    expect(['BUILDING', 'READY', 'FAILED'].filter(isVersionFinal)).toEqual(['READY', 'FAILED']);
  });
});

describe('failure codes', () => {
  it('explains each code the platform names, and admits to not knowing the others', () => {
    for (const code of ['SCRIPT_ERROR', 'SCRIPT_TIMEOUT', 'SCRIPT_OUT_OF_MEMORY', 'SCRIPT_OUTPUT_LIMIT', 'SCRIPT_DENIED', 'SCRIPT_SCHEMA_VIOLATION', 'SCRIPT_NO_RESULT', 'SCRIPT_BUILD_REJECTED', 'SCRIPT_SANDBOX_UNAVAILABLE']) {
      expect(failureHelp(code)).not.toMatch(/does not know/);
    }
    expect(failureHelp('SCRIPT_FROM_THE_FUTURE')).toMatch(/does not know/);
    expect(failureHelp(undefined)).toBe('');
  });

  it('says where a build stopped from the prefix of its failure', () => {
    expect(buildFailureHelp('rejected: the script must export a default function')).toMatch(/rejected the source/);
    expect(buildFailureHelp('lockfile: pins left-pad')).toMatch(/pins a package/);
    expect(buildFailureHelp('unavailable: no runner')).toMatch(/No runner/);
    expect(buildFailureHelp('something else')).toBe('');
  });
});

describe('testedness: whether the draft as it is has been tried', () => {
  it('is untested when nothing was ever run', () => {
    expect(testedness([], at(0), false)).toEqual({ kind: 'untested', reason: 'no-run' });
  });

  it('counts a successful run asked for at or after the last change of the draft', () => {
    expect(testedness([run(5, 'SUCCEEDED')], at(5), false)).toMatchObject({ kind: 'passed' });
    expect(testedness([run(10, 'SUCCEEDED')], at(5), false)).toMatchObject({ kind: 'passed' });
  });

  it('does not count a run from before the draft changed', () => {
    expect(testedness([run(4, 'SUCCEEDED')], at(5), false)).toEqual({ kind: 'untested', reason: 'draft-changed' });
  });

  it('does not count anything while there are changes the platform has not got', () => {
    expect(testedness([run(10, 'SUCCEEDED')], at(5), true)).toEqual({ kind: 'untested', reason: 'unsaved' });
  });

  it('reports a failure, and a run that has not ended, but a success wins over an earlier failure', () => {
    expect(testedness([run(10, 'FAILED')], at(5), false)).toMatchObject({ kind: 'failed' });
    expect(testedness([run(10, 'RUNNING')], at(5), false)).toMatchObject({ kind: 'pending' });
    expect(testedness([run(12, 'SUCCEEDED'), run(10, 'FAILED')], at(5), false)).toMatchObject({ kind: 'passed' });
    expect(testedness([run(11, 'FAILED'), run(10, 'QUEUED')], at(5), false)).toMatchObject({ kind: 'pending' });
  });

  it('takes the newest run of its kind', () => {
    const out = testedness([run(8, 'SUCCEEDED'), run(12, 'SUCCEEDED'), run(10, 'SUCCEEDED')], at(5), false);
    expect(out.kind === 'passed' && out.run.createdAt).toBe(at(12));
  });

  it('does not trust a time it cannot read', () => {
    expect(testedness([run(10, 'SUCCEEDED', { createdAt: 'yesterday' })], at(5), false)).toEqual({ kind: 'untested', reason: 'draft-changed' });
    expect(testedness([run(10, 'SUCCEEDED')], 'never', false)).toEqual({ kind: 'untested', reason: 'draft-changed' });
  });
});
