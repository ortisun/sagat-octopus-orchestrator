import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { apiError, bodyOf, gate, json, keyOf, mountEditor, mountEditorInDataRouter, putsOf, sampleIr, settle } from '../../../test/workflowsHarness';
import type { Platform } from '../../../test/workflowsHarness';

const state = () => screen.getByTestId('save-state');

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor({ view: 'list', ...options });
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  return mounted;
}

const nodesOf = (put: { body: { ir: Record<string, unknown> } }) => (put.body.ir['nodes'] as { id: string }[]).map((n) => n.id);
const removeDone = async (user: Awaited<ReturnType<typeof opened>>['user']) => user.click(screen.getByRole('button', { name: 'Delete done' }));

afterEach(() => {
  vi.useRealTimers();
});

describe('autosave (D146)', () => {
  it('saves the draft after a quiet moment, on the revision it opened, with an idempotency key, and takes the new revision from the answer', async () => {
    const { user, platform } = await opened();
    await removeDone(user);
    expect(state()).toHaveTextContent('Unsaved changes');
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 2'));
    const [put] = putsOf(platform);
    expect(putsOf(platform)).toHaveLength(1);
    expect(put?.body.draftRevision).toBe(1);
    expect(nodesOf(put!)).toEqual(['check', 'pay']);
    expect(put?.key).toMatch(/^[0-9a-f-]{36}$/);
    // The next save is on the revision the answer gave.
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(2));
    expect(putsOf(platform)[1]?.body.draftRevision).toBe(2);
    expect(putsOf(platform)[1]?.key).not.toBe(put?.key);
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 3'));
  });

  it('waits 1.5 s of quiet, and a change inside the wait postpones it; the indicator says so', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { user, platform } = await opened({ fakeTimers: true, realTimings: true });
    await removeDone(user);
    await act(() => vi.advanceTimersByTimeAsync(1_000));
    expect(putsOf(platform)).toHaveLength(0);
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await act(() => vi.advanceTimersByTimeAsync(1_000));
    expect(putsOf(platform)).toHaveLength(0);
    expect(state()).toHaveTextContent('Unsaved changes');
    await act(() => vi.advanceTimersByTimeAsync(700));
    await waitFor(() => expect(putsOf(platform)).toHaveLength(1));
    expect(nodesOf(putsOf(platform)[0]!)).toEqual(['check']);
  });

  it('sends one save for a burst of changes', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 150 } });
    await removeDone(user);
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 2'));
    expect(putsOf(platform)).toHaveLength(1);
  });

  it('sends nothing for a change that was undone before the save', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 100 } });
    await removeDone(user);
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    await settle(300);
    expect(putsOf(platform)).toHaveLength(0);
    expect(state()).toHaveTextContent('Saved · revision 1');
  });

  it('saves at once with Save now, and says it is saved in a live region', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    await removeDone(user);
    await user.click(screen.getByRole('button', { name: 'Save now' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 2'));
    expect(putsOf(platform)).toHaveLength(1);
    await waitFor(() => expect(screen.getByText('Saved. Revision 2.')).toBeInTheDocument());
    expect(screen.getByText('Saved. Revision 2.').closest('[aria-live="polite"]')).not.toBeNull();
  });

  it('saves with Ctrl+S as well', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    await removeDone(user);
    await user.keyboard('{Control>}s{/Control}');
    await waitFor(() => expect(putsOf(platform)).toHaveLength(1));
  });

  it('says there is nothing to save when there is nothing', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    await user.click(screen.getByRole('button', { name: 'Save now' }));
    await waitFor(() => expect(screen.getByText('Nothing to save.')).toBeInTheDocument());
    expect(putsOf(platform)).toHaveLength(0);
  });

  it('saves when focus leaves the editor, without waiting for the quiet time', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    expect(putsOf(platform)).toHaveLength(0);
    await user.click(document.body);
    await waitFor(() => expect(putsOf(platform)).toHaveLength(1));
  });

  it('does not save while the focus only moves around inside the editor', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.click(screen.getByRole('textbox', { name: 'Activity' }));
    await settle(100);
    expect(putsOf(platform)).toHaveLength(0);
  });

  it('saves when the page is hidden', async () => {
    const { user, platform } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    await removeDone(user);
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    fireEvent(document, new Event('visibilitychange'));
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    await waitFor(() => expect(putsOf(platform)).toHaveLength(1));
  });

  it('shows a save that is on its way', async () => {
    const hold = gate<void>();
    const { user } = await opened({
      wrapFetch: (inner) => async (url, init) => {
        const answer = await inner(url, init);
        if (init?.method === 'PUT') await hold.opened;
        return answer;
      },
    });
    await removeDone(user);
    await waitFor(() => expect(state()).toHaveTextContent('Saving…'));
    await act(async () => {
      hold.release();
      await settle(10);
    });
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 2'));
  });

  it('keeps typing that arrives while a save is in flight, and saves it next', async () => {
    const hold = gate<void>();
    let puts = 0;
    const { user, platform } = await opened({
      wrapFetch: (inner) => async (url, init) => {
        const answer = await inner(url, init);
        if (init?.method === 'PUT' && ++puts === 1) await hold.opened;
        return answer;
      },
    });
    await removeDone(user);
    await waitFor(() => expect(puts).toBe(1));
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await act(async () => {
      hold.release();
      await settle(10);
    });
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 3'));
    expect(putsOf(platform).map((p) => p.body.draftRevision)).toEqual([1, 2]);
    expect(nodesOf(putsOf(platform)[1]!)).toEqual(['check']);
  });
});

