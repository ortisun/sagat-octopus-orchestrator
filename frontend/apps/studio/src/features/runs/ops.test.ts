import { describe, expect, it } from 'vitest';
import type { RuntimeNode } from '../../api/types';
import { actionLabel, asksForStepId, checkReason, consequence, runOperations, stepOperations } from './ops';
import type { Action, Operation, RunFacts } from './ops';

const operator = (p: string) => p === 'run:operate';
const admin = () => true;
const viewer = () => false;
const node = (over: Partial<RuntimeNode> = {}): RuntimeNode => ({ id: 'pay', type: 'activity', status: 'WAITING', waitingOn: 'operator', ...over });
const run = (over: Partial<RunFacts> = {}): RunFacts => ({ status: 'MANUAL_REVIEW', paused: false, ...over });
const by = (ops: Operation[], action: Action): Operation => ops.find((o) => o.action === action)!;
const unresolvedEffectCodes = [
  'OUTCOME_UNKNOWN',
  'LEASE_EXPIRED',
  'ATTEMPT_TIMEOUT',
  'RECONCILIATION_FAILED',
  'RECONCILIATION_UNKNOWN',
  'RECONCILIATION_NOT_FINAL',
  'STORAGE_QUOTA_EXCEEDED',
] as const;

describe('runOperations', () => {
  it('offers pause and cancel on a running run, and not resume', () => {
    const ops = runOperations(run({ status: 'RUNNING' }), operator);
    expect(by(ops, 'PAUSE').enabled).toBe(true);
    expect(by(ops, 'CANCEL').enabled).toBe(true);
    expect(by(ops, 'RESUME')).toMatchObject({ enabled: false, reason: 'The run is not paused.' });
  });

  it('offers resume and cancel on a paused run, and not pause', () => {
    const ops = runOperations(run({ status: 'PAUSED', paused: true }), operator);
    expect(by(ops, 'RESUME').enabled).toBe(true);
    expect(by(ops, 'CANCEL').enabled).toBe(true);
    expect(by(ops, 'PAUSE')).toMatchObject({ enabled: false, reason: 'The run is already paused.' });
  });

  it('knows a run can be paused while it shows another status, such as needing a person', () => {
    const ops = runOperations(run({ status: 'MANUAL_REVIEW', paused: true }), operator);
    expect(by(ops, 'RESUME').enabled).toBe(true);
  });

  it('refuses everything on a finished run, with that reason', () => {
    for (const status of ['SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED']) {
      const ops = runOperations(run({ status }), admin);
      for (const action of ['PAUSE', 'RESUME', 'CANCEL'] as const) expect(by(ops, action)).toMatchObject({ enabled: false, reason: 'The run is finished.' });
    }
  });

  it('does not offer pause or cancel to a run that is known to be stopping', () => {
    const ops = runOperations(run({ status: 'CANCELLING' }), operator);
    expect(by(ops, 'PAUSE')).toMatchObject({ enabled: false, reason: 'The run is already being stopped.' });
    expect(by(ops, 'CANCEL')).toMatchObject({ enabled: false, reason: 'The run is already being stopped.' });
  });

  it('leaves to the platform what the status does not settle: a run that compensates or waits for a person may or may not have been asked to stop', () => {
    for (const status of ['COMPENSATING', 'MANUAL_REVIEW']) {
      const ops = runOperations(run({ status }), operator);
      expect(by(ops, 'PAUSE').enabled).toBe(true);
      expect(by(ops, 'CANCEL').enabled).toBe(true);
    }
  });

  it('says what permission is missing, whatever the state', () => {
    const ops = runOperations(run({ status: 'RUNNING' }), viewer);
    for (const action of ['PAUSE', 'RESUME', 'CANCEL'] as const) expect(by(ops, action)).toMatchObject({ enabled: false, reason: 'You need the run:operate permission to do this.', needs: 'run:operate' });
  });

  it('offers to start a run again only when it ended, and to someone who may operate and start runs', () => {
    const both = (p: string) => p === 'run:operate' || p === 'run:start';
    for (const status of ['SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT', 'COMPENSATED', 'PARTIAL', 'COMPENSATION_FAILED']) {
      expect(by(runOperations(run({ status }), both), 'RERUN_AS_NEW').enabled).toBe(true);
    }
    // While it goes on, starting it again would do its work twice.
    for (const status of ['RUNNING', 'PAUSED', 'MANUAL_REVIEW', 'COMPENSATING']) {
      expect(by(runOperations(run({ status }), both), 'RERUN_AS_NEW')).toMatchObject({ enabled: false, reason: 'The run has not ended: starting it again would do its work twice.' });
    }
    expect(by(runOperations(run({ status: 'FAILED' }), operator), 'RERUN_AS_NEW')).toMatchObject({ enabled: false, reason: 'You need the run:start permission to do this.' });
    expect(by(runOperations(run({ status: 'FAILED' }), viewer), 'RERUN_AS_NEW')).toMatchObject({ enabled: false, reason: 'You need the run:operate permission to do this.' });
  });

  it('no longer lists holding the run: a step is held, not the run', () => {
    expect(runOperations(run(), admin).map((o) => o.action)).toEqual(['PAUSE', 'RESUME', 'CANCEL', 'RERUN_AS_NEW']);
  });
});

