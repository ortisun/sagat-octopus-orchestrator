import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { createApp } from '../../app/context';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { RUN_ID, failure, jsonResponse, signedInApp } from '../../../test/runs';
import type { PayloadDisclosure } from './PayloadDisclosure';
import { PayloadDisclosureView } from './PayloadDisclosure';

const READ = ['run:read'];
type Disclosed = Extract<PayloadDisclosure, { kind: 'disclosed' }>;

const disclosed = (over: Partial<Disclosed> = {}): Disclosed => ({
  runId: RUN_ID,
  runVersion: 17,
  selection: { source: 'input' },
  policy: 'classified-v1',
  caller: { disclosedLevels: ['public', 'internal'], revealPii: 'granted', revealed: false },
  kind: 'disclosed',
  schema: { carrier: 'inputSchema', versionNumber: 4, rules: 1 },
  sizeBytes: 120,
  truncated: true,
  tree: {
    kind: 'container',
    type: 'object',
    level: 'public',
    childCount: 4,
    omittedChildren: 1,
    children: [
      { ordinal: 0, name: 'orderId', node: { kind: 'value', type: 'string', level: 'public', value: 'order-7' } },
      { ordinal: 1, name: 'amount', node: { kind: 'value', type: 'number', level: 'internal', value: 42 } },
      { ordinal: 2, name: 'customer', node: { kind: 'withheld', type: 'object', level: 'pii', reason: 'REVEAL_NOT_REQUESTED' } },
    ],
  },
  ...over,
});

async function mounted(answer: PayloadDisclosure | ((body: Record<string, unknown>) => PayloadDisclosure) = disclosed()) {
  const bodies: Record<string, unknown>[] = [];
  const platform = fakePlatform({
    [`POST /runs/${RUN_ID}/payload-disclosures`]: (init) => {
      if (typeof init.body !== 'string') throw new Error('disclosure request has no JSON body');
      const body = JSON.parse(init.body) as Record<string, unknown>;
      bodies.push(body);
      return typeof answer === 'function' ? answer(body) : answer;
    },
  });
  const app = await signedInApp(platform.fetcher, READ);
  const view = renderApp(<PayloadDisclosureView runId={RUN_ID} selector={{ source: 'input' }} />, { app });
  return { ...view, app, platform, bodies, user: userEvent.setup() };
}

