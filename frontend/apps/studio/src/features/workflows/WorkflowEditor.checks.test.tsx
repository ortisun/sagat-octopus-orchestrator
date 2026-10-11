import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { bodyOf, gate, json, mountEditor, putsOf, urlText, validationsOf } from '../../../test/workflowsHarness';

const problem = (over: Record<string, unknown> = {}) => ({ severity: 'error', code: 'SCHEMA', message: 'a problem', ...over });
const invalid = (...problems: Record<string, unknown>[]) => ({ valid: false, problems, warnings: [] });
const summary = () => screen.getByTestId('validation-summary');

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor({ view: 'list', ...options });
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  return mounted;
}

afterEach(() => {
  vi.useRealTimers();
});

describe('validation', () => {
  it('checks the document with the platform once when the editor opens, sending the document in the editor', async () => {
    const { platform } = await opened();
    await waitFor(() => expect(summary()).toHaveTextContent('0 problems · 0 warnings · 0 pieces of advice'));
    expect(validationsOf(platform)).toHaveLength(1);
    const body = bodyOf<{ ir: { metadata: { name: string }; nodes: unknown[] } }>(validationsOf(platform)[0]?.init);
    expect(body.ir.metadata.name).toBe('order-flow');
    expect(body.ir.nodes).toHaveLength(3);
  });

  it('says the summary in a polite live region that is always there, even when the panel is closed', async () => {
    const { user } = await opened({ behaviour: { validate: () => invalid(problem(), problem({ code: 'CYCLE' })) } });
    await waitFor(() => expect(summary()).toHaveTextContent('2 problems'));
    expect(summary()).toHaveAttribute('role', 'status');
    await user.click(screen.getByRole('button', { name: 'Validation and versions' }));
    expect(screen.queryByRole('region', { name: 'Validation' })).not.toBeInTheDocument();
    expect(summary()).toHaveTextContent('2 problems · 0 warnings');
  });

  it('asks again about the same document when the person says so, because the answer also depends on what exists', async () => {
    let n = 0;
    const { user, platform } = await opened({
      behaviour: { validate: () => (++n === 1 ? invalid(problem({ code: 'UNKNOWN_SCRIPT', message: 'the script artifact was not built for this tenant' })) : { valid: true, problems: [], warnings: [] }) },
    });
    await waitFor(() => expect(summary()).toHaveTextContent('1 problem'));
    expect(screen.getByRole('button', { name: 'Publish…' })).toHaveAttribute('aria-disabled', 'true');
    // The script is built meanwhile (somewhere else): the document is the same, and so is its key.
    await user.click(screen.getByRole('button', { name: 'Check again' }));
    await waitFor(() => expect(summary()).toHaveTextContent('0 problems'));
    expect(validationsOf(platform)).toHaveLength(2);
    expect(bodyOf(validationsOf(platform)[1]?.init)).toEqual(bodyOf(validationsOf(platform)[0]?.init));
    expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled');
  });

  it('asks again, once, when the person comes back to the window after a while, because the platform may know more by now', async () => {
    const { platform } = await opened();
    await waitFor(() => expect(validationsOf(platform)).toHaveLength(1));
    // Not yet: the answer is fresh.
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await new Promise((resolve) => setTimeout(resolve, 60));
    expect(validationsOf(platform)).toHaveLength(1);
    // A minute later the window comes back: it gains focus and becomes visible, which is one reason to ask, not two.
    const later = Date.now() + 60_000;
    const clock = vi.spyOn(Date, 'now').mockReturnValue(later);
    try {
      act(() => {
        window.dispatchEvent(new Event('focus'));
        document.dispatchEvent(new Event('visibilitychange'));
      });
      await waitFor(() => expect(validationsOf(platform)).toHaveLength(2));
      await new Promise((resolve) => setTimeout(resolve, 100));
      expect(validationsOf(platform)).toHaveLength(2);
    } finally {
      clock.mockRestore();
    }
  });

  it('waits 600 ms of quiet after a change before it asks again', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { user, platform } = await opened({ fakeTimers: true, realTimings: true });
    await waitFor(() => expect(validationsOf(platform)).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await act(() => vi.advanceTimersByTimeAsync(450));
    expect(validationsOf(platform)).toHaveLength(1);
    // Another change inside the quiet time starts the wait again.
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await act(() => vi.advanceTimersByTimeAsync(450));
    expect(validationsOf(platform)).toHaveLength(1);
    await act(() => vi.advanceTimersByTimeAsync(300));
    await waitFor(() => expect(validationsOf(platform)).toHaveLength(2));
    const body = bodyOf<{ ir: { nodes: { id: string }[] } }>(validationsOf(platform)[1]?.init);
    expect(body.ir.nodes.map((n) => n.id)).toEqual(['check']);
  });

  it('checks again for every change that sticks, not for a change that was undone before the wait ended', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 100, autosaveMs: 60_000 } });
    await waitFor(() => expect(validationsOf(platform)).toHaveLength(1));
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    await new Promise((r) => setTimeout(r, 250));
    // The document is the one that was already checked; nothing new to ask.
    expect(validationsOf(platform)).toHaveLength(1);
  });

  it('ignores an old answer that arrives after a newer one', async () => {
    const first = gate<Response>();
    let calls = 0;
    const { user } = await opened({
      timings: { validateMs: 10, autosaveMs: 60_000 },
      wrapFetch: (inner) => (url, init) => {
        if (init?.method === 'POST' && urlText(url).endsWith('/validate')) {
          calls++;
          // The second request (the first change) is the slow one.
          if (calls === 2) return first.opened;
          if (calls === 3) return Promise.resolve(json({ valid: true, problems: [], warnings: [] }));
        }
        return inner(url, init);
      },
    });
    await waitFor(() => expect(calls).toBe(1));
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await waitFor(() => expect(calls).toBe(2));
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(calls).toBe(3));
    await waitFor(() => expect(summary()).toHaveTextContent('0 problems'));
    // The slow, older answer now arrives, saying something else.
    await act(async () => {
      first.release(json({ valid: false, problems: [problem({ message: 'OLD NEWS' })], warnings: [] }));
      await new Promise((r) => setTimeout(r, 30));
    });
    expect(summary()).toHaveTextContent('0 problems');
    expect(screen.queryByText('OLD NEWS')).not.toBeInTheDocument();
  });

  it('cancels the request of an older document when a newer one is asked for', async () => {
    const signals: AbortSignal[] = [];
    const never = gate<Response>();
    let calls = 0;
    const { user } = await opened({
      timings: { validateMs: 10, autosaveMs: 60_000 },
      wrapFetch: (inner) => (url, init) => {
        if (init?.method === 'POST' && urlText(url).endsWith('/validate')) {
          calls++;
          if (init.signal) signals.push(init.signal);
          if (calls === 2) return never.opened;
        }
        return inner(url, init);
      },
    });
    await waitFor(() => expect(calls).toBe(1));
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await waitFor(() => expect(calls).toBe(2));
    expect(signals[1]?.aborted).toBe(false);
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(signals[1]?.aborted).toBe(true));
  });

  it('shows the problems of the platform, and a problem about a position in the document finds its node', async () => {
    const { user } = await opened({
      behaviour: {
        validate: () =>
          invalid(
            problem({ code: 'SCHEMA', message: 'missing activity', path: '/nodes/1/activity' }),
            problem({ code: 'CYCLE', message: 'there is a loop', nodeId: 'check' }),
            problem({ code: 'UNKNOWN_ENTRYPOINT', message: 'no first step', path: '/entrypoint' }),
          ),
      },
    });
    const panel = await screen.findByRole('region', { name: 'Validation' });
    await within(panel).findByText('missing activity');
    const list = within(panel).getAllByRole('list')[0]!;
    expect(within(list).getAllByRole('listitem')).toHaveLength(3);
    // The chips name the node; the one about the document has no node.
    await user.click(within(panel).getByRole('button', { name: 'Select node pay' }));
    expect(screen.getByRole('heading', { level: 2, name: 'Step: pay' })).toBeInTheDocument();
    expect(within(list).getByText('Workflow')).toBeInTheDocument();
    // The nodes with problems show it, in words, in the list.
    expect(screen.getByTestId('row-pay')).toHaveTextContent('1 problem');
    expect(screen.getByTestId('row-check')).toHaveTextContent('1 problem');
    expect(screen.getByTestId('row-done')).not.toHaveTextContent('problem');
  });

  it('shows the warnings of the platform apart from the problems', async () => {
    await opened({
      behaviour: {
        validate: () => ({ valid: true, problems: [], warnings: [{ severity: 'warning', code: 'RECURSIVE_CHILD', message: 'this child may start itself', nodeId: 'pay' }] }),
      },
    });
    const panel = await screen.findByRole('region', { name: 'Validation' });
    expect(await within(panel).findByRole('heading', { name: 'Warnings (1)' })).toBeInTheDocument();
    expect(within(panel).getByRole('heading', { name: 'Problems (0)' })).toBeInTheDocument();
    expect(within(panel).getByText('this child may start itself')).toBeInTheDocument();
    expect(summary()).toHaveTextContent('0 problems · 1 warning');
  });

  it('shows advice the Studio works out apart from the platform, labelled as not checked by it, and counts it in the summary', async () => {
    const { user } = await opened();
    await waitFor(() => expect(summary()).toHaveTextContent('0 pieces of advice'));
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Step' }));
    const panel = screen.getByRole('region', { name: 'Validation' });
    expect(within(panel).getByRole('heading', { name: /^Advice \(not checked by the platform\)/ })).toBeInTheDocument();
    await waitFor(() => expect(summary()).toHaveTextContent('0 problems · 0 warnings · 2 pieces of advice'));
    expect(within(panel).getByText(/"activity_1" has no timeout/)).toBeInTheDocument();
    expect(within(panel).getByText(/is not reachable from the first step/)).toBeInTheDocument();
    // Advice is on the node, in words, but is not a problem and blocks nothing.
    expect(screen.getByTestId('row-activity_1')).toHaveTextContent('2 advice');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
  });

  it('says it could not check, with the trace id, and does not pretend the document is valid', async () => {
    await opened({
      behaviour: {
        validate: () =>
          new Response(JSON.stringify({ code: 'INTERNAL', message: 'the validator fell over', traceId: '9'.repeat(32) }), {
            status: 500,
            headers: { 'content-type': 'application/json' },
          }),
      },
    });
    const panel = await screen.findByRole('region', { name: 'Validation' });
    const alert = await within(panel).findByRole('alert');
    expect(alert).toHaveTextContent('the validator fell over');
    expect(alert).toHaveTextContent('9'.repeat(32));
    expect(summary()).toHaveTextContent('');
    expect(screen.getByText('The platform could not check this just now.')).toBeInTheDocument();
  });

  it('can be closed and opened, and the state is in aria-expanded', async () => {
    const { user } = await opened();
    const toggle = screen.getByRole('button', { name: 'Validation and versions' });
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await user.click(toggle);
    expect(await screen.findByRole('region', { name: 'Validation' })).toBeInTheDocument();
  });
});

