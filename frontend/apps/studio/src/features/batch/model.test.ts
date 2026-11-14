import { describe, expect, it } from 'vitest';
import type { BatchSelector } from '../../api/endpoints';
import { EMPTY_DRAFT, describeFilters, failureText, isActive, itemState, jobState, maxItemsOf, progress, reasonProblem, reasonText, requestOf, selectorOf } from './model';
import type { Draft } from './model';

const SELECTOR: BatchSelector = { filter: {} };

/** The problem a request was refused with, or undefined when it was made. */
const problemOf = (result: ReturnType<typeof requestOf>): string | undefined => ('problem' in result ? result.problem : undefined);
const REASON = 'clean up the runs that are stuck';
const counts = (c: Partial<Record<'selected' | 'succeeded' | 'skipped' | 'failed' | 'cancelled' | 'deferred', number>>) => ({ selected: 0, succeeded: 0, skipped: 0, failed: 0, cancelled: 0, deferred: 0, ...c });

describe('the selector of a batch job', () => {
  it('is the filters of the run list and nothing else', () => {
    expect(selectorOf({ status: '', workflowId: '', origin: '' })).toEqual({ filter: {} });
    expect(selectorOf({ status: 'MANUAL_REVIEW', workflowId: '0194f5e2-7b1c-7000-8000-000000000001', origin: '' })).toEqual({
      filter: { status: ['MANUAL_REVIEW'], workflowId: '0194f5e2-7b1c-7000-8000-000000000001' },
    });
    expect(selectorOf({ status: '', workflowId: '', origin: 'AD_HOC_STEP' })).toEqual({ filter: { origin: 'AD_HOC_STEP' } });
  });

  it('is described in words, with the name of the workflow when it is known', () => {
    expect(describeFilters({ status: '', workflowId: '', origin: '' })).toBe('every run of the namespace');
    expect(describeFilters({ status: 'MANUAL_REVIEW', workflowId: 'w-1', origin: '' }, 'checkout')).toBe('the runs with status manual review and workflow checkout');
  });

  it('carries the business id prefix of the run list, as the platform takes it: a literal in the filter', () => {
    expect(selectorOf({ status: '', workflowId: '', origin: '', businessIdPrefix: 'order-' })).toEqual({ filter: { businessIdPrefix: 'order-' } });
    expect(selectorOf({ status: 'FAILED', workflowId: '', origin: 'AD_HOC_STEP', businessIdPrefix: 'order-' })).toEqual({
      filter: { status: ['FAILED'], origin: 'AD_HOC_STEP', businessIdPrefix: 'order-' },
    });
    // characters that mean something elsewhere are not touched: the platform matches them as they are
    expect(selectorOf({ status: '', workflowId: '', origin: '', businessIdPrefix: '50%_off\\' })).toEqual({ filter: { businessIdPrefix: '50%_off\\' } });
  });

  it('has no key for an empty prefix or none: no null, no empty string', () => {
    expect(selectorOf({ status: '', workflowId: '', origin: '', businessIdPrefix: '' })).toEqual({ filter: {} });
    expect(selectorOf({ status: '', workflowId: '', origin: '' })).toEqual({ filter: {} });
    expect(JSON.stringify(selectorOf({ status: 'FAILED', workflowId: '', origin: '', businessIdPrefix: '' }))).not.toContain('businessIdPrefix');
  });

  it('says the prefix in words, among the other filters', () => {
    expect(describeFilters({ status: '', workflowId: '', origin: '', businessIdPrefix: 'order-' })).toBe('the runs with business id starting with "order-"');
    expect(describeFilters({ status: 'MANUAL_REVIEW', workflowId: 'w-1', origin: '', businessIdPrefix: 'order-' }, 'checkout')).toBe(
      'the runs with status manual review and workflow checkout and business id starting with "order-"',
    );
    expect(describeFilters({ status: '', workflowId: '', origin: '', businessIdPrefix: '' })).toBe('every run of the namespace');
  });
});

