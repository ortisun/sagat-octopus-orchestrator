import { screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { NS_ID, mountEditor, putsOf, sampleIr } from '../../../test/workflowsHarness';
import type { Platform } from '../../../test/workflowsHarness';

const row = (id: string) => screen.getByTestId(`row-${id}`);

async function opened(options: Parameters<typeof mountEditor>[0] = {}) {
  const mounted = await mountEditor({ view: 'list', ...options });
  await screen.findByRole('heading', { level: 1, name: 'order-flow' });
  return mounted;
}

type SavedIr = { nodes: Record<string, unknown>[]; triggers?: Record<string, unknown>[]; metadata: Record<string, unknown>; [k: string]: unknown };

/** The document of the latest save, once a save that satisfies `check` has gone out. */
async function saved(platform: Platform, check: (ir: SavedIr) => void = () => undefined): Promise<SavedIr> {
  await waitFor(() => {
    expect(putsOf(platform).length).toBeGreaterThan(0);
    check(putsOf(platform).at(-1)!.body.ir as SavedIr);
  });
  return putsOf(platform).at(-1)!.body.ir as SavedIr;
}

const node = (ir: { nodes: Record<string, unknown>[] }, id: string) => ir.nodes.find((n) => n['id'] === id)!;

describe('the fields of a node', () => {
  it('shows the fields of the kind, with required ones marked for assistive technology', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    expect(screen.getByRole('textbox', { name: 'Activity' })).toHaveAttribute('aria-required', 'true');
    expect(screen.getByRole('textbox', { name: 'Activity' })).toHaveAccessibleDescription(/The name a worker registers/);
    expect(screen.getByRole('textbox', { name: 'Activity' })).toHaveValue('payment.submit');
    expect(screen.getByRole('group', { name: 'Timeout' })).toBeInTheDocument();
    expect(screen.getByRole('group', { name: 'Retries' })).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: /Cannot be undone/ })).not.toBeChecked();
  });

  it('edits a text field and saves it', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const field = screen.getByRole('textbox', { name: 'Activity' });
    await user.clear(field);
    await user.type(field, 'payment.capture');
    await saved(platform, (ir) => expect(node(ir, 'pay')['activity']).toBe('payment.capture'));
  });

  it('edits an expression in a monospace text area', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit check' }));
    const field = screen.getByRole('textbox', { name: 'Expression' });
    expect(field.tagName).toBe('TEXTAREA');
    expect(field).toHaveAttribute('spellcheck', 'false');
    await user.clear(field);
    await user.type(field, 'input.amount > 100');
    await saved(platform, (ir) => expect(node(ir, 'check')['expression']).toBe('input.amount > 100'));
  });

  it('takes a duration as an amount and a unit and stores milliseconds', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const amount = screen.getByRole('textbox', { name: 'Timeout amount' });
    expect(amount).toHaveValue('30');
    expect(screen.getByRole('combobox', { name: 'Timeout unit' })).toHaveValue('s');
    await user.clear(amount);
    await user.type(amount, '2');
    await user.selectOptions(screen.getByRole('combobox', { name: 'Timeout unit' }), 'min');
    expect(amount).toHaveValue('2');
    await saved(platform, (ir) => expect(node(ir, 'pay')['timeoutMs']).toBe(120_000));
  });

  it('does not store a duration that is not a whole number, says so, and goes back to the last good value on leaving', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const amount = screen.getByRole('textbox', { name: 'Timeout amount' });
    await user.clear(amount);
    await user.type(amount, '1x');
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole number of s');
    expect(amount).toHaveAttribute('aria-invalid', 'true');
    // The last good value ("1", typed before the "x") is what the document holds; reselecting the node shows it.
    await user.click(screen.getByRole('button', { name: 'Edit check' }));
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    expect(screen.getByRole('textbox', { name: 'Timeout amount' })).toHaveValue('1');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('removes a duration when its field is emptied', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.clear(screen.getByRole('textbox', { name: 'Timeout amount' }));
    await saved(platform, (ir) => expect('timeoutMs' in node(ir, 'pay')).toBe(false));
  });

  it('edits the retry policy as a sub-form and stores it only while it says something', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const retries = screen.getByRole('group', { name: 'Retries' });
    await user.type(within(retries).getByRole('textbox', { name: 'Most attempts' }), '5');
    await user.selectOptions(within(retries).getByRole('combobox', { name: 'Delay between attempts' }), 'exponential');
    await user.type(within(retries).getByRole('textbox', { name: 'First delay amount' }), '2');
    await user.selectOptions(within(retries).getByRole('combobox', { name: 'First delay unit' }), 's');
    await user.click(within(retries).getByRole('checkbox', { name: 'Add jitter' }));
    await saved(platform, (ir) => expect(node(ir, 'pay')['retry']).toEqual({ maxAttempts: 5, strategy: 'exponential', initialDelayMs: 2_000, jitter: true }));
  });

  it('refuses a number of attempts outside what the platform accepts', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const attempts = screen.getByRole('textbox', { name: 'Most attempts' });
    await user.type(attempts, '500');
    expect(screen.getByRole('alert')).toHaveTextContent('from 1 to 100');
    expect(attempts).toHaveAttribute('aria-invalid', 'true');
  });

  it('keeps a list of secret names: adds valid ones, refuses others, removes', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    const add = screen.getByRole('textbox', { name: 'Add to secrets' });
    await user.type(add, 'stripe.key{Enter}');
    await user.type(add, 'Bad Name{Enter}');
    expect(screen.getByRole('alert')).toHaveTextContent('starts with a lowercase letter');
    await user.clear(add);
    await user.type(add, 'stripe.key{Enter}');
    expect(screen.getByRole('alert')).toHaveTextContent('already in the list');
    await user.clear(add);
    await user.type(add, 'db-password');
    await user.click(screen.getByRole('button', { name: 'Add' }));
    await saved(platform, (ir) => expect(node(ir, 'pay')['secrets']).toEqual(['stripe.key', 'db-password']));
    await user.click(screen.getByRole('button', { name: 'Remove secrets stripe.key' }));
    await saved(platform, (ir) => expect(node(ir, 'pay')['secrets']).toEqual(['db-password']));
  });

  it('toggles a boolean and picks from a select', async () => {
    const { user, platform } = await opened();
    await user.click(screen.getByRole('button', { name: 'Edit pay' }));
    await user.click(screen.getByRole('checkbox', { name: /Cannot be undone/ }));
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Parallel' }));
    await user.selectOptions(screen.getByRole('combobox', { name: 'Join' }), 'any');
    await saved(platform, (ir) => {
      expect(node(ir, 'pay')['irreversible']).toBe(true);
      expect(node(ir, 'parallel_1')['join']).toBe('any');
    });
  });

  it('tells the author that nodes the platform cannot run yet can be drawn but not published', async () => {
    const { user } = await opened();
    for (const [kind, label] of [
      ['For each', 'For each'],
      ['Human approval', 'Human approval'],
    ] as const) {
      await user.click(screen.getByRole('button', { name: 'Add node' }));
      await user.click(await screen.findByRole('button', { name: kind }));
      expect(screen.getByRole('heading', { level: 2, name: new RegExp(`^${label}:`) })).toBeInTheDocument();
      expect(screen.getByRole('alert')).toHaveTextContent('The platform cannot run this node yet; you can draw it but not publish it.');
    }
    expect(row('foreach_1')).toHaveTextContent('Not run yet');
    expect(row('humanApproval_1')).toHaveTextContent('Not run yet');
  });

  it('lists what the platform and the Studio say about the selected node, apart from each other', async () => {
    const { user } = await opened({
      behaviour: {
        validate: () => ({ valid: false, problems: [{ severity: 'error', code: 'EXPRESSION', message: 'the expression is not valid', nodeId: 'check' }], warnings: [] }),
      },
    });
    await user.click(screen.getByRole('button', { name: 'Edit check' }));
    const checks = await screen.findByRole('region', { name: 'Checks for this node' });
    await within(checks).findByText('the expression is not valid');
    expect(within(checks).getByText('Problem.')).toBeInTheDocument();
  });
});

