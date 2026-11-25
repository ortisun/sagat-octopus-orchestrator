import { render, screen, within } from '@testing-library/react';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { Run } from '../../api/types';
import { openRunPage } from '../../../test/runPage';
import { at, graphOf, node, runOf } from '../../../test/runs';
import { installRunsDom } from '../../../test/runsDom';
import { nodeSummary, waitingWords } from './display';
import { gateWords, rateWords } from './gates';
import { RunGates } from './RunGates';

// Feature 022 (T022-055, T022-085; FR-022-022, FR-022-048, FR-022-057; FR-007-010): a run's permit and slot.

let undo: () => void;
beforeAll(() => {
  undo = installRunsDom();
});
afterAll(() => undo());

type RunConcurrency = NonNullable<Run['concurrency']>;
type RunRateLimit = NonNullable<Run['rateLimit']>;

const permit = (over: Partial<RunConcurrency> = {}): RunConcurrency => ({
  state: 'WAITING',
  pool: 'ORDINARY',
  group: 'orders-by-account',
  keyLabel: '3f9a1c02be7d',
  since: at(5),
  queueDeadline: at(605),
  ...over,
});

const slot = (over: Partial<RunRateLimit> = {}): RunRateLimit => ({
  state: 'WAITING',
  pool: 'ORDINARY',
  group: 'provider-starts',
  keyLabel: '0c1d2e3f4a5b',
  limit: 100,
  per: 'minute',
  since: at(10),
  queueDeadline: at(910),
  position: 3,
  estimatedGrantAt: at(40),
  ...over,
});

const facts = (region: HTMLElement, heading: string) => {
  const group = within(region).getByRole('group', { name: heading });
  return Object.fromEntries(within(group).getAllByRole('term').map((term) => [term.textContent, term.nextElementSibling?.textContent ?? '']));
};

describe('the words of a gate', () => {
  it('a permit held or waited for, with its group, pool and label', () => {
    expect(gateWords(permit({ state: 'HOLDING', queueDeadline: null }))).toMatchObject({ state: 'Holds a concurrency permit', waiting: false, sinceLabel: 'Holding since', unsupported: [] });
    expect(gateWords(permit())).toMatchObject({ state: 'Waiting for a concurrency permit', waiting: true, sinceLabel: 'Waiting since', deadline: at(605), key: '3f9a1c02be7d', group: 'orders-by-account', pool: 'Ordinary runs' });
    expect(gateWords(permit({ pool: 'DRAFT_TEST', group: '=order-flow' }))).toMatchObject({ pool: 'Draft test runs', group: '=order-flow (the workflow order-flow alone)' });
    expect(gateWords(permit({ group: '=order-flow/notify' }))).toMatchObject({ group: '=order-flow/notify (the step notify of order-flow alone)' });
    expect(gateWords(undefined)).toBeUndefined();
  });

  it('says a hidden label in words, not as an empty value (FR-022-057)', () => {
    expect(gateWords(permit({ keyLabel: null }))?.key).toBe('Hidden: you may not reveal personal data in this namespace, so the platform does not show the key label.');
    expect(rateWords(slot({ keyLabel: null }))?.key).toBe('Hidden: you may not reveal personal data in this namespace, so the platform does not show the key label.');
  });

  it('a waiting slot with its place and the platform estimate, called an estimate', () => {
    expect(rateWords(slot())).toMatchObject({ state: 'Waiting for a rate-limit slot', limit: 'At most 100 a minute', position: '2 ahead of it in the queue', estimate: at(40), unsupported: [] });
    expect(rateWords(slot({ position: 1 }))?.position).toBe('Next in the queue');
    expect(rateWords(slot({ position: null, estimatedGrantAt: null }))).toMatchObject({ position: 'Not known at this read', estimate: null });
    const granted = rateWords(slot({ state: 'GRANTED', since: at(10), queueDeadline: null, position: null, estimatedGrantAt: null }));
    expect(granted).toMatchObject({ state: 'Granted a rate-limit slot', waiting: false, sinceLabel: 'Granted at', countsUntil: at(70) });
  });

  it('shows what a newer platform sends as unsupported, never as a known word and never a crash (FR-007-010)', () => {
    const unknown = gateWords({ ...permit(), state: 'RELEASING', pool: 'BATCH' } as unknown as RunConcurrency);
    expect(unknown?.state).toBe('In a state this Studio does not know (RELEASING)');
    expect(unknown?.unsupported).toEqual(['state RELEASING', 'pool BATCH']);
    const rate = rateWords({ ...slot(), per: 'day', state: 'X'.repeat(200) } as unknown as RunRateLimit);
    expect(rate?.limit).toBe('At most 100 in a window this Studio does not know (day)');
    expect(rate?.unsupported).toEqual([`state ${'X'.repeat(40)}…`, 'window day']);
    for (const value of ['HOLDING', 7, ['x'], null] as const) {
      expect(gateWords(value as unknown as RunConcurrency)).toMatchObject({ state: 'A gate this Studio cannot read', unsupported: ['the whole value'] });
    }
  });
});