describe('the request a draft makes', () => {
  const draft = (changes: Partial<Draft>): Draft => ({ ...EMPTY_DRAFT, ...changes });

  it('asks for what the draft says and trims the reason', () => {
    const result = requestOf(draft({ action: 'PAUSE', maxItems: '500', rate: '12.5' }), SELECTOR, `  ${REASON}  `);
    expect(result).toEqual({ body: { selector: SELECTOR, action: 'PAUSE', reason: REASON, maxItems: 500, itemsPerSecond: 12.5 } });
  });

  it('leaves the defaults of the platform when the limits are empty', () => {
    const result = requestOf(draft({}), SELECTOR, REASON);
    expect(result).toEqual({ body: { selector: SELECTOR, action: 'CANCEL', reason: REASON } });
  });

  it('refuses a reason that is too short or too long, counting characters and not bytes', () => {
    expect(reasonProblem('short')).toMatch(/at least 8/);
    expect(reasonProblem('é'.repeat(8))).toBeUndefined();
    expect(reasonProblem('x'.repeat(2001))).toMatch(/2000/);
    expect(problemOf(requestOf(draft({}), SELECTOR, 'short'))).toMatch(/at least 8/);
  });

  it('refuses limits outside what the platform takes', () => {
    for (const maxItems of ['0', '100001', '1.5', 'many', '-3']) {
      expect(problemOf(requestOf(draft({ maxItems }), SELECTOR, REASON))).toMatch(/from 1 to 100,000/);
    }
    for (const rate of ['0', '-1', '200.5', 'fast', 'NaN']) {
      expect(problemOf(requestOf(draft({ rate }), SELECTOR, REASON))).toMatch(/at most 200/);
    }
    expect(maxItemsOf('100000')).toEqual({ value: 100000 });
    expect(maxItemsOf('')).toEqual({});
  });

  it('needs a valid signal: a name, a JSON object, at most 256 KiB', () => {
    const signal = (changes: Partial<Draft>) => requestOf(draft({ action: 'SIGNAL', signalName: 'order.approved', ...changes }), SELECTOR, REASON);
    expect(signal({})).toEqual({ body: { selector: SELECTOR, action: 'SIGNAL', reason: REASON, signal: { name: 'order.approved', payload: {} } } });
    expect(problemOf(signal({ signalName: 'bad/name' }))).toMatch(/signal name/);
    for (const payload of ['{', '[]', 'null', '12']) expect(problemOf(signal({ payload }))).toMatch(/JSON/);
    expect(problemOf(signal({ payload: JSON.stringify({ value: 'é'.repeat(140_000) }) }))).toMatch(/256 KiB/);
  });

  it('needs a step for the actions that name one, including the step of a compensation', () => {
    const step = (action: 'RETRY_STEP' | 'MARK_MANUAL_REVIEW', stepId: string) => requestOf(draft({ action, stepId }), SELECTOR, REASON);
    expect(step('RETRY_STEP', 'reserve~c')).toEqual({ body: { selector: SELECTOR, action: 'RETRY_STEP', reason: REASON, stepId: 'reserve~c' } });
    expect(step('MARK_MANUAL_REVIEW', 'charge')).toMatchObject({ body: { stepId: 'charge' } });
    for (const bad of ['', 'two words', '~c']) expect(problemOf(step('RETRY_STEP', bad))).toMatch(/Name the step/);
  });

  it('sends no parameter an action does not take', () => {
    const result = requestOf(draft({ action: 'RESUME', signalName: 'left.over', stepId: 'left-over' }), SELECTOR, REASON);
    expect(result).toEqual({ body: { selector: SELECTOR, action: 'RESUME', reason: REASON } });
  });
});

describe('progress', () => {
  it('counts the items that have an outcome of the items selected', () => {
    expect(progress({ state: 'RUNNING', counts: counts({ selected: 40, succeeded: 20, skipped: 4, failed: 1, cancelled: 0, deferred: 3 }) })).toEqual({ done: 25, total: 40, percent: 62 });
  });

  it('is complete for a job that selected nothing and ended, and not started while it selects', () => {
    expect(progress({ state: 'COMPLETED', counts: counts({}) })).toEqual({ done: 0, total: 0, percent: 100 });
    expect(progress({ state: 'SELECTING', counts: counts({}) })).toEqual({ done: 0, total: 0, percent: 0 });
  });

  it('knows which states are still at work', () => {
    expect(['SELECTING', 'RUNNING', 'CANCELLING'].every(isActive)).toBe(true);
    expect(['COMPLETED', 'CANCELLED', 'FAILED', undefined].some(isActive)).toBe(false);
  });
});

describe('words', () => {
  it('give every state a word and a shape, and say so for one they do not know', () => {
    for (const state of ['SELECTING', 'RUNNING', 'CANCELLING', 'COMPLETED', 'CANCELLED', 'FAILED']) expect(jobState(state).label).not.toBe('Unknown');
    for (const state of ['PENDING', 'DEFERRED', 'SUCCEEDED', 'SKIPPED', 'FAILED', 'CANCELLED']) expect(itemState(state).symbol).not.toBe('?');
    expect(jobState('SOMETHING_NEW').label).toBe('Unknown');
  });

  it('explain the reasons of a skip or a deferral, and show an unknown code as it is', () => {
    expect(reasonText('NOT_WAITING')).toMatch(/waiting/);
    expect(reasonText('A_NEW_REASON')).toBe('A_NEW_REASON');
    expect(reasonText(null)).toBe('');
    expect(failureText('AUTHORITY_REVOKED')).toMatch(/lost a permission/);
    expect(failureText(null)).toBe('');
  });
});
