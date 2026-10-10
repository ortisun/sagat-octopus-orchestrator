import { screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { apiError, bodyOf, emptyDiff, keyOf, mountEditor, putsOf, sampleIr, settle, urlOf } from '../../../test/workflowsHarness';
import type { Behaviour, MountOptions } from '../../../test/workflowsHarness';

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor({ view: 'list', ...options });
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
  return mounted;
}

const posts = (platform: { calls: { method: string; url: string; init: RequestInit }[] }) => platform.calls.filter((c) => c.method === 'POST' && c.url.endsWith('/publish'));
const sentBody = (call: { init: RequestInit } | undefined) => bodyOf<{ draftRevision: number; releaseNotes?: string }>(call?.init);

const richDiff = {
  from: { kind: 'version', number: 2 },
  to: { kind: 'draft', revision: 4 },
  identical: false,
  nodes: { added: ['audit'], removed: ['legacy'], changed: [{ id: 'pay', fields: ['activity', 'timeoutMs'] }] },
  edges: { added: [{ from: 'pay', to: 'audit', kind: 'next' }], removed: [{ from: 'pay', to: 'legacy', kind: 'next' }] },
  changes: [{ path: '/metadata', change: 'changed' }],
  breaking: [],
  attention: [],
};

const breakingDiff = {
  ...richDiff,
  breaking: [{ code: 'INPUT_PROPERTY_REMOVED', message: 'The input property "amount" was removed.', path: '/inputSchema/properties/amount' }],
  attention: [{ code: 'ACTIVITY_ADDED', message: 'The activity "audit.log" is new: a worker must serve it.', nodeId: 'audit' }],
};

