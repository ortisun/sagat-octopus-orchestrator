import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { createApp } from '../../app/context';
import { AUTH_CONTEXT, TOKEN, fakePlatform, renderApp } from '../../../test/render';
import { RUN_ID, jsonResponse, signedInApp } from '../../../test/runs';
import type { PayloadInspection as Inspection } from './PayloadInspection';
import { PayloadInspector } from './PayloadInspection';

const INSPECT = ['run:read', 'run:payload:inspect'];

const inline: Inspection = {
  runId: RUN_ID,
  runVersion: 17,
  selection: { source: 'input' },
  policy: 'structure-only-v1',
  kind: 'inline-json',
  sizeBytes: 93,
  truncated: true,
  tree: {
    type: 'object',
    childCount: 40,
    omittedChildren: 39,
    children: [{ type: 'string', redacted: true }],
  },
};

async function mounted(answer: Inspection = inline, permissions = INSPECT) {
  let body: unknown;
  const platform = fakePlatform({
    [`POST /runs/${RUN_ID}/payload-inspections`]: (init) => {
      body = JSON.parse(typeof init.body === 'string' ? init.body : 'null');
      return answer;
    },
  });
  const app = await signedInApp(platform.fetcher, permissions);
  const view = renderApp(<PayloadInspector runId={RUN_ID} selector={{ source: 'input' }} />, { app });
  return { ...view, app, platform, body: () => body, user: userEvent.setup() };
}

describe('redacted payload inspection', () => {
  it('is lazy and renders useful ordinal structure without a key, value, copy or export control', async () => {
    const { user, platform, body } = await mounted();
    expect(platform.calls).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'Inspect redacted payload' }));
    expect(await screen.findByText('Redacted structure only.')).toBeInTheDocument();
    expect(screen.getByRole('tree', { name: 'Redacted payload structure' })).toHaveTextContent('Root object 40 children');
    expect(screen.getByRole('tree')).toHaveTextContent('Field 1 string value hidden');
    expect(screen.getByText(/39 children omitted/)).toBeInTheDocument();
    expect(body()).toEqual({ source: 'input' });
    expect(document.body.textContent).not.toContain('ordinary-private-key');
    expect(document.body.textContent).not.toContain('ORDINARY-SECRET-CANARY');
    expect(screen.queryByRole('button', { name: /copy|export|download/i })).not.toBeInTheDocument();
  });

  it('does not offer inspection without the dedicated permission', async () => {
    const { platform } = await mounted(inline, ['run:read']);
    expect(screen.queryByRole('button', { name: /inspect redacted payload/i })).not.toBeInTheDocument();
    expect(platform.calls).toHaveLength(0);
  });

  it('shows external and unavailable states without pretending to inspect external bytes', async () => {
    const external: Inspection = {
      runId: RUN_ID,
      runVersion: 2,
      selection: { source: 'journal', sequence: 9 },
      nodeId: 'charge',
      attempt: 2,
      policy: 'structure-only-v1',
      kind: 'external',
      sizeBytes: 900_000,
      contentKind: 'json',
      reason: 'EXTERNAL_CONTENT_NOT_INSPECTED',
    };
    const first = await mounted(external);
    await first.user.click(screen.getByRole('button', { name: 'Inspect redacted payload' }));
    expect(await screen.findByText(/content itself was not inspected/)).toBeInTheDocument();
    first.unmount();

    const unavailable: Inspection = {
      runId: RUN_ID,
      runVersion: 3,
      selection: { source: 'journal', sequence: 11 },
      policy: 'structure-only-v1',
      kind: 'unavailable',
      reason: 'NO_RETAINED_PAYLOAD',
    };
    const second = await mounted(unavailable);
    await second.user.click(screen.getByRole('button', { name: 'Inspect redacted payload' }));
    expect(await screen.findByText('This event has no retained payload.')).toBeInTheDocument();
  });

  it('does not resurrect a loaded result after leaving and returning to the same scope', async () => {
    const { user, app, platform } = await mounted();
    await user.click(screen.getByRole('button', { name: 'Inspect redacted payload' }));
    expect(await screen.findByText('Redacted structure only.')).toBeInTheDocument();

    act(() => app.namespace.set('another-namespace'));
    expect(screen.queryByText('Redacted structure only.')).not.toBeInTheDocument();
    act(() => app.namespace.set('dev'));
    expect(screen.queryByText('Redacted structure only.')).not.toBeInTheDocument();
    expect(platform.calls).toHaveLength(1);
  });

  it('aborts and clears an in-flight result when namespace or session changes', async () => {
    let finish: ((response: Response) => void) | undefined;
    let aborted = false;
    const fetcher = ((_url: string, init: RequestInit = {}) =>
      new Promise<Response>((resolve, reject) => {
        finish = resolve;
        init.signal?.addEventListener('abort', () => {
          aborted = true;
          reject(new DOMException('aborted', 'AbortError'));
        });
      })) as typeof fetch;
    const app = createApp({ fetch: fetcher });
    await app.session.signIn(TOKEN, () => Promise.resolve({ ...AUTH_CONTEXT, permissions: INSPECT } as never));
    app.namespace.set('payments');
    renderApp(<PayloadInspector runId={RUN_ID} selector={{ source: 'input' }} />, { app });
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Inspect redacted payload' }));
    expect(screen.getByText('Reading the redacted structure…')).toBeInTheDocument();

    act(() => app.namespace.set('another-namespace'));
    await waitFor(() => expect(aborted).toBe(true));
    expect(screen.queryByText('Reading the redacted structure…')).not.toBeInTheDocument();
    await act(async () => {
      finish?.(jsonResponse(inline));
      await Promise.resolve();
    });
    expect(screen.queryByText('Redacted structure only.')).not.toBeInTheDocument();

    act(() => app.session.signOut());
    expect(screen.queryByRole('button', { name: /inspect redacted payload/i })).not.toBeInTheDocument();
  });

  it('aborts an in-flight request when the same subject starts a new session', async () => {
    let finish: ((response: Response) => void) | undefined;
    let aborted = false;
    const fetcher = ((_url: string, init: RequestInit = {}) =>
      new Promise<Response>((resolve, reject) => {
        finish = resolve;
        init.signal?.addEventListener('abort', () => {
          aborted = true;
          reject(new DOMException('aborted', 'AbortError'));
        });
      })) as typeof fetch;
    const app = createApp({ fetch: fetcher });
    const context = { ...AUTH_CONTEXT, permissions: INSPECT } as never;
    await app.session.signIn(TOKEN, () => Promise.resolve(context));
    app.namespace.set('payments');
    renderApp(<PayloadInspector runId={RUN_ID} selector={{ source: 'input' }} />, { app });
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Inspect redacted payload' }));

    await act(async () => app.session.signIn(`${TOKEN}2`, () => Promise.resolve(context)));
    await waitFor(() => expect(aborted).toBe(true));
    expect(screen.queryByText('Reading the redacted structure…')).not.toBeInTheDocument();
    await act(async () => {
      finish?.(jsonResponse(inline));
      await Promise.resolve();
    });
    expect(screen.queryByText('Redacted structure only.')).not.toBeInTheDocument();
  });
});