describe('a save that fails', () => {
  const failFirst = (count: number, response: () => Response) => {
    let seen = 0;
    return {
      calls: () => seen,
      wrap:
        (inner: typeof fetch): typeof fetch =>
        (url, init) =>
          init?.method === 'PUT' && ++seen <= count ? Promise.resolve(response()) : inner(url, init),
    };
  };

  it('says it could not save, with the trace id, and offers Save now', async () => {
    const f = failFirst(1, () => apiError(503, 'UNAVAILABLE', 'The platform is busy.', '7'.repeat(32)));
    const { user } = await opened({ wrapFetch: f.wrap });
    await removeDone(user);
    await waitFor(() => expect(state()).toHaveTextContent(`Could not save (trace ${'7'.repeat(32)})`));
    expect(state().closest('.wf-save')).toHaveAttribute('data-state', 'failed');
    await user.click(screen.getByRole('button', { name: 'Save now' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 2'));
  });

  it('does not try again by itself in a loop: it waits for another change or for Save now', async () => {
    const f = failFirst(100, () => apiError(500, 'INTERNAL', 'boom'));
    const { user, platform } = await opened({ wrapFetch: f.wrap, timings: { validateMs: 15, autosaveMs: 30 } });
    await removeDone(user);
    await waitFor(() => expect(state()).toHaveTextContent('Could not save'));
    await settle(300);
    expect(f.calls()).toBe(1);
    // Another change is a reason to try again.
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(f.calls()).toBe(2));
    expect(putsOf(platform)).toHaveLength(0);
  });

  it('retries the same save with the same idempotency key, and a changed document with a new one', async () => {
    const seen: { key: string | null; revision: number; nodes: number }[] = [];
    let n = 0;
    const { user } = await opened({
      timings: { validateMs: 15, autosaveMs: 30 },
      wrapFetch: (inner) => (url, init) => {
        if (init?.method === 'PUT') {
          const body = bodyOf<{ draftRevision: number; ir: { nodes: unknown[] } }>(init);
          seen.push({ key: keyOf({ init }) ?? null, revision: body.draftRevision, nodes: body.ir.nodes.length });
          if (++n <= 2) return Promise.resolve(apiError(503, 'UNAVAILABLE', 'busy'));
        }
        return inner(url, init);
      },
    });
    await removeDone(user);
    await waitFor(() => expect(state()).toHaveTextContent('Could not save'));
    await user.click(screen.getByRole('button', { name: 'Save now' }));
    await waitFor(() => expect(seen).toHaveLength(2));
    await waitFor(() => expect(state()).toHaveTextContent('Could not save'));
    // A different document: a new key.
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 2'));
    expect(seen).toHaveLength(3);
    expect(seen[0]?.key).toBeTruthy();
    expect(seen[1]?.key).toBe(seen[0]?.key);
    expect(seen[2]?.key).not.toBe(seen[0]?.key);
    expect(seen.map((s) => s.revision)).toEqual([1, 1, 1]);
    expect(seen.map((s) => s.nodes)).toEqual([2, 2, 1]);
  });

  it('does not lose its own save when the answer is lost: the retry that follows is recognised, not reported as a conflict', async () => {
    let first = true;
    const { user, platform } = await opened({
      timings: { validateMs: 15, autosaveMs: 30 },
      wrapFetch: (inner) => async (url, init) => {
        if (init?.method === 'PUT' && first) {
          first = false;
          await inner(url, init);
          throw new TypeError('network down');
        }
        return inner(url, init);
      },
    });
    await removeDone(user);
    await waitFor(() => expect(state()).toHaveTextContent('Could not save'));
    expect(platform.state.revision).toBe(2);
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 3'));
    expect(screen.queryByRole('dialog', { name: 'Someone changed this draft' })).not.toBeInTheDocument();
    expect(nodesOf(putsOf(platform).at(-1)!)).toEqual(['check']);
  });
});

describe('two people editing the same draft (D146)', () => {
  const theirs = () => ({ ...sampleIr(), nodes: [...(sampleIr()['nodes'] as { id: string }[]).filter((n) => n.id !== 'pay'), { id: 'audit', type: 'end' }], entrypoint: 'check' });
  const conflicting = (platform: Platform) => {
    platform.state.revision = 5;
    platform.state.ir = theirs();
  };

  async function inConflict() {
    const mounted = await opened();
    // Somebody else saves while this editor still holds revision 1.
    conflicting(mounted.platform);
    await mounted.user.click(screen.getByRole('button', { name: 'Delete done' }));
    const dialog = await screen.findByRole('dialog', { name: 'Someone changed this draft' });
    return { ...mounted, dialog };
  }

  it('keeps the author copy, loads the other and shows both before anything is decided', async () => {
    const { dialog, platform } = await inConflict();
    expect(state()).toHaveTextContent('Conflict');
    expect(dialog).toHaveTextContent('your changes were not saved');
    expect(dialog).toHaveTextContent('revision 5');
    // The differences are in front of the author: their draft against yours.
    expect(within(dialog).getByText(/Comparing/)).toHaveTextContent('their draft with your draft');
    expect(dialog).toHaveTextContent('Added (1):');
    expect(dialog).toHaveTextContent('pay');
    expect(dialog).toHaveTextContent('audit');
    // The classification is the platform's; the Studio does not claim nothing breaks.
    expect(dialog).not.toHaveTextContent('no breaking changes');
    expect(dialog).toHaveTextContent('judged by the platform when the draft is published');
    for (const name of ['Take theirs', 'Keep mine and save over theirs', 'Save mine as a copy']) expect(within(dialog).getByRole('button', { name })).toBeEnabled();
    // Nothing was overwritten: the only save that went out was refused.
    expect(platform.state.revision).toBe(5);
    expect(platform.state.ir).toEqual(theirs());
    // The author's copy is still in the editor.
    expect(screen.queryByTestId('row-done')).not.toBeInTheDocument();
    expect(screen.getByTestId('row-pay')).toBeInTheDocument();
  });

  it('does not try to save again while the conflict is open', async () => {
    const { platform } = await inConflict();
    await settle(250);
    expect(platform.state.revision).toBe(5);
    expect(platform.calls.filter((c) => c.method === 'PUT')).toHaveLength(1);
  });

  it('Take theirs discards the author copy and continues from their draft, saved', async () => {
    const { user, dialog, platform } = await inConflict();
    await user.click(within(dialog).getByRole('button', { name: 'Take theirs' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Someone changed this draft' })).not.toBeInTheDocument());
    expect(state()).toHaveTextContent('Saved · revision 5');
    expect(screen.getByTestId('row-audit')).toBeInTheDocument();
    expect(screen.queryByTestId('row-pay')).not.toBeInTheDocument();
    await settle(150);
    expect(platform.calls.filter((c) => c.method === 'PUT')).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Undo' })).toBeDisabled();
  });

  it('Keep mine saves over theirs, on top of their revision, with a key of its own', async () => {
    const { user, dialog, platform } = await inConflict();
    await user.click(within(dialog).getByRole('button', { name: 'Keep mine and save over theirs' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 6'));
    const puts = putsOf(platform);
    expect(puts.map((p) => p.body.draftRevision)).toEqual([1, 5]);
    expect(nodesOf(puts[1]!)).toEqual(['check', 'pay']);
    expect(puts[1]?.key).not.toBe(puts[0]?.key);
    expect(screen.queryByRole('dialog', { name: 'Someone changed this draft' })).not.toBeInTheDocument();
  });

  it('Keep mine shows the new differences when somebody saved again in the meantime', async () => {
    const { user, dialog, platform } = await inConflict();
    // A third save lands before the author decides.
    platform.state.revision = 9;
    await user.click(within(dialog).getByRole('button', { name: 'Keep mine and save over theirs' }));
    await waitFor(() => expect(screen.getByRole('dialog', { name: 'Someone changed this draft' })).toHaveTextContent('revision 9'));
    expect(platform.state.revision).toBe(9);
    expect(state()).toHaveTextContent('Conflict');
  });

  const created = { id: '33333333-3333-4333-8333-333333333333', name: 'order-flow-copy', draftRevision: 1, ir: {} };

  it('Save mine as a copy creates name-copy with the author document and continues from theirs', async () => {
    const posts: { name: string; ir: { metadata: { name: string }; nodes: { id: string }[] } }[] = [];
    const { user, platform } = await opened({
      behaviour: {
        extra: {
          'POST /workflows': (init) => {
            posts.push(bodyOf(init));
            return json(created, 201);
          },
        },
      },
    });
    conflicting(platform);
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    const dialog = await screen.findByRole('dialog', { name: 'Someone changed this draft' });
    await user.click(within(dialog).getByRole('button', { name: 'Save mine as a copy' }));
    expect(await screen.findByText(/was saved as a new workflow/)).toHaveTextContent('order-flow-copy');
    expect(posts).toHaveLength(1);
    expect(posts[0]?.name).toBe('order-flow-copy');
    expect(posts[0]?.ir.metadata.name).toBe('order-flow-copy');
    expect(posts[0]?.ir.nodes.map((n) => n.id)).toEqual(['check', 'pay']);
    expect(screen.getByRole('link', { name: 'Open order-flow-copy' })).toHaveAttribute('href', `/workflows/${created.id}`);
    // This editor now holds their draft, saved; the original was not touched.
    expect(state()).toHaveTextContent('Saved · revision 5');
    expect(screen.getByTestId('row-audit')).toBeInTheDocument();
    expect(platform.state.ir).toEqual(theirs());
    await user.click(screen.getByRole('button', { name: 'Stay here' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('tries the next name when the copy name is taken', async () => {
    const names: string[] = [];
    const { user, platform } = await opened({
      behaviour: {
        extra: {
          'POST /workflows': (init) => {
            const name = bodyOf<{ name: string }>(init).name;
            names.push(name);
            return names.length === 1 ? apiError(409, 'CONFLICT', 'exists') : json({ ...created, name }, 201);
          },
        },
      },
    });
    conflicting(platform);
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    const dialog = await screen.findByRole('dialog', { name: 'Someone changed this draft' });
    await user.click(within(dialog).getByRole('button', { name: 'Save mine as a copy' }));
    await screen.findByText(/was saved as a new workflow/);
    expect(names).toEqual(['order-flow-copy', 'order-flow-copy-2']);
  });

  it('keeps the dialog and the author copy when the copy cannot be made, and says why', async () => {
    const { user, platform } = await opened({ behaviour: { extra: { 'POST /workflows': () => apiError(403, 'FORBIDDEN', 'You may not create workflows.', '6'.repeat(32)) } } });
    conflicting(platform);
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    const dialog = await screen.findByRole('dialog', { name: 'Someone changed this draft' });
    await user.click(within(dialog).getByRole('button', { name: 'Save mine as a copy' }));
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('You may not create workflows.');
    expect(screen.queryByTestId('row-done')).not.toBeInTheDocument();
    expect(screen.getByTestId('row-pay')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Take theirs' })).toBeEnabled();
  });

  it('can be put off: the author copy stays, saving stays off, and the conflict can be reopened', async () => {
    const { user, dialog, platform } = await inConflict();
    await user.click(within(dialog).getByRole('button', { name: 'Decide later' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Someone changed this draft' })).not.toBeInTheDocument());
    expect(state()).toHaveTextContent('Conflict');
    await user.click(screen.getByRole('button', { name: 'Delete pay' }));
    await settle(200);
    expect(platform.calls.filter((c) => c.method === 'PUT')).toHaveLength(1);
    await user.click(screen.getByRole('button', { name: 'Resolve conflict…' }));
    expect(await screen.findByRole('dialog', { name: 'Someone changed this draft' })).toBeInTheDocument();
  });

  it('blocks publishing until the conflict is resolved', async () => {
    const { user, dialog } = await inConflict();
    await user.click(within(dialog).getByRole('button', { name: 'Decide later' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Publish…' })).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByRole('button', { name: 'Publish…' })).toHaveAccessibleDescription(/conflict/);
  });

  it('is no conflict at all when the other draft says exactly what is being saved', async () => {
    const { user, platform } = await opened();
    // What the editor will hold after "done" is deleted: the links to it are gone with it.
    const mine = {
      ...sampleIr(),
      nodes: [
        { id: 'check', type: 'condition', expression: 'input.amount > 10', ifTrue: 'pay' },
        { id: 'pay', type: 'activity', activity: 'payment.submit', timeoutMs: 30_000 },
      ],
    };
    platform.state.revision = 7;
    platform.state.ir = mine;
    await removeDone(user);
    await waitFor(() => expect(state()).toHaveTextContent('Saved · revision 7'));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('leaving the page with unsaved changes', () => {
  it('asks before leaving, and Stay keeps the editor as it was', async () => {
    const { user, router, platform } = await mountEditorInDataRouter({ view: 'list', timings: { validateMs: 15, autosaveMs: 60_000 } });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await act(() => router.navigate('/workflows'));
    const dialog = await screen.findByRole('dialog', { name: 'Leave without saving?' });
    await user.click(within(dialog).getByRole('button', { name: 'Stay' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(router.state.location.pathname).toBe(`/workflows/11111111-1111-4111-8111-111111111111`);
    expect(screen.queryByTestId('row-done')).not.toBeInTheDocument();
    expect(putsOf(platform)).toHaveLength(0);
  });

  it('Leave without saving goes on without saving', async () => {
    const { user, router, platform } = await mountEditorInDataRouter({ view: 'list', timings: { validateMs: 15, autosaveMs: 60_000 } });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await act(() => router.navigate('/workflows'));
    await user.click(await screen.findByRole('button', { name: 'Leave without saving' }));
    expect(await screen.findByText('the list of workflows')).toBeInTheDocument();
    expect(putsOf(platform)).toHaveLength(0);
  });

  it('Save and leave saves first, and goes on once it is saved', async () => {
    const { user, router, platform } = await mountEditorInDataRouter({ view: 'list', timings: { validateMs: 15, autosaveMs: 60_000 } });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await act(() => router.navigate('/workflows'));
    await user.click(await screen.findByRole('button', { name: 'Save and leave' }));
    expect(await screen.findByText('the list of workflows')).toBeInTheDocument();
    expect(putsOf(platform)).toHaveLength(1);
  });

  it('stays, and says so, when the save before leaving fails', async () => {
    const { user, router } = await mountEditorInDataRouter({
      view: 'list',
      timings: { validateMs: 15, autosaveMs: 60_000 },
      wrapFetch: (inner) => (url, init) => (init?.method === 'PUT' ? Promise.resolve(apiError(500, 'INTERNAL', 'boom')) : inner(url, init)),
    });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await act(() => router.navigate('/workflows'));
    await user.click(await screen.findByRole('button', { name: 'Save and leave' }));
    expect(await screen.findByText(/could not be saved, so you are still here/)).toBeInTheDocument();
    expect(router.state.location.pathname).toContain('/workflows/1111');
  });

  it('lets go without a question when everything is saved', async () => {
    const { router } = await mountEditorInDataRouter({ view: 'list' });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await act(() => router.navigate('/workflows'));
    expect(await screen.findByText('the list of workflows')).toBeInTheDocument();
  });

  it('asks the browser to warn before the tab is closed while something is unsaved, and not otherwise', async () => {
    const { user } = await opened({ timings: { validateMs: 15, autosaveMs: 60_000 } });
    const before = () => {
      const event = new Event('beforeunload', { cancelable: true });
      window.dispatchEvent(event);
      return event.defaultPrevented;
    };
    expect(before()).toBe(false);
    await removeDone(user);
    expect(before()).toBe(true);
    await user.click(screen.getByRole('button', { name: 'Save now' }));
    await waitFor(() => expect(state()).toHaveTextContent('Saved'));
    expect(before()).toBe(false);
  });
});