describe('publishing', () => {
  it('saves the pending change first, then shows what will change against the active version', async () => {
    const order: string[] = [];
    const { user, platform } = await opened({
      timings: { validateMs: 15, autosaveMs: 60_000 },
      behaviour: { diff: () => (order.push('diff'), richDiff) },
      wrapFetch: (inner) => (url, init) => {
        if (init?.method === 'PUT') order.push('put');
        return inner(url, init);
      },
    });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    expect(order).toEqual(['put', 'diff']);
    expect(putsOf(platform)).toHaveLength(1);
    expect(dialog).toHaveTextContent('Comparing version 2 with the draft (revision 4)');
    expect(dialog).toHaveTextContent('Added (1): audit');
    expect(dialog).toHaveTextContent('Removed (1): legacy');
    expect(dialog).toHaveTextContent('Changed: pay (activity, timeoutMs)');
    expect(dialog).toHaveTextContent('Added: pay → audit (next)');
    expect(dialog).toHaveTextContent('Removed: pay → legacy (next)');
    expect(dialog).toHaveTextContent('/metadata changed');
    expect(dialog).toHaveTextContent('The platform found no breaking changes of the kinds it checks');
    expect(within(dialog).queryByRole('checkbox')).not.toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Publish' })).toBeEnabled();
    // The diff asks for the active version against the draft.
    const asked = platform.calls.find((c) => c.url.includes('/diff'));
    expect([...urlOf(asked!).searchParams.keys()]).toEqual([]);
  });

  it('publishes the revision it saved, with the release notes and an idempotency key, and says so', async () => {
    const { user, platform } = await opened({ behaviour: { diff: () => richDiff } });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.type(within(dialog).getByRole('textbox', { name: 'Release notes' }), 'Adds the audit step');
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    const [call] = posts(platform);
    expect(sentBody(call)).toEqual({ draftRevision: 1, releaseNotes: 'Adds the audit step' });
    expect(keyOf(call)).toMatch(/^[0-9a-f-]{36}$/);
    expect(await screen.findByText('Published version 1.', { selector: '.banner' })).toBeInTheDocument();
    expect(await screen.findByText('Published version 1.', { selector: '[aria-live="polite"] p' })).toBeInTheDocument();
    expect(screen.getByText('Active version 1')).toBeInTheDocument();
  });

  it('sends no release notes when there are none', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    await waitFor(() => expect(posts(platform)).toHaveLength(1));
    expect(sentBody(posts(platform)[0])).toEqual({ draftRevision: 1 });
  });

  it('limits the release notes to 2048 characters and counts them', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    const notes = await within(dialog).findByRole('textbox', { name: 'Release notes' });
    expect(notes).toHaveAttribute('maxlength', '2048');
    await user.click(notes);
    await user.paste('x'.repeat(10));
    expect(within(dialog).getByText(/10 of 2048 characters/)).toBeInTheDocument();
  });

  it('sets breaking changes apart, with words and a shape, and needs the box ticked before it publishes', async () => {
    const { user, platform } = await opened({ behaviour: { diff: () => breakingDiff } });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    const breaking = await within(dialog).findByRole('region', { name: /Breaking changes \(1\)/ });
    expect(breaking).toHaveTextContent('These changes can make callers or event publishers fail. Runs already started keep the version they started on.');
    expect(breaking).toHaveTextContent('The input property "amount" was removed.');
    expect(breaking.querySelector('[aria-hidden="true"]')).toHaveTextContent('⚠');
    const prep = within(dialog).getByRole('region', { name: /Needs preparation \(1\)/ });
    expect(prep).toHaveTextContent('The activity "audit.log" is new');
    expect(prep).toHaveTextContent('audit');

    const publish = within(dialog).getByRole('button', { name: 'Publish' });
    const ack = within(dialog).getByRole('checkbox', { name: 'I understand these changes break the contract' });
    expect(publish).toBeDisabled();
    expect(publish).toHaveAccessibleDescription(/Tick the box above/);
    await user.click(ack);
    expect(publish).toBeEnabled();
    await user.click(ack);
    expect(publish).toBeDisabled();
    expect(posts(platform)).toHaveLength(0);
    await user.click(ack);
    await user.click(publish);
    await waitFor(() => expect(posts(platform)).toHaveLength(1));
  });

  it('says when the draft is the same as the active version', async () => {
    const { user } = await opened({ behaviour: { diff: () => ({ ...emptyDiff(), identical: true }) } });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    expect(await screen.findByText('There are no differences.')).toBeInTheDocument();
  });

  it('compares with nothing when it is the first version', async () => {
    const { user } = await opened({
      behaviour: {
        diff: () => ({
          ...richDiff,
          from: { kind: 'empty' },
          nodes: { added: ['check', 'pay', 'done'], removed: [], changed: [] },
          edges: { added: [], removed: [] },
          changes: [],
        }),
      },
    });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    expect(await screen.findByText(/Comparing/)).toHaveTextContent('nothing (no version yet) with the draft (revision 4)');
  });

  it('shows the reason the platform gives when it refuses, keeps the dialog, and retries with the same key', async () => {
    let refusals = 1;
    const { user, platform } = await opened({
      behaviour: {
        publish: () => (refusals-- > 0 ? apiError(422, 'VALIDATION_FAILED', 'Node "x" cannot be published: the platform does not run it yet.', '5'.repeat(32)) : undefined),
      },
    });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveTextContent('Node "x" cannot be published');
    expect(alert).toHaveTextContent('5'.repeat(32));
    expect(dialog).toBeVisible();
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(posts(platform)).toHaveLength(2);
    expect(keyOf(posts(platform)[1])).toBe(keyOf(posts(platform)[0]));
  });

  it('uses a new idempotency key when the release notes changed between tries', async () => {
    let refusals = 1;
    const { user, platform } = await opened({ behaviour: { publish: () => (refusals-- > 0 ? apiError(503, 'UNAVAILABLE', 'busy') : undefined) } });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    await within(dialog).findByRole('alert');
    await user.type(within(dialog).getByRole('textbox', { name: 'Release notes' }), 'now with notes');
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    await waitFor(() => expect(posts(platform)).toHaveLength(2));
    expect(keyOf(posts(platform)[1])).not.toBe(keyOf(posts(platform)[0]));
  });

  it('shows a failure to compare, with a way to try again, and publishes nothing', async () => {
    let failures = 1;
    const { user, platform } = await opened({ behaviour: { diff: () => (failures-- > 0 ? apiError(500, 'INTERNAL', 'diff failed', '4'.repeat(32)) : richDiff) } });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('diff failed');
    expect(within(dialog).getByRole('button', { name: 'Publish' })).toBeDisabled();
    await user.click(within(dialog).getByRole('button', { name: 'Try again' }));
    await within(dialog).findByText(/Comparing/);
    expect(posts(platform)).toHaveLength(0);
    expect(within(dialog).getByRole('button', { name: 'Publish' })).toBeEnabled();
  });

  it('shows a failure to save before publishing, and does not compare or publish', async () => {
    const { user, platform } = await opened({
      timings: { validateMs: 15, autosaveMs: 60_000 },
      wrapFetch: (inner) => (url, init) => (init?.method === 'PUT' ? Promise.resolve(apiError(500, 'INTERNAL', 'cannot save right now')) : inner(url, init)),
    });
    await user.click(screen.getByRole('button', { name: 'Delete done' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Publish…' })).not.toHaveAttribute('aria-disabled'));
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('Saving the draft failed.');
    expect(platform.calls.some((c) => c.url.includes('/diff'))).toBe(false);
    expect(posts(platform)).toHaveLength(0);
  });

  it('opens the conflict dialog, and publishes nothing, when the draft moved on under the author', async () => {
    const { user, platform } = await opened({
      behaviour: {
        diff: () => richDiff,
        publish: (_body, state) => {
          // Somebody saved between the diff and the publish.
          state.revision = 8;
          state.ir = { ...sampleIr(), nodes: [{ id: 'other', type: 'end' }], entrypoint: 'other' };
          return apiError(409, 'CONFLICT', 'The draft moved on.');
        },
      },
    });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    const conflict = await screen.findByRole('dialog', { name: 'Someone changed this draft' });
    expect(conflict).toHaveTextContent('revision 8');
    expect(screen.queryByRole('dialog', { name: 'Publish workflow' })).not.toBeInTheDocument();
    expect(posts(platform)).toHaveLength(1);
    expect(screen.getByTestId('save-state')).toHaveTextContent('Conflict');
  });

  it('looks again when the draft only moved to a newer revision with the same content', async () => {
    let published = 0;
    const { user, platform } = await opened({
      behaviour: {
        diff: () => richDiff,
        publish: (_body, state) => {
          if (published++ > 0) return undefined;
          // The same document, saved again: the revision moved, the content did not.
          state.revision += 3;
          return apiError(409, 'CONFLICT', 'The draft moved on.');
        },
      },
    });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    // The note is on screen while it saves and compares again, and stays above the new comparison.
    await waitFor(() => expect(within(dialog).getByText(/saved again by someone else with the same content \(revision 4\)/)).toBeInTheDocument());
    await within(dialog).findByText(/Comparing/);
    expect(screen.queryByRole('dialog', { name: 'Someone changed this draft' })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    await waitFor(() => expect(posts(platform)).toHaveLength(2));
    expect(sentBody(posts(platform)[1]).draftRevision).toBe(4);
  });

  it('gives focus back to the button that opened the dialog when it is cancelled', async () => {
    const { user } = await opened();
    const opener = screen.getByRole('button', { name: 'Publish…' });
    await user.click(opener);
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });

  it('shows the platform notes after publishing, and says when it was an existing version made active again', async () => {
    const { user } = await opened({
      behaviour: {
        publish: () => ({
          workflowId: 'w',
          versionId: 'v',
          versionNumber: 3,
          digest: 'sha256:' + 'a'.repeat(64),
          created: false,
          warnings: ['The workflow has no trigger of its own.'],
        }),
      },
    });
    await user.click(screen.getByRole('button', { name: 'Publish…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Publish workflow' });
    await within(dialog).findByText(/Comparing/);
    await user.click(within(dialog).getByRole('button', { name: 'Publish' }));
    const banner = await screen.findByText(/identical to version 3, which is active again/);
    expect(banner).toHaveTextContent('The workflow has no trigger of its own.');
  });

  it('has no Publish button for a user who may not publish', async () => {
    await mountEditor({ view: 'list', permissions: ['definition:read', 'definition:write'] });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    expect(screen.queryByRole('button', { name: 'Publish…' })).not.toBeInTheDocument();
    await settle(50);
  });
});

describe('the versions', () => {
  const doc = (n: number) => ({ ...sampleIr(), nodes: [...(sampleIr()['nodes'] as unknown[]), ...(n > 1 ? [{ id: 'extra', type: 'end' }] : [])] });
  const versions = [1, 2].map((n) => ({
    versionId: `ver-${n}`,
    workflowId: 'w',
    versionNumber: n,
    digest: 'sha256:' + String(n).repeat(64),
    publishedBy: 'alice',
    publishedAt: '2026-04-0' + n + 'T10:00:00Z',
    ...(n === 2 ? { releaseNotes: 'The second one' } : {}),
    document: doc(n),
  }));
  const options = (behaviour: Behaviour = {}): MountOptions => ({ view: 'list', state: { versions, activeVersion: 2 }, behaviour });

  async function openVersions(behaviour: Behaviour = {}) {
    const mounted = await mountEditor(options(behaviour));
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await mounted.user.click(screen.getByRole('tab', { name: 'Versions' }));
    await screen.findByRole('table', { name: /Published versions/ });
    return mounted;
  }

  it('lists the published versions, newest first, and marks the active one', async () => {
    await openVersions();
    const rows = within(screen.getByRole('table', { name: /Published versions/ })).getAllByRole('row');
    expect(rows).toHaveLength(3);
    expect(rows[1]).toHaveTextContent('v2');
    expect(rows[1]).toHaveTextContent('Active');
    expect(rows[1]).toHaveTextContent('alice');
    expect(rows[2]).toHaveTextContent('v1');
    expect(rows[2]).not.toHaveTextContent('Active');
  });

  it('says nothing is published yet', async () => {
    const mounted = await mountEditor({ view: 'list' });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    await mounted.user.click(screen.getByRole('tab', { name: 'Versions' }));
    expect(await screen.findByText(/Nothing is published yet/)).toBeInTheDocument();
  });

  it('shows one version read only: its nodes and its release notes, with nothing to edit', async () => {
    const { user } = await openVersions();
    await user.click(screen.getByRole('button', { name: 'View version 2' }));
    const view = await screen.findByRole('region', { name: 'Version 2' });
    await within(view).findByText(/The second one/);
    expect(within(view).getByRole('heading', { name: 'Version 2 (read only)' })).toBeInTheDocument();
    expect(within(view).getAllByRole('row')).toHaveLength(5);
    expect(within(view).queryByRole('button', { name: /^Delete/ })).not.toBeInTheDocument();
    expect(within(view).queryByRole('button', { name: /^Edit/ })).not.toBeInTheDocument();
    await user.click(within(view).getByRole('button', { name: 'Close version 2' }));
    expect(screen.queryByRole('region', { name: 'Version 2' })).not.toBeInTheDocument();
  });

  it('compares a version with the draft through the platform and shows it like the publish dialog does', async () => {
    const asked: string[] = [];
    const { user } = await openVersions({ diff: (url) => (asked.push(url), breakingDiff) });
    await user.click(screen.getByRole('button', { name: 'Compare version 1 with the draft' }));
    const comparison = await screen.findByRole('region', { name: 'Comparison' });
    await within(comparison).findByRole('region', { name: /Breaking changes \(1\)/ });
    expect(within(comparison).getByRole('region', { name: /Needs preparation \(1\)/ })).toBeInTheDocument();
    expect(new URL(asked[0]!, 'http://x').searchParams.get('from')).toBe('1');
    expect(new URL(asked[0]!, 'http://x').searchParams.get('to')).toBe('draft');
  });

  it('compares two versions chosen from the lists', async () => {
    const asked: string[] = [];
    const { user } = await openVersions({ diff: (url) => (asked.push(url), richDiff) });
    await user.selectOptions(screen.getByRole('combobox', { name: 'Compare' }), 'version 1');
    await user.selectOptions(screen.getByRole('combobox', { name: 'with' }), 'version 2');
    await user.click(screen.getByRole('button', { name: 'Show differences' }));
    await within(await screen.findByRole('region', { name: 'Comparison' })).findByText(/Added \(1\)/);
    const search = new URL(asked[0]!, 'http://x').searchParams;
    expect([search.get('from'), search.get('to')]).toEqual(['1', '2']);
  });

  it('says a comparison with the draft leaves out what is not saved yet', async () => {
    const { user } = await openVersions({ diff: () => richDiff });
    await user.click(screen.getByRole('button', { name: 'Compare version 1 with the draft' }));
    expect(await screen.findByText(/The draft here is the one the platform has saved \(revision 1\)/)).toBeInTheDocument();
  });

  it('shows a failure to compare', async () => {
    const { user } = await openVersions({ diff: () => apiError(404, 'NOT_FOUND', 'no such version', '3'.repeat(32)) });
    await user.click(screen.getByRole('button', { name: 'Compare version 1 with the draft' }));
    const alert = await screen.findByText(/no such version/);
    expect(alert.closest('[role="alert"]')).toHaveTextContent('3'.repeat(32));
  });
});