describe('under React StrictMode, which runs every effect twice in development', () => {
  it('still asks the platform once on load, and saves once for one change', async () => {
    const { user, platform } = await opened({ strict: true });
    await waitFor(() => expect(summary()).toHaveTextContent('0 problems'));
    expect(validationsOf(platform)).toHaveLength(1);
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await waitFor(() => expect(screen.getByTestId('save-state')).toHaveTextContent('Saved · revision 2'));
    expect(putsOf(platform)).toHaveLength(1);
    await waitFor(() => expect(validationsOf(platform)).toHaveLength(2));
  });
});

describe('publishing is gated on the validation', () => {
  it('is disabled while there are problems, says why, and takes the user to the problems when pressed', async () => {
    const { user } = await opened({ behaviour: { validate: () => invalid(problem(), problem({ code: 'CYCLE' })) } });
    await waitFor(() => expect(summary()).toHaveTextContent('2 problems'));
    const publish = screen.getByRole('button', { name: 'Publish…' });
    expect(publish).toHaveAttribute('aria-disabled', 'true');
    expect(publish).toHaveAccessibleDescription(/blocked: the platform found 2 problems/);
    await user.click(publish);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Validation' })).toHaveFocus();
  });

  it('can be reached from the reason with a button that shows the problems', async () => {
    const { user } = await opened({ behaviour: { validate: () => invalid(problem()) } });
    await waitFor(() => expect(summary()).toHaveTextContent('1 problem'));
    await user.click(screen.getByRole('button', { name: 'Validation and versions' }));
    await user.click(screen.getByRole('button', { name: 'Show the problems' }));
    expect(await screen.findByRole('region', { name: 'Validation' })).toHaveFocus();
  });

  it('waits for the check of the document in the editor before it opens', async () => {
    const slow = gate<Response>();
    let calls = 0;
    const { user } = await opened({
      timings: { validateMs: 10, autosaveMs: 60_000 },
      wrapFetch: (inner) => (url, init) => {
        if (init?.method === 'POST' && urlText(url).endsWith('/validate') && ++calls === 2) return slow.opened;
        return inner(url, init);
      },
    });
    await waitFor(() => expect(summary()).toHaveTextContent('0 problems'));
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).toHaveAttribute('aria-disabled', 'true'));
    expect(screen.getByRole('button', { name: 'Publish…' })).toHaveAccessibleDescription(/waits for the platform to finish checking/);
    await act(async () => {
      slow.release(json({ valid: true, problems: [], warnings: [] }));
      await new Promise((r) => setTimeout(r, 30));
    });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
  });

  it('is open when the document is valid', async () => {
    await opened();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
  });
});
