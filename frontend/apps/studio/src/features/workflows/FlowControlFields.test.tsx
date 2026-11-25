import { readFileSync } from 'node:fs';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import type { IrDocument } from '../../model/ir';
import { canonical, readDocument } from '../../model/ir';
import { mountEditor, putsOf } from '../../../test/workflowsHarness';
import type { Platform } from '../../../test/workflowsHarness';
import { FlowControlFields } from './FlowControlFields';
import { WORKFLOW } from './flowControl';
import type { Owner } from './flowControl';

// Feature 022 (T022-054, T022-085): the fields of `concurrency` and `rateLimit`, reached and filled with the keyboard.

const FIXTURES = `${process.cwd()}/../../../tests/contract/flow-control-step-policies/fixtures`;
const fixture = (name: string): IrDocument => readDocument(JSON.parse(readFileSync(`${FIXTURES}/${name}`, 'utf8'))).doc;

/** The fields over a document held in state, the way the editor holds it; `latest` is the document after each change. */
function mount(initial: IrDocument, owner: Owner, disabled = false) {
  const seen: { latest: IrDocument } = { latest: initial };
  function Harness() {
    const [doc, setDoc] = useState(initial);
    seen.latest = doc;
    return <FlowControlFields doc={doc} owner={owner} disabled={disabled} change={(edit) => setDoc((current) => edit(current))} />;
  }
  render(<Harness />);
  return { seen, user: userEvent.setup() };
}

/** Tabs forward until `target` has focus: the order of the page is what is tested, and a pointer is never used. */
async function tabTo(user: ReturnType<typeof userEvent.setup>, target: HTMLElement) {
  for (let presses = 0; presses < 60 && document.activeElement !== target; presses++) await user.tab();
  expect(target).toHaveFocus();
}