describe('script nodes', () => {
  const digest = 'sha256:' + 'c'.repeat(64);
  const extra = {
    [`GET /namespaces/${NS_ID}/scripts`]: () => ({ items: [{ id: 'script-1', namespaceId: NS_ID, name: 'normalise', language: 'typescript' }] }),
    [`GET /namespaces/${NS_ID}/scripts/script-1/versions`]: () => ({
      items: [
        { id: 'sv-2', scriptId: 'script-1', number: 2, language: 'python', status: 'READY', artifactDigest: digest },
        { id: 'sv-1', scriptId: 'script-1', number: 1, language: 'typescript', status: 'FAILED' },
        // A version still being built that already carries a digest is not one to pin.
        { id: 'sv-3', scriptId: 'script-1', number: 3, language: 'typescript', status: 'BUILDING', artifactDigest: 'sha256:' + 'd'.repeat(64) },
      ],
    }),
  };

  async function withScriptNode(options: Parameters<typeof mountEditor>[0] = {}) {
    const mounted = await opened({ behaviour: { extra }, ...options });
    await mounted.user.click(screen.getByRole('button', { name: 'Add node' }));
    await mounted.user.click(await screen.findByRole('button', { name: 'Script' }));
    return mounted;
  }

  it('picks a script, then one of its versions that built, and keeps the digest and the language of it', async () => {
    const { user, platform } = await withScriptNode();
    const script = await screen.findByRole('combobox', { name: 'Script' });
    await waitFor(() => expect(within(script).getByRole('option', { name: 'normalise (typescript)' })).toBeInTheDocument());
    await user.selectOptions(script, 'normalise (typescript)');
    const version = await screen.findByRole('combobox', { name: 'Built version' });
    await waitFor(() => expect(within(version).getAllByRole('option')).toHaveLength(2));
    // A version that failed to build, or is still building, cannot be pinned.
    expect(within(version).queryByRole('option', { name: /v1/ })).not.toBeInTheDocument();
    expect(within(version).queryByRole('option', { name: /v3/ })).not.toBeInTheDocument();
    await user.selectOptions(version, 'v2 · python');
    expect(screen.getByRole('textbox', { name: 'Artifact' })).toHaveValue(digest);
    expect(screen.getByRole('combobox', { name: 'Language' })).toHaveValue('python');
    await saved(platform, (ir) => expect(node(ir, 'script_1')).toMatchObject({ artifactDigest: digest, language: 'python' }));
  });

  it('still takes a digest typed or pasted', async () => {
    const { user } = await withScriptNode();
    const field = screen.getByRole('textbox', { name: 'Artifact' });
    await user.click(field);
    await user.paste(digest);
    expect(field).toHaveValue(digest);
  });

  it('says what is missing when the user may not read scripts, and leaves the digest field', async () => {
    const { user } = await opened({ permissions: ['definition:read', 'definition:write'], behaviour: { extra } });
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Script' }));
    expect(screen.getByText(/needs the script:read permission/)).toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: 'Script' })).not.toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Artifact' })).toBeInTheDocument();
  });

  it('shows a failure to load the scripts, with a way to try again', async () => {
    const { user } = await withScriptNode({
      behaviour: {
        extra: {
          [`GET /namespaces/${NS_ID}/scripts`]: () =>
            new Response(JSON.stringify({ code: 'UNAVAILABLE', message: 'scripts are down', traceId: 'f'.repeat(32) }), {
              status: 503,
              headers: { 'content-type': 'application/json' },
            }),
        },
      },
    });
    const alert = await screen.findByText(/scripts are down/);
    expect(alert.closest('[role="alert"]')).toHaveTextContent('f'.repeat(32));
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
    expect(user).toBeDefined();
  });
});

