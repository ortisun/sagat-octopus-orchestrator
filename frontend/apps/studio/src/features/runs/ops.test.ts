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

  it('does not offer pause or cancel to a run that is already being stopped', () => {
    for (const status of ['CANCELLING', 'COMPENSATING']) {
      const ops = runOperations(run({ status }), operator);
      expect(by(ops, 'PAUSE')).toMatchObject({ enabled: false, reason: 'The run is already being stopped.' });
      expect(by(ops, 'CANCEL')).toMatchObject({ enabled: false, reason: 'The run is already being stopped.' });
    }
  });

  it('says what permission is missing, whatever the state', () => {
    const ops = runOperations(run({ status: 'RUNNING' }), viewer);
    for (const action of ['PAUSE', 'RESUME', 'CANCEL'] as const) expect(by(ops, action)).toMatchObject({ enabled: false, reason: 'You need the run:operate permission to do this.', needs: 'run:operate' });
  });

  it('lists what the platform does not do yet, disabled, for everybody (G145)', () => {
    for (const can of [admin, operator, viewer]) {
      const ops = runOperations(run({ status: 'FAILED' }), can);
      for (const action of ['RERUN_AS_NEW', 'MARK_MANUAL_REVIEW'] as const) expect(by(ops, action)).toMatchObject({ enabled: false, reason: 'The platform does not offer this yet.' });
    }
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

  it('asks for saga:override to retry a step whose effect may already have happened', () => {
    const unknown = node({ failureCode: 'OUTCOME_UNKNOWN' });
    expect(by(stepOperations(unknown, run(), operator), 'RETRY_STEP')).toMatchObject({ enabled: false });
    expect(by(stepOperations(unknown, run(), operator), 'RETRY_STEP').reason).toMatch(/OUTCOME_UNKNOWN.*saga:override/);
    expect(by(stepOperations(unknown, run(), admin), 'RETRY_STEP').enabled).toBe(true);
    // A failure the platform knows left no effect is the operator's to retry.
    expect(by(stepOperations(node({ failureCode: 'RECONCILED_NOT_APPLIED' }), run(), operator), 'RETRY_STEP').enabled).toBe(true);
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
    expect(consequence('CANCEL')).toMatch(/compensated according to its cancel policy/);
  });

  it('asks for the step id back only for the two decisions that override', () => {
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