describe('classified payload disclosure', () => {
  it('is lazy and shows levels by words and shapes, withheld nodes and omitted children', async () => {
    const { user, platform, bodies } = await mounted();
    expect(platform.calls).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'View permitted values' }));

    const tree = await screen.findByRole('tree', { name: 'Classified payload values' });
    expect(tree).toHaveTextContent('○ public');
    expect(tree).toHaveTextContent('□ internal');
    expect(tree).toHaveTextContent('△ pii');
    expect(tree).toHaveTextContent('customer △ pii object [withheld: pii]');
    expect(screen.getByText(/1 children omitted by the disclosure limits/)).toBeInTheDocument();
    expect(within(tree).getByText(/order-7/, { selector: 'code' })).toBeInTheDocument();
    expect(bodies).toEqual([{ source: 'input' }]);
    expect(screen.queryByRole('button', { name: /copy|export|download/i })).not.toBeInTheDocument();
  });

  it('offers reveal only when the response grants it and sends reveal after keyboard confirmation', async () => {
    const revealed = disclosed({
      caller: { disclosedLevels: ['public', 'internal', 'pii'], revealPii: 'granted', revealed: true },
      tree: { kind: 'value', type: 'string', level: 'pii', value: 'personal-canary' },
    });
    const { user, bodies } = await mounted((body) => (body['reveal'] === undefined ? disclosed() : revealed));
    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    const reveal = await screen.findByRole('button', { name: 'Reveal personal data' });
    reveal.focus();
    await user.keyboard('{Enter}');
    const dialog = screen.getByRole('dialog', { name: 'Reveal personal data?' });
    expect(dialog).toHaveTextContent('audited request');
    within(dialog).getByRole('button', { name: 'Reveal personal data' }).focus();
    await user.keyboard('{Enter}');

    expect(await screen.findByText(/personal-canary/)).toBeInTheDocument();
    expect(bodies).toEqual([{ source: 'input' }, { source: 'input', reveal: ['pii'] }]);
    expect(screen.queryByRole('button', { name: 'Reveal personal data' })).not.toBeInTheDocument();
  });

  it('does not offer reveal when the response says it is not granted', async () => {
    const { user } = await mounted(disclosed({ caller: { disclosedLevels: ['public'], revealPii: 'not_granted', revealed: false } }));
    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    await screen.findByText('Classified values.');
    expect(screen.queryByRole('button', { name: 'Reveal personal data' })).not.toBeInTheDocument();
  });

  it('keeps values out of browser storage and the query cache, then destroys them when closed', async () => {
    const local = vi.spyOn(Storage.prototype, 'setItem');
    const open = globalThis.indexedDB ? vi.spyOn(globalThis.indexedDB, 'open') : undefined;
    const { user, app } = await mounted();
    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    expect(await screen.findByText(/order-7/)).toBeInTheDocument();
    expect(local).not.toHaveBeenCalled();
    if (open) expect(open).not.toHaveBeenCalled();
    expect(app.queries.getQueryCache().getAll()).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'Close values' }));
    expect(screen.queryByText(/order-7/)).not.toBeInTheDocument();
    local.mockRestore();
    open?.mockRestore();
  });

  it('shows closed unavailable and error states without reflecting protected detail', async () => {
    const unavailable: PayloadDisclosure = {
      runId: RUN_ID,
      runVersion: 17,
      selection: { source: 'input' },
      policy: 'classified-v1',
      caller: { disclosedLevels: [], revealPii: 'not_granted', revealed: false },
      kind: 'unavailable',
      reason: 'NOT_DISCLOSABLE',
    };
    const first = await mounted(unavailable);
    await first.user.click(screen.getByRole('button', { name: 'View permitted values' }));
    expect(await screen.findByText('This payload is not disclosable. No value or validation detail was returned.')).toBeInTheDocument();
    first.unmount();

    const platform = fakePlatform({
      [`POST /runs/${RUN_ID}/payload-disclosures`]: () => failure(503, 'UNAVAILABLE', 'The platform is not available right now.'),
    });
    const app = await signedInApp(platform.fetcher, READ);
    renderApp(<PayloadDisclosureView runId={RUN_ID} selector={{ source: 'input' }} />, { app });
    await userEvent.setup().click(screen.getByRole('button', { name: 'View permitted values' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Reading permitted payload values failed.');
    expect(alert).toHaveTextContent('The platform is not available right now.');
  });

  it('aborts and clears on run, event, tenant and session changes and ignores stale answers', async () => {
    const pending: Array<{ signal: AbortSignal; finish: (response: Response) => void }> = [];
    const fetcher = ((_url: string, init: RequestInit = {}) =>
      new Promise<Response>((resolve) => {
        pending.push({ signal: init.signal as AbortSignal, finish: resolve });
      })) as typeof fetch;
    const app = createApp({ fetch: fetcher });
    const context = { ...AUTH_CONTEXT, permissions: READ } as never;
    await app.session.signIn(TOKEN, () => Promise.resolve(context));
    app.namespace.set('payments');

    let changeRun!: () => void;
    let changeEvent!: () => void;
    function Harness() {
      const [run, setRun] = useState(RUN_ID);
      const [sequence, setSequence] = useState(1);
      changeRun = () => setRun(`${RUN_ID.slice(0, -1)}2`);
      changeEvent = () => setSequence(2);
      return <PayloadDisclosureView runId={run} selector={{ source: 'journal', sequence }} />;
    }
    renderApp(<Harness />, { app });
    const user = userEvent.setup();

    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    act(changeRun);
    await waitFor(() => expect(pending[0]?.signal.aborted).toBe(true));
    act(() => pending[0]?.finish(jsonResponse(disclosed())));
    expect(screen.queryByText(/order-7/)).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    act(changeEvent);
    await waitFor(() => expect(pending[1]?.signal.aborted).toBe(true));
    expect(screen.queryByText('Reading the classified projection…')).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    act(() => app.namespace.set('refunds'));
    await waitFor(() => expect(pending[2]?.signal.aborted).toBe(true));

    await user.click(screen.getByRole('button', { name: 'View permitted values' }));
    await act(async () => app.session.signIn(`${TOKEN}2`, () => Promise.resolve(context)));
    await waitFor(() => expect(pending[3]?.signal.aborted).toBe(true));
    expect(screen.queryByText(/order-7/)).not.toBeInTheDocument();
  });
});

it('refuses a numeric projection the browser cannot display exactly', async () => {
  const raw = JSON.stringify(disclosed({
    tree: { kind: 'value', type: 'number', level: 'public', value: 0 },
  })).replace('"value":0', '"value":9007199254740993');
  const platform = fakePlatform({
    [`POST /runs/${RUN_ID}/payload-disclosures`]: () => new Response(raw, {
      status: 200, headers: { 'content-type': 'application/json' },
    }),
  });
  const app = await signedInApp(platform.fetcher, READ);
  renderApp(<PayloadDisclosureView runId={RUN_ID} selector={{ source: 'input' }} />, { app });
  await userEvent.setup().click(screen.getByRole('button', { name: 'View permitted values' }));
  expect(await screen.findByText(/payload contains JSON numbers the Studio cannot display exactly/)).toBeInTheDocument();
  expect(screen.queryByRole('tree', { name: 'Classified payload values' })).not.toBeInTheDocument();
  expect(screen.queryByText('9007199254740992')).not.toBeInTheDocument();
});