describe('the settings of the workflow', () => {
  it('opens on the settings when no node is selected, with the name that cannot be changed here', async () => {
    await opened();
    expect(screen.getByRole('heading', { level: 2, name: 'Workflow settings' })).toBeInTheDocument();
    expect(screen.getByText('order-flow', { selector: 'code' })).toBeInTheDocument();
  });

  it('edits the description and the first step', async () => {
    const { user, platform } = await opened();
    await user.type(screen.getByRole('textbox', { name: 'Description' }), 'Takes an order');
    await user.selectOptions(screen.getByRole('combobox', { name: 'First step' }), 'pay (Step)');
    await saved(platform, (ir) => {
      expect(ir.metadata['description']).toBe('Takes an order');
      expect(ir['entrypoint']).toBe('pay');
    });
  });

  it('edits the triggers of every kind with the fields of the kind', async () => {
    const { user, platform } = await opened();
    const add = async (kind: string) => {
      await user.selectOptions(screen.getByRole('combobox', { name: 'New trigger' }), kind);
      await user.click(screen.getByRole('button', { name: 'Add trigger' }));
    };
    await add('Event: an event of a type');
    await user.type(screen.getByRole('textbox', { name: 'Trigger 2: Event type' }), 'order.created');
    await user.type(screen.getByRole('textbox', { name: 'Trigger 2: Source' }), 'api');
    await add('Webhook: an endpoint of the namespace');
    await user.type(screen.getByRole('textbox', { name: 'Trigger 3: Endpoint' }), 'shop');
    await add('Message: a connector consumes a queue or topic');
    await user.type(screen.getByRole('textbox', { name: 'Trigger 4: Connector' }), 'orders');
    await add('Schedule: a cron expression');
    await user.type(screen.getByRole('textbox', { name: 'Trigger 5: Schedule' }), '0 9 * * 1');
    await user.type(screen.getByRole('textbox', { name: 'Trigger 5: Time zone' }), 'Europe/Lisbon');
    await saved(platform, (ir) =>
      expect(ir.triggers).toEqual([
        { kind: 'manual' },
        { kind: 'event', eventType: 'order.created', source: 'api' },
        { kind: 'webhook', endpoint: 'shop' },
        { kind: 'message', connector: 'orders' },
        { kind: 'cron', cron: '0 9 * * 1', timezone: 'Europe/Lisbon' },
      ]),
    );
  });

  it('removes a trigger', async () => {
    const { user } = await opened();
    await user.click(screen.getByRole('button', { name: 'Remove trigger 1' }));
    expect(screen.getByText('No triggers.')).toBeInTheDocument();
  });

  it('keeps a JSON schema as text, saves it only when it is a JSON object, and says why when it is not', async () => {
    const { user, platform } = await opened();
    const input = screen.getByRole('textbox', { name: 'Input schema' });
    await user.click(input);
    await user.paste('{"type": "object"');
    expect(screen.getByRole('alert')).toHaveTextContent('not valid JSON');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    await user.tab();
    expect(input).toHaveValue('{"type": "object"');
    await user.click(input);
    await user.keyboard('{End}');
    await user.paste(', "required": ["amount"]}');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    await saved(platform, (ir) => expect(ir['inputSchema']).toEqual({ type: 'object', required: ['amount'] }));
    await user.clear(input);
    await user.paste('[1, 2]');
    expect(screen.getByRole('alert')).toHaveTextContent('A schema is a JSON object');
    await user.clear(input);
    await saved(platform, (ir) => expect('inputSchema' in ir).toBe(false));
  });
});

describe('a document with kinds the Studio does not know', () => {
  it('shows an unknown node and keeps it as it is', async () => {
    const ir = sampleIr();
    (ir['nodes'] as unknown[]).push({ id: 'odd', type: 'teleport', next: 'done', secret: 'kept' });
    const { user, platform } = await opened({ state: { ir } });
    expect(row('odd')).toHaveTextContent('teleport');
    await user.click(screen.getByRole('button', { name: 'Edit odd' }));
    expect(screen.getByText(/not one this Studio knows/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Add node' }));
    await user.click(await screen.findByRole('button', { name: 'Timer' }));
    await saved(platform, (ir) => expect(node(ir, 'odd')).toEqual({ id: 'odd', type: 'teleport', next: 'done', secret: 'kept' }));
  });
});
