import { screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { BASE, ID, json, mountSchedules, schedule } from '../../../test/schedules';
import { SchedulesPage } from './SchedulesPage';

const mount = (options: { permissions?: string[]; routes?: Record<string, (init: RequestInit, url: string) => unknown> } = {}) =>
  mountSchedules(<SchedulesPage />, { path: '/schedules', route: '/schedules', ...options });

describe('The list of schedules', () => {
  it('names each schedule, its workflow, its rule and the next time, in words', async () => {
    await mount({
      routes: {
        [`GET ${BASE}`]: () => ({
          items: [
            schedule(),
            schedule({ scheduleId: '0194f5e2-7b1c-7000-8000-0000000000c4', name: 'weekly', spec: { cron: ['0 9 * * 1'], timezone: 'Europe/Lisbon', jitterMs: 0 }, policies: { overlap: 'BUFFER_ONE', catchupWindowMs: 1, catchupMode: 'ONE', pauseOnFailure: false, bufferLimit: 1 } }),
          ],
          nextCursor: null,
        }),
      },
    });
    const table = await screen.findByRole('table', { name: 'Schedules' });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(2);
    expect(within(rows[0]!).getByRole('link', { name: 'nightly' })).toHaveAttribute('href', `/schedules/${ID}`);
    expect(within(rows[0]!).getByText('Active')).toBeVisible();
    expect(within(rows[0]!).getByText('nightly-report')).toBeVisible();
    expect(within(rows[0]!).getByText('every 2 hours (UTC)')).toBeVisible();
    expect(within(rows[0]!).getByText('Skip')).toBeVisible();
    expect(rows[0]!.querySelector('time')).toHaveAttribute('datetime', '2026-11-05T02:00:00Z');
    expect(within(rows[1]!).getByText('cron 0 9 * * 1 (Europe/Lisbon)')).toBeVisible();
    expect(within(rows[1]!).getByText('Keep one')).toBeVisible();
  });

  it('shows no next time for a schedule that is paused or has no action left', async () => {
    await mount({
      routes: {
        [`GET ${BASE}`]: () => ({
          items: [schedule({ name: 'held', state: 'PAUSED' }), schedule({ scheduleId: '0194f5e2-7b1c-7000-8000-0000000000c4', name: 'spent', exhausted: true })],
          nextCursor: null,
        }),
      },
    });
    const table = await screen.findByRole('table', { name: 'Schedules' });
    const rows = within(table).getAllByRole('row').slice(1);
    for (const row of rows) expect(row.querySelector('time')).toBeNull();
    expect(within(rows[0]!).getByText('Paused')).toBeVisible();
    expect(within(rows[1]!).getByText('Exhausted')).toBeVisible();
  });

  it('pages: the second page is asked for with the cursor of the first', async () => {
    let calls = 0;
    const { user, platform } = await mount({
      routes: {
        [`GET ${BASE}`]: () => (calls++ === 0 ? { items: [schedule()], nextCursor: 'next-1' } : { items: [schedule({ scheduleId: '0194f5e2-7b1c-7000-8000-0000000000c4', name: 'second' })], nextCursor: null }),
      },
    });
    await screen.findByRole('link', { name: 'nightly' });
    await user.click(screen.getByRole('button', { name: 'Load more' }));
    await screen.findByRole('link', { name: 'second' });
    expect(platform.calls.some((call) => call.url.includes('cursor=next-1'))).toBe(true);
    expect(screen.queryByRole('button', { name: 'Load more' })).not.toBeInTheDocument();
  });

  it('asks the platform for the state a person chooses, from the first page, and shows what it answered', async () => {
    let calls = 0;
    const { user, platform } = await mount({
      routes: {
        [`GET ${BASE}`]: (_init, url) => {
          calls++;
          return url.includes('state=PAUSED')
            ? { items: [schedule({ name: 'held', state: 'PAUSED' })], nextCursor: null }
            : { items: [schedule(), schedule({ scheduleId: '0194f5e2-7b1c-7000-8000-0000000000c4', name: 'second' })], nextCursor: 'more' };
        },
      },
    });
    await screen.findByRole('link', { name: 'second' });
    await user.click(screen.getByRole('button', { name: 'Load more' }));
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('cursor=more'))).toBe(true));
    await user.selectOptions(screen.getByLabelText('State'), 'PAUSED');
    expect(await screen.findByRole('link', { name: 'held' })).toBeVisible();
    expect(screen.queryByRole('link', { name: 'second' })).not.toBeInTheDocument();
    const asked = platform.calls.filter((call) => call.url.includes('state=PAUSED'));
    expect(asked.length).toBeGreaterThan(0);
    expect(asked.every((call) => !call.url.includes('cursor='))).toBe(true);
    expect(calls).toBeGreaterThan(2);
  });

  it('asks for the workflow a person chooses', async () => {
    const { user, platform } = await mount();
    await screen.findByRole('link', { name: 'nightly' });
    await user.selectOptions(await screen.findByLabelText('Workflow'), 'nightly-report');
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('workflowId=0194f5e2-7b1c-7000-8000-000000000001'))).toBe(true));
  });

  it('asks for a name once the person has stopped typing, not for every letter', async () => {
    const { user, platform } = await mount();
    await screen.findByRole('link', { name: 'nightly' });
    await user.type(screen.getByLabelText('Name starts with'), '  rep ');
    await waitFor(() => expect(platform.calls.some((call) => call.url.includes('name=rep'))).toBe(true));
    const names = platform.calls.map((call) => new URL(call.url, 'http://studio.test').searchParams.get('name')).filter((name) => name !== null);
    expect(names, 'one request for the word, trimmed').toEqual(['rep']);
  });

  it('says no schedule matches when filters leave none, and that there is none when they do not', async () => {
    const { user } = await mount({ routes: { [`GET ${BASE}`]: (_init, url) => (url.includes('state=') ? { items: [], nextCursor: null } : { items: [schedule()], nextCursor: null }) } });
    await screen.findByRole('link', { name: 'nightly' });
    await user.selectOptions(screen.getByLabelText('State'), 'ACTIVE');
    expect(await screen.findByText('No schedule matches these filters.')).toBeVisible();
    expect(screen.queryByText('No schedule yet.')).not.toBeInTheDocument();
  });

  it('says there is none yet', async () => {
    await mount({ routes: { [`GET ${BASE}`]: () => ({ items: [], nextCursor: null }) } });
    expect(await screen.findByText('No schedule yet.')).toBeVisible();
  });

  it('offers a new schedule only to somebody who may write one and start its runs', async () => {
    await mount();
    expect(await screen.findByRole('link', { name: 'New schedule' })).toHaveAttribute('href', '/schedules/new');
  });

  it.each([['schedule:read', 'run:start'], ['schedule:read', 'schedule:write']])('offers no new schedule with only %s and %s', async (...permissions) => {
    await mount({ permissions });
    await screen.findByRole('table', { name: 'Schedules' });
    expect(screen.queryByRole('link', { name: 'New schedule' })).not.toBeInTheDocument();
  });

  it('says what went wrong, with a way to try again', async () => {
    let fail = true;
    const { user } = await mount({
      routes: { [`GET ${BASE}`]: () => (fail ? json(503, { code: 'UNAVAILABLE', message: 'Try again soon.', retryable: true }) : { items: [schedule()], nextCursor: null }) },
    });
    expect(await screen.findByText(/Listing schedules failed/)).toBeVisible();
    fail = false;
    await user.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('link', { name: 'nightly' })).toBeVisible();
    expect(screen.queryByText(/Listing schedules failed/)).not.toBeInTheDocument();
  });

  it('is not shown, and asks nothing of the platform, without schedule:read', async () => {
    const { platform } = await mount({ permissions: ['namespace:read'] });
    expect(await screen.findByText(/schedule:read/)).toBeVisible();
    expect(platform.calls.filter((call) => call.url.includes('/schedules'))).toHaveLength(0);
  });
});
