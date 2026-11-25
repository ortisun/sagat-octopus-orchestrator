import { expect, test } from '@playwright/test';
import { env } from '../support/env';
import { expectAccessible } from '../support/accessibility';
import { activate, choose, openFromMenu, reach, signInByKeyboard, typeInto } from '../support/keyboard';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo } from '../support/studio';
import { namespaceIdOf, unique } from '../support/world';

/**
 * Feature 022 in a real browser, with the keyboard alone (T022-054, T022-055, T022-056, T022-085; FR-022-035, FR-022-050):
 * an author declares a concurrency limit and a rate limit on the workflow and a rate limit on a step, and the draft the
 * platform stores holds the policies of the shared fixture `provider-sync.workflow.json` member for member and in the same
 * order; an operator reaches the namespace's concurrency and rate-limit keys and filters them by group.
 *
 * The platform of these journeys runs with the flow-control capabilities switched off and without the digest keys, so no
 * gated run can be admitted here: the run view of a permit or a slot is proved by the Vitest suite of `RunGates`, and the
 * listings are seen empty (a page without a next one needs no sealed cursor).
 */
test.use({ viewport: { width: 1470, height: 900 } });

test('FLOW-CONTROL an author declares concurrency and rate limits with the keyboard and an operator reads the keys', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = unique('provider-sync');
  const created = await api.call<{ id: string }>(
    'POST',
    '/workflows',
    {
      name,
      ir: {
        ...IR_BASE,
        metadata: { name },
        entrypoint: 'fetch',
        triggers: [{ kind: 'manual' }],
        nodes: [
          { id: 'fetch', type: 'activity', activity: 'provider.fetch', timeoutMs: 5000, next: 'done' },
          { id: 'done', type: 'end' },
        ],
      },
    },
    201,
  );
  const id = created.body.id;

  await signInByKeyboard(page, env.tokens.admin, env.namespace);
  await goTo(page, `/workflows/${id}`);
  await expect(page.getByRole('heading', { level: 1, name })).toBeVisible();
  const inspector = page.getByRole('complementary', { name: 'Inspector' });
  await expect(inspector.getByRole('heading', { level: 2, name: 'Workflow settings' })).toBeVisible();

  // The workflow's own gates: turning one on takes focus to its first field.
  const settings = inspector.getByRole('region', { name: 'Flow control' });
  await activate(page, settings.getByRole('button', { name: 'Add a concurrency limit' }));
  await expect(settings.getByRole('textbox', { name: 'Most holders at once' })).toBeFocused();
  await typeInto(page, settings.getByRole('textbox', { name: 'Most holders at once' }), '1');
  await typeInto(page, settings.getByRole('textbox', { name: 'Concurrency key' }), 'input.accountId');

  await activate(page, settings.getByRole('button', { name: 'Add a rate limit' }));
  await choose(page, settings.getByRole('combobox', { name: 'Rate-limit window' }), 'a minute');
  // The platform's own bound for the window, said beside the field; the refused number is not written.
  await typeInto(page, settings.getByRole('textbox', { name: 'Most grants per window' }), '6001');
  await expect(settings.getByRole('alert').filter({ hasText: 'Enter a whole number from 1 to 6000 (the most a minute allows).' })).toBeVisible();
  await typeInto(page, settings.getByRole('textbox', { name: 'Most grants per window' }), '100');
  await expect(settings.getByRole('alert')).toHaveCount(0);
  await typeInto(page, settings.getByRole('textbox', { name: 'Rate-limit key' }), 'input.providerId');
  await typeInto(page, settings.getByRole('textbox', { name: 'Rate-limit group' }), 'provider-starts');
  await typeInto(page, settings.getByRole('textbox', { name: 'Rate-limit queue timeout amount' }), '15');
  await choose(page, settings.getByRole('combobox', { name: 'Rate-limit queue timeout unit' }), 'min');
  await expectAccessible(page, 'the workflow settings with two gates');

  // The step's gate, from the list of nodes.
  await activate(page, page.getByRole('button', { name: 'List' }));
  await activate(page, page.getByRole('button', { name: 'Edit fetch' }));
  const step = inspector.getByRole('region', { name: 'Flow control' });
  await expect(step).toContainText('attempts of this step');
  await activate(page, step.getByRole('button', { name: 'Add a rate limit' }));
  await typeInto(page, step.getByRole('textbox', { name: 'Most grants per window' }), '10');
  await choose(page, step.getByRole('combobox', { name: 'Rate-limit window' }), 'a second');
  await typeInto(page, step.getByRole('textbox', { name: 'Rate-limit key' }), 'input.providerId');
  await expectAccessible(page, 'the inspector of a step with a gate');
  await expect(page.getByRole('group', { name: 'Editor tools' })).toContainText(/Saved · revision \d+/, { timeout: 20_000 });

  // What the platform stored is the fixture's policies, member for member and in its order.
  await expect
    .poll(async () => {
      const draft = (await api.call<{ ir: Record<string, unknown> & { nodes: Array<Record<string, unknown>> } }>('GET', `/workflows/${id}`, undefined, 200)).body.ir;
      return JSON.stringify([draft['concurrency'], draft['rateLimit'], draft.nodes.find((n) => n['id'] === 'fetch')?.['rateLimit']]);
    }, { timeout: 20_000 })
    .toBe(
      JSON.stringify([
        { key: 'input.accountId', limit: 1 },
        { group: 'provider-starts', key: 'input.providerId', limit: 100, per: 'minute', queueTimeoutMs: 900000 },
        { key: 'input.providerId', limit: 10, per: 'second' },
      ]),
    );

  // An operator's view of the keys: both listings, reached and filtered with the keyboard.
  await openFromMenu(page, 'Operations', 'Operations');
  const keys = page.getByRole('region', { name: 'Flow-control keys' });
  const concurrency = keys.getByRole('region', { name: 'Concurrency keys' });
  await expect(concurrency.getByText('No concurrency key has a holder or a waiter in this namespace.')).toBeVisible();
  await expect(keys.getByRole('region', { name: 'Rate-limit keys' }).getByText('No rate-limit key has a counted grant or a waiter in this namespace.')).toBeVisible();
  const namespaceId = await namespaceIdOf(api, env.namespace);
  const filtered = page.waitForRequest((request) => request.url().includes(`/namespaces/${namespaceId}/concurrency-keys`) && request.url().includes('group=provider-starts'));
  await typeInto(page, concurrency.getByRole('textbox', { name: 'Group' }), 'provider-starts');
  await page.keyboard.press('Enter');
  await filtered;
  await expect(concurrency.getByText('No concurrency key has a holder or a waiter in this namespace.')).toBeVisible();
  await reach(page, keys.getByRole('region', { name: 'Rate-limit keys' }).getByRole('button', { name: 'Filter' }));
  await expectAccessible(page, 'the flow-control keys of the namespace');

  await page.setViewportSize({ width: 375, height: 812 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  expect(problems).toEqual([]);
});