describe('the run view', () => {
  it('lists the permit and the slot of a run with their instants', () => {
    render(<RunGates run={runOf({ status: 'WAITING', concurrency: permit({ state: 'HOLDING', queueDeadline: null }), rateLimit: slot() })} />);
    const region = screen.getByRole('region', { name: 'Flow control' });
    expect(facts(region, 'Concurrency permit')).toMatchObject({ State: 'Holds a concurrency permit', Group: 'orders-by-account', Pool: 'Ordinary runs', 'Key label': '3f9a1c02be7d' });
    expect(within(within(region).getByRole('group', { name: 'Concurrency permit' })).queryByText('Gives up at')).not.toBeInTheDocument();
    const rate = facts(region, 'Rate-limit slot');
    expect(rate).toMatchObject({ State: 'Waiting for a rate-limit slot', Limit: 'At most 100 a minute', Place: '2 ahead of it in the queue' });
    expect(rate['Estimated grant']).toMatch(/an estimate, not a promise/);
    expect(rate['Gives up at']).toMatch(/RATE_LIMIT_WAIT_TIMEOUT/);
    const times = [...within(region).getByRole('group', { name: 'Rate-limit slot' }).querySelectorAll('time')];
    expect(times.map((t) => t.getAttribute('datetime'))).toEqual([at(10), at(910), at(40)]);
  });

  it('says an absent estimate is absent, and a newer state is unsupported', () => {
    render(<RunGates run={runOf({ rateLimit: { ...slot({ estimatedGrantAt: null }), state: 'PAUSED' } as unknown as RunRateLimit })} />);
    const rate = facts(screen.getByRole('region', { name: 'Flow control' }), 'Rate-limit slot');
    expect(rate['State']).toBe('In a state this Studio does not know (PAUSED)');
    expect(rate['Unsupported']).toBe('state PAUSED: shown as the platform sent it');
  });

  it('shows nothing for a run without a gate', () => {
    const { container } = render(<RunGates run={runOf()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('is in the header of the run page, from the single-run read', async () => {
    await openRunPage({ run: runOf({ status: 'WAITING', concurrency: permit({ keyLabel: null }) }) });
    const region = await screen.findByRole('region', { name: 'Flow control' });
    expect(region.closest('header')).not.toBeNull();
    expect(facts(region, 'Concurrency permit')['Key label']).toMatch(/^Hidden: you may not reveal personal data/);
    expect(facts(region, 'Concurrency permit')['Gives up at']).toMatch(/CONCURRENCY_WAIT_TIMEOUT/);
  });
});

describe('a step waiting for a gate', () => {
  it('says which gate it waits for, wherever a step says what it waits for', () => {
    const rate = node('fetch', 'WAITING', { waitingOn: 'rate_limit' });
    const permitted = node('notify', 'WAITING', { waitingOn: 'concurrency' });
    expect(waitingWords(rate)).toBe('Waiting for a rate-limit slot');
    expect(waitingWords(permitted)).toBe('Waiting for a concurrency permit');
    expect(nodeSummary(rate)).toMatch(/waiting for a rate-limit slot$/);
  });

  it('is shown in the table of the run page', async () => {
    const { user } = await openRunPage({ graph: graphOf([node('fetch', 'WAITING', { waitingOn: 'rate_limit' })], [], { status: 'RUNNING' }) });
    await user.click(await screen.findByRole('tab', { name: 'Table' }));
    const row = await screen.findByRole('row', { name: /fetch/ });
    expect(row).toHaveTextContent('Waiting for a rate-limit slot');
  });
});
