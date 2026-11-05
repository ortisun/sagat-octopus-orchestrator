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

async function mount({
  permissions = ['run:start'],
  choices = WORKFLOWS,
  initialId,
  failures = 0,
  status = 202,
}: { permissions?: string[]; choices?: typeof WORKFLOWS; initialId?: string; failures?: number; status?: number } = {}) {
  let remaining = failures;
  const platform = fakePlatform({
    'POST /runs': () => {
      if (remaining > 0) {
        remaining -= 1;
        return new Response(JSON.stringify({ code: 'UNAVAILABLE', message: 'Try again.', retryable: true }), { status: 503, headers: { 'content-type': 'application/json' } });
      }
      return new Response(JSON.stringify({ runId: RUN_ID }), { status, headers: { 'content-type': 'application/json' } });
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

  it('rejects numeric tokens that cannot be preserved exactly', () => {
    for (const text of [
      '{"id":9007199254740993}',
      '{"nested":{"id":9007199254740993}}',
      '{"value":1e400}',
      '{"value":1.1234567890123456789}',
    ]) {
      expect(parseInput(text)).toEqual({ problem: 'Use strings for exact integers and decimals.' });
    }
  });

  it('accepts safe number boundaries and preserves exact numeric strings', () => {
    expect(parseInput('{"minimum":-9007199254740991,"maximum":9007199254740991}')).toEqual({
      input: { minimum: -9007199254740991, maximum: 9007199254740991 },
    });
    expect(parseInput('{"id":"9007199254740993","amount":"1.1234567890123456789"}')).toEqual({
      input: { id: '9007199254740993', amount: '1.1234567890123456789' },
    });
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

  it('refuses unsafe numeric input before asking the platform', async () => {
    const { user, platform } = await mount();
    await user.click(screen.getByRole('button', { name: 'Start run…' }));
    const field = await screen.findByLabelText('Input (JSON object)');
    await user.clear(field);
    await user.click(field);
    await user.paste('{"databaseParameter":{"id":9007199254740993}}');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Use strings for exact integers and decimals.');
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

describe('Start run with a business identifier', () => {
  const open = async (user: ReturnType<typeof userEvent.setup>) => {
    await user.click(screen.getByRole('button', { name: 'Start run…' }));
    await screen.findByRole('dialog', { name: 'Start a run' });
  };

  it('asks for none, and shows what to do with a held one only once there is an identifier', async () => {
    const { user } = await mount();
    await open(user);
    expect(screen.getByLabelText('Business identifier (optional)')).toHaveValue('');
    expect(screen.queryByLabelText('If an open run already has it')).not.toBeInTheDocument();
    await user.type(screen.getByLabelText('Business identifier (optional)'), 'order-1001');
    expect(screen.getByLabelText('If an open run already has it')).toHaveValue('FAIL');
    expect(screen.getByLabelText('If a finished run still remembers it')).toHaveValue('ALLOW_DUPLICATE');
  });

  it('sends the identifier and the policies that are not the defaults, trimmed', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.type(screen.getByLabelText('Business identifier (optional)'), '  order-1001  ');
    await user.selectOptions(screen.getByLabelText('If an open run already has it'), 'USE_EXISTING');
    await user.selectOptions(screen.getByLabelText('If a finished run still remembers it'), 'REJECT_DUPLICATE');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(platform.calls).toHaveLength(1));
    expect(bodyOf(platform, 0)).toEqual({
      workflowId: WORKFLOWS[0]!.id,
      input: {},
      businessId: 'order-1001',
      businessIdConflictPolicy: 'USE_EXISTING',
      businessIdReusePolicy: 'REJECT_DUPLICATE',
    });
  });

  it('sends no identifier field at all without one', async () => {
    const { user, platform } = await mount();
    await open(user);
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(platform.calls).toHaveLength(1));
    expect(bodyOf(platform, 0)).toEqual({ workflowId: WORKFLOWS[0]!.id, input: {} });
  });

  it('refuses an identifier that cannot be one, in words, before asking the platform', async () => {
    const { user, platform } = await mount();
    await open(user);
    const field = screen.getByLabelText('Business identifier (optional)');
    await user.type(field, 'has a space');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    expect(await screen.findByText(/no space, no accent/)).toBeVisible();
    expect(platform.calls).toHaveLength(0);
  });

  it('says nothing was started when the platform returned the open run that has the identifier', async () => {
    const { user, platform } = await mount({ status: 200 });
    await open(user);
    await user.type(screen.getByLabelText('Business identifier (optional)'), 'order-1001');
    await user.selectOptions(screen.getByLabelText('If an open run already has it'), 'USE_EXISTING');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(screen.getByTestId('where')).toHaveTextContent(`/runs/${RUN_ID}`));
    expect(platform.calls).toHaveLength(1);
    expect(await screen.findByText(/already has an open run/)).toBeInTheDocument();
  });

  it('keeps the key of an unchanged request and changes it with the identifier', async () => {
    const { user, platform } = await mount({ failures: 2 });
    await open(user);
    const field = screen.getByLabelText('Business identifier (optional)');
    await user.type(field, 'order-1');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await screen.findByText(/Starting the run/);
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(platform.calls).toHaveLength(2));
    expect(key(platform, 1)).toBe(key(platform, 0));
    // another identifier is another request, and a retry of the first must not answer for it
    await screen.findByText(/Starting the run/);
    await user.clear(field);
    await user.type(field, 'order-2');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    await waitFor(() => expect(platform.calls).toHaveLength(3));
    expect(key(platform, 2)).not.toBe(key(platform, 0));
  });

  it('shows the platform’s refusal of a held identifier', async () => {
    const platform = fakePlatform({
      'POST /runs': () =>
        new Response(JSON.stringify({ code: 'CONFLICT', message: 'another open execution holds this business identifier', retryable: false, details: { reason: 'BUSINESS_ID_IN_USE', runId: RUN_ID } }), {
          status: 409,
          headers: { 'content-type': 'application/json' },
        }),
    });
    const app = createApp({ fetch: platform.fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: ['run:start'] } as never));
    app.namespace.set('payments');
    renderApp(<StartRun choices={WORKFLOWS} />, { app });
    const user = userEvent.setup();
    await open(user);
    await user.type(screen.getByLabelText('Business identifier (optional)'), 'order-1001');
    await user.click(screen.getByRole('button', { name: 'Start run' }));
    expect(await screen.findByText(/Starting the run failed/)).toBeVisible();
    expect(screen.getByText('BUSINESS_ID_IN_USE', { selector: 'code' })).toBeVisible();
  });
});
