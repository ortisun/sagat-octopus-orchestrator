import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes, useLocation } from 'react-router';
import { describe, expect, it } from 'vitest';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { createApp } from '../../app/context';
import { StartRun, parseInput } from './StartRun';

const RUN_ID = '019b5d6e-0000-7000-8000-0000000000aa';
const WORKFLOWS = [
  { id: '019b5d6e-0000-7000-8000-000000000001', name: 'order-saga' },
  { id: '019b5d6e-0000-7000-8000-000000000002', name: 'refund' },
];

function Where() {
  const { pathname } = useLocation();
  return <output data-testid="where">{pathname}</output>;
}

async function mount({ permissions = ['run:start'], choices = WORKFLOWS, initialId, failures = 0 }: { permissions?: string[]; choices?: typeof WORKFLOWS; initialId?: string; failures?: number } = {}) {
  let remaining = failures;
  const platform = fakePlatform({
    'POST /runs': () => {
      if (remaining > 0) {
        remaining -= 1;
        return new Response(JSON.stringify({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }), { status: 503, headers: { 'content-type': 'application/json' } });
      }
      return new Response(JSON.stringify({ runId: RUN_ID }), { status: 202, headers: { 'content-type': 'application/json' } });
    },
  });
  const app = createApp({ fetch: platform.fetcher });
  await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions } as never));
  app.namespace.set('payments');
  renderApp(
    <Routes>
      <Route
        path="/"
        element={
          <>
            <StartRun choices={choices} {...(initialId ? { initialId } : {})} />
            <Where />
          </>
        }
      />
      <Route path="/runs/:runId" element={<Where />} />
    </Routes>,
    { app },
  );
  return { platform, user: userEvent.setup() };
}

const key = (platform: Awaited<ReturnType<typeof mount>>['platform'], index: number) => new Headers(platform.calls[index]!.init.headers).get('Idempotency-Key');
const bodyOf = (platform: Awaited<ReturnType<typeof mount>>['platform'], index: number) => JSON.parse(platform.calls[index]!.init.body as string) as unknown;

describe('parseInput', () => {
  it('takes a JSON object and nothing else', () => {
    expect(parseInput('{"orderId":"1"}')).toEqual({ input: { orderId: '1' } });
    expect(parseInput('{}')).toEqual({ input: {} });
    for (const text of ['{', '', '[]', 'null', '"a"', '12', 'true']) expect(parseInput(text)).toHaveProperty('problem');
  });

  it('counts bytes, not characters, against the 256 KiB the platform is sent', () => {
    expect(parseInput(JSON.stringify({ v: 'é'.repeat(140_000) }))).toEqual({ problem: 'The input must be at most 256 KiB.' });
    expect(parseInput(JSON.stringify({ v: 'a'.repeat(100_000) }))).toHaveProperty('input');
  });
});

describe('Start run', () => {
  it('is not offered to somebody without run:start, nor when no workflow has a published version', async () => {
    await mount({ permissions: ['run:read', 'definition:read'] });
    expect(screen.queryByRole('button', { name: /Start run/ })).not.toBeInTheDocument();
  });

  it('is not offered when there is nothing to start', async () => {
    await mount({ choices: [] });
    expect(screen.queryByRole('button', { name: /Start run/ })).not.toBeInTheDocument();
  });

  it('says what it will do and sends nothing until it is confirmed', async () => {
    const { user, platform } = await mount();
    await user.click(screen.getByRole('button', { name: 'Start run…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Start a run' });
    expect(within(dialog).getByText(/recorded with your name/)).toBeInTheDocument();
    expect(within(dialog).getAllByRole('option').map((o) => o.textContent)).toEqual(['order-saga', 'refund']);
    expect(platform.calls).toHaveLength(0);
  });

  it('starts the chosen workflow with the typed input and a key, and opens the run that was started', async () => {
    const { user, platform } = await mount();
    await user.click(screen.getByRole('button', { name: 'Start run…' }));
    await user.selectOptions(await screen.findByLabelText('Workflow'), 'refund');
    const field = screen.getByLabelText('Input (JSON object)');
    await user.clear(field);
    await user.click(field);
    await user.paste('{"orderId":"1234"}');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent(`/runs/${RUN_ID}`));
    expect(platform.calls).toHaveLength(1);
    expect(bodyOf(platform, 0)).toEqual({ workflowId: WORKFLOWS[1]!.id, input: { orderId: '1234' } });
    expect(key(platform, 0)).toBeTruthy();
  });

  it('refuses an input that is not a JSON object before asking the platform', async () => {
    const { user, platform } = await mount();
    await user.click(screen.getByRole('button', { name: 'Start run…' }));
    const field = await screen.findByLabelText('Input (JSON object)');
    for (const text of ['{', '[1]']) {
      await user.clear(field);
      await user.click(field);
      await user.paste(text);
      await user.click(screen.getByRole('button', { name: 'Start run' }));
      expect(await screen.findByRole('alert')).toHaveTextContent(/JSON object/);
    }
    expect(platform.calls).toHaveLength(0);
  });

  it('keeps the key when the same request is sent again after a failure, so it starts one run, and changes it when the input changes', async () => {
    const { user, platform } = await mount({ failures: 2 });
    await user.click(screen.getByRole('button', { name: 'Start run…' }));
    await screen.findByLabelText('Input (JSON object)');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await screen.findByText(/Starting the run/);
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(platform.calls).toHaveLength(2));
    expect(key(platform, 1)).toBe(key(platform, 0));
    const field = screen.getByLabelText('Input (JSON object)');
    await user.clear(field);
    await user.click(field);
    await user.paste('{"a":1}');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(platform.calls).toHaveLength(3));
    expect(key(platform, 2)).not.toBe(key(platform, 0));
  });

  it('in the row of a workflow, starts that workflow only and names it for assistive technology', async () => {
    await mount({ choices: [WORKFLOWS[0]!], initialId: WORKFLOWS[0]!.id });
    const button = screen.getByRole('button', { name: 'Start run… of order-saga' });
    expect(button).toHaveTextContent('Start run…');
  });
});