describe('stepOperations', () => {
  it('offers retry and resolve for a step that waits for a person; a compensation is retried the other way', () => {
    const ops = stepOperations(node(), run(), admin);
    expect(by(ops, 'RETRY_STEP').enabled).toBe(true);
    expect(by(ops, 'RESOLVE_STEP').enabled).toBe(true);
    expect(by(ops, 'RETRY_COMPENSATION')).toMatchObject({ enabled: false });
    expect(by(ops, 'SKIP_COMPENSATION')).toMatchObject({ enabled: false, reason: 'Only a compensation can be skipped.' });
  });

  it('offers retry, skip and resolve for a compensation that waits for a person, but not Retry step', () => {
    const ops = stepOperations(node({ id: 'reserve~c' }), run(), admin);
    expect(by(ops, 'RETRY_COMPENSATION').enabled).toBe(true);
    expect(by(ops, 'SKIP_COMPENSATION').enabled).toBe(true);
    expect(by(ops, 'RESOLVE_STEP').enabled).toBe(true);
    expect(by(ops, 'RETRY_STEP')).toMatchObject({ enabled: false, reason: 'This is a compensation: use Retry compensation.' });
  });

  it('offers nothing for a step that does not wait for a decision, and says so', () => {
    for (const waitingOn of [null, undefined, 'timer', 'event', 'child'] as const) {
      const ops = stepOperations(node({ id: 'reserve~c', status: 'RUNNING', waitingOn }), run({ status: 'RUNNING' }), admin);
      for (const o of ops) expect(o.enabled).toBe(false);
      expect(by(ops, 'RETRY_COMPENSATION').reason).toBe('This step is not waiting for a decision.');
      expect(by(ops, 'RESOLVE_STEP').reason).toBe('This step is not waiting for a decision.');
    }
  });

  it('asks saga:override for skipping and resolving, run:operate for retrying', () => {
    const ops = stepOperations(node({ id: 'reserve~c' }), run(), operator);
    expect(by(ops, 'RETRY_COMPENSATION').enabled).toBe(true);
    expect(by(ops, 'SKIP_COMPENSATION')).toMatchObject({ enabled: false, reason: 'You need the saga:override permission to do this.', needs: 'saga:override' });
    expect(by(ops, 'RESOLVE_STEP')).toMatchObject({ enabled: false, reason: 'You need the saga:override permission to do this.' });
  });

  it('gives a viewer nothing, naming what they lack', () => {
    const ops = stepOperations(node(), run(), viewer);
    expect(by(ops, 'RETRY_STEP')).toMatchObject({ enabled: false, reason: 'You need the run:operate permission to do this.' });
    expect(by(ops, 'RESOLVE_STEP')).toMatchObject({ enabled: false, reason: 'You need the saga:override permission to do this.' });
  });

  it('disables direct retry of every unresolved original or compensation even for an admin', () => {
    for (const failureCode of unresolvedEffectCodes) {
      for (const compensation of [false, true]) {
        for (const status of ['WAITING', 'RETRY_WAIT'] as const) {
          const unresolved = node({
            id: compensation ? 'pay~c' : 'pay',
            status,
            waitingOn: status === 'WAITING' ? 'operator' : null,
            failureCode,
          });
          const facts = run({ status: status === 'WAITING' ? 'MANUAL_REVIEW' : 'RUNNING' });
          const action = compensation ? 'RETRY_COMPENSATION' : 'RETRY_STEP';
          const retry = by(stepOperations(unresolved, facts, admin), action);

          expect(retry.enabled, `${failureCode} ${unresolved.id} ${status}`).toBe(false);
          expect(retry.reason, `${failureCode} remains visible`).toContain(failureCode);
          expect(retry.reason, `${failureCode} requires an outcome-bearing decision`).toMatch(/resolve|resolution/i);

          if (status === 'WAITING') {
            const decisions = stepOperations(unresolved, facts, admin);
            expect(by(decisions, 'RESOLVE_STEP').enabled).toBe(true);
            expect(by(decisions, 'SKIP_COMPENSATION').enabled).toBe(compensation);
          }
        }
      }
    }
  });

  it('retains retry for known failures without weakening compensation decisions', () => {
    for (const compensation of [false, true]) {
      for (const status of ['WAITING', 'RETRY_WAIT'] as const) {
        const known = node({
          id: compensation ? 'reserve~c' : 'pay',
          status,
          waitingOn: status === 'WAITING' ? 'operator' : null,
          failureCode: 'PAYMENT_DECLINED',
        });
        const facts = run({ status: status === 'WAITING' ? 'MANUAL_REVIEW' : 'RUNNING' });
        const action = compensation ? 'RETRY_COMPENSATION' : 'RETRY_STEP';
        expect(by(stepOperations(known, facts, operator), action).enabled).toBe(true);
      }
    }
  });

  it('offers to retry a step that is waiting out its retry now, without waiting for the delay', () => {
    const waiting = node({ status: 'RETRY_WAIT', waitingOn: null, failureCode: 'PAYMENT_DECLINED' });
    const ops = stepOperations(waiting, run({ status: 'RUNNING' }), operator);
    expect(by(ops, 'RETRY_STEP').enabled).toBe(true);
    expect(by(ops, 'RESOLVE_STEP')).toMatchObject({ enabled: false });
    // Even a stale RETRY_WAIT projection must fail closed when its failure code says the outcome is unknown.
    const unknown = node({ status: 'RETRY_WAIT', waitingOn: null, failureCode: 'LEASE_EXPIRED' });
    expect(by(stepOperations(unknown, run({ status: 'RUNNING' }), admin), 'RETRY_STEP')).toMatchObject({ enabled: false });
    expect(consequence('RETRY_STEP', 'pay', waiting)).toMatch(/Starts the next attempt of step pay now, without waiting for the delay/);
  });

  it('offers to hold a step that has not started, and says why not for the others', () => {
    for (const status of ['READY', 'RETRY_WAIT'] as const) {
      expect(by(stepOperations(node({ status, waitingOn: null }), run({ status: 'RUNNING' }), operator), 'MARK_MANUAL_REVIEW').enabled).toBe(true);
    }
    for (const status of ['RUNNING', 'LEASED', 'WAITING', 'SUCCEEDED', 'FAILED'] as const) {
      expect(by(stepOperations(node({ status, waitingOn: null }), run({ status: 'RUNNING' }), operator), 'MARK_MANUAL_REVIEW').reason).toMatch(/Only a step that has not started can be held/);
    }
    expect(by(stepOperations(node({ id: 'pay~r', status: 'READY', waitingOn: null }), run({ status: 'RUNNING' }), operator), 'MARK_MANUAL_REVIEW').reason).toBe('A lookup is held by holding the step it is for.');
    expect(by(stepOperations(node({ status: 'READY', waitingOn: null }), run({ status: 'RUNNING' }), viewer), 'MARK_MANUAL_REVIEW')).toMatchObject({ enabled: false, reason: 'You need the run:operate permission to do this.' });
    expect(consequence('MARK_MANUAL_REVIEW', 'pay')).toMatch(/Holds step pay for a person/);
  });

  it('refuses a decision on a finished run', () => {
    const ops = stepOperations(node(), run({ status: 'PARTIAL' }), admin);
    for (const o of ops) expect(o).toMatchObject({ enabled: false, reason: 'The run is finished.' });
  });
});