async function typeWithKeyboard(user: ReturnType<typeof userEvent.setup>, target: HTMLElement, text: string) {
  await tabTo(user, target);
  await user.keyboard(text.replace(/[{[]/g, (c) => c + c));
}

describe('the flow-control fields of the workflow settings', () => {
  it('with the keyboard only, declares the policies of provider-sync and writes the fixture', async () => {
    const expected = fixture('provider-sync.workflow.json');
    const start: IrDocument = { ...expected };
    delete start['concurrency'];
    delete start['rateLimit'];
    const { seen, user } = mount(start, WORKFLOW);

    const section = screen.getByRole('region', { name: 'Flow control' });
    expect(within(section).getByRole('group', { name: 'Concurrency limit' })).toHaveTextContent('Not limited.');
    expect(within(section).getByRole('group', { name: 'Rate limit' })).toHaveTextContent('Not limited.');

    await tabTo(user, screen.getByRole('button', { name: 'Add a concurrency limit' }));
    await user.keyboard('{Enter}');
    expect(seen.latest['concurrency']).toEqual({});
    expect(screen.getByRole('textbox', { name: 'Most holders at once' })).toHaveAccessibleDescription(/1 to 10000/);
    expect(screen.getByText('Enter the most holders at once.')).toBeInTheDocument();
    // Turning a gate on takes the keyboard to its first field.
    expect(screen.getByRole('textbox', { name: 'Most holders at once' })).toHaveFocus();
    await typeWithKeyboard(user, screen.getByRole('textbox', { name: 'Most holders at once' }), '1');
    await typeWithKeyboard(user, screen.getByRole('textbox', { name: 'Concurrency key' }), 'input.accountId');

    await tabTo(user, screen.getByRole('button', { name: 'Add a rate limit' }));
    await user.keyboard('{Enter}');
    await typeWithKeyboard(user, screen.getByRole('textbox', { name: 'Most grants per window' }), '100');
    const window = screen.getByRole('combobox', { name: 'Rate-limit window' });
    await tabTo(user, window);
    // A native select is operated by the browser; the value is chosen without a pointer.
    await user.selectOptions(window, 'minute');
    await typeWithKeyboard(user, screen.getByRole('textbox', { name: 'Rate-limit key' }), 'input.providerId');
    await typeWithKeyboard(user, screen.getByRole('textbox', { name: 'Rate-limit group' }), 'provider-starts');
    await typeWithKeyboard(user, screen.getByRole('textbox', { name: 'Rate-limit queue timeout amount' }), '15');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Rate-limit queue timeout unit' }), 'min');

    expect(JSON.stringify(seen.latest['concurrency'])).toBe(JSON.stringify(expected['concurrency']));
    expect(JSON.stringify(seen.latest['rateLimit'])).toBe(JSON.stringify(expected['rateLimit']));
    expect(canonical(seen.latest)).toBe(canonical(expected));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('says beside the field what the platform would refuse, and does not write a refused number', async () => {
    const { seen, user } = mount({ ...fixture('provider-sync.workflow.json') }, WORKFLOW);
    const limit = screen.getByRole('textbox', { name: 'Most grants per window' });
    await user.clear(limit);
    await user.type(limit, '6001');
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole number from 1 to 6000 (the most a minute allows).');
    expect((seen.latest['rateLimit'] as Record<string, unknown>)['limit']).toBe(600);

    // A shorter window lowers the ceiling of what is already written: the check says so and the value stays.
    await user.clear(limit);
    await user.type(limit, '500');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Rate-limit window' }), 'second');
    expect(screen.getByRole('textbox', { name: 'Most grants per window' })).toHaveAccessibleDescription(/the most a second allows/);
    expect((seen.latest['rateLimit'] as Record<string, unknown>)['limit']).toBe(500);

    const group = screen.getByRole('textbox', { name: 'Rate-limit group' });
    await user.clear(group);
    await user.type(group, 'Provider Starts');
    expect(group).toHaveAccessibleDescription(/starts with a lowercase letter/);

    const timeout = screen.getByRole('textbox', { name: 'Rate-limit queue timeout amount' });
    await user.clear(timeout);
    await user.type(timeout, '0');
    expect(screen.getByText('A queue timeout is from 1 ms to one day (86400000 ms).')).toBeInTheDocument();
  });

  it('removes a policy entirely, and keeps a member it does not know while saying so', async () => {
    const doc: IrDocument = { ...fixture('account-serial.workflow.json'), concurrency: { burst: 4, limit: 2 } };
    const { seen, user } = mount(doc, WORKFLOW);
    expect(screen.getByRole('group', { name: 'Concurrency limit' })).toHaveTextContent('It also holds burst, which this Studio does not know: kept as it is.');
    await user.type(screen.getByRole('textbox', { name: 'Concurrency key' }), 'input.accountId');
    expect(seen.latest['concurrency']).toEqual({ key: 'input.accountId', limit: 2, burst: 4 });
    await user.click(screen.getByRole('button', { name: 'Remove the concurrency limit' }));
    expect('concurrency' in seen.latest).toBe(false);
  });

  it('shows a gate that is not an object as unreadable and only lets it be removed', async () => {
    const { seen, user } = mount({ ...fixture('account-serial.workflow.json'), rateLimit: 'fast' }, WORKFLOW);
    const gate = screen.getByRole('group', { name: 'Rate limit' });
    expect(gate).toHaveTextContent('is not one this Studio can read');
    expect(within(gate).queryByRole('textbox')).not.toBeInTheDocument();
    await user.click(within(gate).getByRole('button', { name: 'Remove the rate limit' }));
    expect('rateLimit' in seen.latest).toBe(false);
  });

  it('a reader without write access sees the values and changes nothing', () => {
    mount(fixture('account-serial.workflow.json'), WORKFLOW, true);
    expect(screen.getByRole('textbox', { name: 'Concurrency group' })).toHaveValue('orders-by-account');
    expect(screen.getByRole('textbox', { name: 'Concurrency group' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: /Remove the/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Add a/ })).not.toBeInTheDocument();
  });
});

describe('the flow-control fields of a node', () => {
  it('writes the node-level policy of account-serial on its step', async () => {
    const expected = fixture('account-serial.workflow.json');
    const start: IrDocument = { ...expected, nodes: expected.nodes.map((n) => (n.id === 'notify' ? { id: n.id, type: n.type, activity: n['activity'], next: n.next } : n)) };
    const { seen, user } = mount(start, { level: 'node', nodeId: 'notify' });
    expect(screen.getByRole('region', { name: 'Flow control' })).toHaveTextContent(/attempts of this step/);
    await user.click(screen.getByRole('button', { name: 'Add a concurrency limit' }));
    await user.type(screen.getByRole('textbox', { name: 'Concurrency key' }), 'steps.price.output.region');
    await user.type(screen.getByRole('textbox', { name: 'Most holders at once' }), '2');
    expect(canonical(seen.latest)).toBe(canonical(expected));
    expect(JSON.stringify(seen.latest.nodes.find((n) => n.id === 'notify')?.['concurrency'])).toBe('{"key":"steps.price.output.region","limit":2}');
  });
});

type SavedIr = { nodes: Record<string, unknown>[]; [k: string]: unknown };
async function saved(platform: Platform, check: (ir: SavedIr) => void) {
  await waitFor(() => {
    expect(putsOf(platform).length).toBeGreaterThan(0);
    check(putsOf(platform).at(-1)!.body.ir as SavedIr);
  });
}

describe('in the editor', () => {
  it('offers the gates in the settings and on an activity, not on a condition, and saves them', async () => {
    const { user, platform } = await mountEditor({ view: 'list' });
    await screen.findByRole('heading', { level: 1, name: 'order-flow' });
    expect(screen.getByRole('heading', { level: 2, name: 'Workflow settings' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Flow control' })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Edit check' }));
    expect(screen.queryByRole('region', { name: 'Flow control' })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.click(screen.getByRole('button', { name: 'Add a rate limit' }));
    await user.selectOptions(screen.getByRole('combobox', { name: 'Rate-limit window' }), 'second');
    await user.type(screen.getByRole('textbox', { name: 'Most grants per window' }), '10');
    await saved(platform, (ir) => expect(ir.nodes.find((n) => n['id'] === 'pay')?.['rateLimit']).toEqual({ limit: 10, per: 'second' }));
  });
});