describe('words', () => {
  it('has a button label for every action and a consequence in plain words', () => {
    const all: Action[] = ['PAUSE', 'RESUME', 'CANCEL', 'RETRY_STEP', 'RETRY_COMPENSATION', 'SKIP_COMPENSATION', 'RESOLVE_STEP', 'RERUN_AS_NEW', 'MARK_MANUAL_REVIEW'];
    for (const action of all) {
      expect(actionLabel(action)).not.toBe('');
      expect(consequence(action, 'pay')).not.toBe('');
    }
    expect(consequence('RETRY_STEP', 'pay')).toBe('Asks for another attempt of step pay; the earlier attempts stay in the journal.');
    expect(consequence('SKIP_COMPENSATION', 'pay~c')).toMatch(/partly undone and cannot be compensated again/);
    expect(consequence('RESOLVE_STEP', 'pay')).toMatch(/overrides what the platform knows/);
    expect(consequence('CANCEL')).toMatch(/preserv.*review|review.*preserv/i);
    expect(consequence('CANCEL')).toMatch(/unknown|unresolved/i);
  });

  it('asks for the step id back only for available outcome-bearing override decisions', () => {
    expect(asksForStepId('SKIP_COMPENSATION')).toBe(true);
    expect(asksForStepId('RESOLVE_STEP')).toBe(true);
    for (const action of ['RETRY_STEP', 'RETRY_COMPENSATION', 'PAUSE', 'RESUME', 'CANCEL'] as const) expect(asksForStepId(action)).toBe(false);

  });
});

describe('checkReason', () => {
  it('counts after trimming, between 8 and 2000 characters', () => {
    expect(checkReason('        a', 'run')).toMatchObject({ length: 1 });
    expect(checkReason('        a', 'run').problem).toMatch(/at least 8/);
    expect(checkReason('1234567', 'run').problem).toBeDefined();
    expect(checkReason('12345678', 'run').problem).toBeUndefined();
    expect(checkReason('x'.repeat(2000), 'run').problem).toBeUndefined();
    expect(checkReason('x'.repeat(2001), 'run').problem).toMatch(/1 too many/);
  });

  it('counts characters, not UTF-16 units', () => {
    expect(checkReason('🙂'.repeat(8), 'run')).toMatchObject({ length: 8 });
    expect(checkReason('🙂'.repeat(8), 'run').problem).toBeUndefined();
  });

  it('keeps the reason of a step decision to plain text on one line, as the platform does', () => {
    expect(checkReason('line one\nline two', 'step').problem).toMatch(/one line/);
    expect(checkReason('line one\nline two', 'run').problem).toBeUndefined();
    expect(checkReason('tab\there, ok', 'step').problem).toMatch(/one line/);
  });
});
