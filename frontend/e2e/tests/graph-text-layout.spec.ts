import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';

test.use({ viewport: { width: 1470, height: 812 } });

const FIRST = 'authorize-customer-payment-with-risk-and-ledger-verification-v2x';
const TIMER = 'wait-for-provider-cooldown-before-evaluating-the-next-route-node';
const CONDITION = 'choose-a-route-from-the-complete-payment-risk-assessment-route01';
const PARALLEL = 'notify-ledger-risk-and-customer-services-in-parallel-branches01x';
const LONG_ACTIVITY = 'payments.authorize_customer_payment_with_required_risk_and_ledger_consistency_checks_via_the_primary_provider';
const SAGA = 'settle-customer-payment-and-compensate-every-completed-step-saga';
const RESERVE = 'reserve-customer-credit-before-submitting-the-final-payment-node';
const SUBMIT = 'submit-customer-payment-to-the-primary-processing-provider-route';
const DONE = 'record-the-compensated-payment-result-and-finish';
const FAILURE = 'PAYMENT_PROCESSOR_PERMANENTLY_DECLINED_AFTER_RISK_REVIEW_DECLINE';

function textWorkflow(name: string): Record<string, unknown> {
  return {
    ...IR_BASE,
    metadata: { name },
    entrypoint: FIRST,
    triggers: [{ kind: 'manual' }],
    nodes: [
      { id: FIRST, type: 'activity', activity: LONG_ACTIVITY, next: TIMER, onError: 'an-error-handler-that-does-not-exist' },
      { id: TIMER, type: 'timer', durationMs: 65_000, next: CONDITION },
      {
        id: CONDITION,
        type: 'condition',
        expression: 'input.customerRiskScore > 9000 && input.paymentRequiresAnAdditionalLedgerConsistencyCheck === true',
        ifTrue: PARALLEL,
        ifFalse: 'a-decision-route-that-does-not-exist',
      },
      { id: PARALLEL, type: 'parallel', join: 'all', branches: ['update-the-ledger-after-payment', 'notify-the-customer-after-payment'] },
      { id: 'update-the-ledger-after-payment', type: 'activity', activity: 'ledger.update_after_customer_payment', timeoutMs: 5_000 },
      { id: 'notify-the-customer-after-payment', type: 'activity', activity: 'customer.notify_after_payment', timeoutMs: 5_000 },
    ],
  };
}

function richRunWorkflow(name: string): Record<string, unknown> {
  return {
    ...IR_BASE,
    metadata: { name },
    entrypoint: SAGA,
    triggers: [{ kind: 'manual' }],
    nodes: [
      { id: SAGA, type: 'saga', begin: RESERVE, next: DONE, onCompensationFailure: 'manual_review', cancelPolicy: 'compensate_completed' },
      {
        id: RESERVE,
        type: 'activity',
        activity: 'credit.reserve_customer_limit_before_submitting_the_final_payment',
        timeoutMs: 5_000,
        next: SUBMIT,
        compensateWith: 'credit.release_customer_limit_after_declined_payment',
        compensationRetry: { maxAttempts: 1, strategy: 'fixed', initialDelayMs: 100, maxDelayMs: 100, jitter: false },
      },
      {
        id: SUBMIT,
        type: 'activity',
        activity: 'payment.submit_customer_payment_to_the_primary_processing_provider',
        timeoutMs: 5_000,
        retry: { maxAttempts: 1 },
      },
      { id: DONE, type: 'end' },
    ],
  };
}

interface GlyphRect {
  owner: string;
  text: string;
  left: number;
  right: number;
  top: number;
  bottom: number;
  ownerLeft: number;
  ownerRight: number;
  ownerTop: number;
  ownerBottom: number;
}

async function glyphRects(root: Locator, selectors: readonly string[]): Promise<GlyphRect[]> {
  return root.evaluate(
    (element, wanted) =>
      wanted.flatMap((selector) =>
        Array.from(element.querySelectorAll(selector)).flatMap((owner, ownerIndex) => {
          const ownerBounds = owner.getBoundingClientRect();
          const walker = document.createTreeWalker(owner, NodeFilter.SHOW_TEXT);
          const result: GlyphRect[] = [];
          for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
            const text = node.textContent ?? '';
            if (text.trim() === '') continue;
            const range = document.createRange();
            range.selectNodeContents(node);
            for (const rect of Array.from(range.getClientRects())) {
              if (rect.width === 0 || rect.height === 0) continue;
              result.push({
                owner: `${selector}[${ownerIndex}]`,
                text: text.trim().slice(0, 90),
                left: rect.left,
                right: rect.right,
                top: rect.top,
                bottom: rect.bottom,
                ownerLeft: ownerBounds.left,
                ownerRight: ownerBounds.right,
                ownerTop: ownerBounds.top,
                ownerBottom: ownerBounds.bottom,
              });
            }
          }
          return result;
        }),
      ),
    selectors,
  );
}

async function glyphContainmentFailures(root: Locator, selectors: readonly string[], where: string): Promise<string[]> {
  const rootBox = await root.boundingBox();
  expect(rootBox, `${where}: root bounds`).not.toBeNull();
  const glyphs = await glyphRects(root, selectors);
  expect(glyphs.length, `${where}: measured glyph runs`).toBeGreaterThan(0);
  return glyphs.flatMap((glyph) => {
    const margin = 0.75;
    const outsideOwner = glyph.left < glyph.ownerLeft - margin || glyph.right > glyph.ownerRight + margin || glyph.top < glyph.ownerTop - margin || glyph.bottom > glyph.ownerBottom + margin;
    const outsideRoot = glyph.left < rootBox!.x - margin || glyph.right > rootBox!.x + rootBox!.width + margin || glyph.top < rootBox!.y - margin || glyph.bottom > rootBox!.y + rootBox!.height + margin;
    return outsideOwner || outsideRoot
      ? [`${where} ${glyph.owner} “${glyph.text}”: glyph ${glyph.left.toFixed(1)},${glyph.top.toFixed(1)}–${glyph.right.toFixed(1)},${glyph.bottom.toFixed(1)}; owner ${glyph.ownerLeft.toFixed(1)},${glyph.ownerTop.toFixed(1)}–${glyph.ownerRight.toFixed(1)},${glyph.ownerBottom.toFixed(1)}`]
      : [];
  });
}

async function textOverlapFailures(root: Locator, groups: readonly string[], where: string): Promise<string[]> {
  const glyphs = await glyphRects(root, groups);
  const failures: string[] = [];
  for (let left = 0; left < glyphs.length; left += 1) {
    for (let right = left + 1; right < glyphs.length; right += 1) {
      const a = glyphs[left]!;
      const b = glyphs[right]!;
      if (a.owner === b.owner) continue;
      const overlapWidth = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const overlapHeight = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      if (overlapWidth > 0.75 && overlapHeight > 0.75) failures.push(`${where} ${a.owner} “${a.text}” overlaps ${b.owner} “${b.text}”`);
    }
  }
  return failures;
}

async function blockBundledFonts(page: Page): Promise<() => Promise<void>> {
  let blocked = 0;
  await page.route(/\.woff2?(?:\?|$)/, async (route) => {
    blocked += 1;
    await route.abort();
  });
  return async () => {
    await page.evaluate(() => document.fonts.ready.then(() => undefined));
    expect(blocked, 'the bundled Geist fonts were requested and blocked').toBeGreaterThan(0);
    expect(
      await page.evaluate(() => {
        let loaded = 0;
        document.fonts.forEach((font) => {
          if (font.status === 'loaded') loaded += 1;
        });
        return loaded;
      }),
      'the browser is exercising the system-font fallback',
    ).toBe(0);
  };
}

async function measureCards(
  cases: ReadonlyArray<readonly [string, Locator]>,
  selectors: readonly string[],
  overlapGroups: readonly string[],
  mode: string,
): Promise<string[]> {
  const failures: string[] = [];
  for (const [label, card] of cases) {
    await expect(card, `${label} card`).toBeVisible();
    failures.push(...(await glyphContainmentFailures(card, selectors, `${mode} ${label}`)));
    failures.push(...(await textOverlapFailures(card, overlapGroups, `${mode} ${label}`)));
  }
  return failures;
}

const workflowCard = (page: Page, id: string) => page.locator(`.wf-canvas .react-flow__node[data-id="${id}"] .wf-card`);

async function initialWorkflowNodeBounds(page: Page, id: string): Promise<{ topInside: boolean; bottomInside: boolean; nodeTop: number; nodeBottom: number; canvasTop: number; canvasBottom: number }> {
  return page.locator('.wf-canvas').evaluate((canvas, nodeId) => {
    const node = canvas.querySelector<HTMLElement>(`.react-flow__node[data-id="${CSS.escape(nodeId)}"]`);
    if (node === null) throw new Error(`workflow node ${nodeId} is absent`);
    const canvasBounds = canvas.getBoundingClientRect();
    const nodeBounds = node.getBoundingClientRect();
    return {
      topInside: nodeBounds.top >= canvasBounds.top,
      bottomInside: nodeBounds.bottom <= canvasBounds.bottom,
      nodeTop: nodeBounds.top,
      nodeBottom: nodeBounds.bottom,
      canvasTop: canvasBounds.top,
      canvasBottom: canvasBounds.bottom,
    };
  }, id);
}

test('TEXT workflow cards keep every glyph, badge and port label readable', async ({ page }) => {
  const expectSystemFonts = await blockBundledFonts(page);
  const api = new Api('admin');
  const name = 'text-layout-workflow-cards';
  const created = await api.call<{ id: string }>('POST', '/workflows', { name, ir: textWorkflow(name) }, 201);
  await signIn(page, env.tokens.admin);
  await goTo(page, `/workflows/${created.body.id}`);
  await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
  await expectSystemFonts();
  const problems = failOnBrowserErrors(page);
  await expect(page.getByTestId('validation-summary')).toContainText(/problems?/, { timeout: 30_000 });

  const first = workflowCard(page, FIRST);
  await expect(first.locator('.wf-badge[data-tone="bad"]')).toBeVisible();
  await expect(first.locator('.wf-badge[data-tone="warn"]')).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('text-editor-reading-light.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  await page.screenshot({ path: test.info().outputPath('text-editor-reading-dark.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'none' });
  const initialEntrypoint = await initialWorkflowNodeBounds(page, FIRST);
  expect(
    initialEntrypoint,
    `the initial camera keeps the complete entrypoint (${initialEntrypoint.nodeTop.toFixed(1)}–${initialEntrypoint.nodeBottom.toFixed(1)}) inside the canvas (${initialEntrypoint.canvasTop.toFixed(1)}–${initialEntrypoint.canvasBottom.toFixed(1)})`,
  ).toMatchObject({ topInside: true, bottomInside: true });
  await page.getByRole('button', { name: 'Fit View', exact: true }).click();
  await page.screenshot({ path: test.info().outputPath('text-editor-overview-light.png'), fullPage: true });

  const selectors = ['.wf-id', '.wf-kind-line', '.wf-detail', '.wf-badge', '.wf-ports span'];
  const overlapGroups = ['.wf-detail', '.wf-badge', '.wf-ports span'];
  const cases: Array<[string, Locator]> = [
    ['manual trigger', workflowCard(page, '@trigger:0')],
    ['activity with checks', first],
    ['timer', workflowCard(page, TIMER)],
    ['condition', workflowCard(page, CONDITION)],
    ['parallel', workflowCard(page, PARALLEL)],
  ];
  const layoutFailures = await measureCards(cases, selectors, overlapGroups, 'light fallback-font');
  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  layoutFailures.push(...(await measureCards(cases, selectors, overlapGroups, 'dark fallback-font')));
  await page.screenshot({ path: test.info().outputPath('text-editor-overview-dark.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'active' });
  expect(await page.evaluate(() => matchMedia('(forced-colors: active)').matches)).toBe(true);
  layoutFailures.push(...(await measureCards(cases, selectors, overlapGroups, 'forced-colors fallback-font')));
  await page.screenshot({ path: test.info().outputPath('text-editor-overview-forced.png'), fullPage: true });

  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'none' });
  await page.getByRole('button', { name: /Layout direction: left to right/i }).click();
  await expect(page.getByRole('button', { name: /Layout direction: top to bottom/i })).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('text-editor-overview-tb-auto.png'), fullPage: true });
  await page.getByRole('button', { name: 'Fit View', exact: true }).click();
  layoutFailures.push(...(await measureCards(cases, selectors, overlapGroups, 'top-to-bottom fallback-font')));
  await page.screenshot({ path: test.info().outputPath('text-editor-overview-tb.png'), fullPage: true });

  await page.locator(`.wf-canvas .react-flow__node[data-id="${FIRST}"]`).click();
  await page.setViewportSize({ width: 320, height: 812 });
  const amount = page.getByRole('textbox', { name: 'Timeout amount' });
  const unit = page.getByRole('combobox', { name: 'Timeout unit' });
  await expect(amount).toBeVisible();
  await expect(unit).toBeVisible();
  await amount.fill('123456789012');
  const amountBox = await amount.boundingBox();
  const unitBox = await unit.boundingBox();
  expect(amountBox, 'Timeout amount has bounds').not.toBeNull();
  expect(unitBox, 'Timeout unit has bounds').not.toBeNull();
  if (amountBox!.width < 120) layoutFailures.push(`320px inspector Timeout amount is ${amountBox!.width.toFixed(1)}px wide; expected at least 120px for twelve digits`);
  const valueFits = await amount.evaluate((element) => {
    const input = element as HTMLInputElement;
    const style = getComputedStyle(input);
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d');
    if (context === null) return false;
    context.font = style.font;
    const contentWidth = input.clientWidth - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight);
    return context.measureText(input.value).width <= contentWidth;
  });
  if (!valueFits) layoutFailures.push('320px inspector clips a twelve-digit Timeout amount');
  const overlapWidth = Math.min(amountBox!.x + amountBox!.width, unitBox!.x + unitBox!.width) - Math.max(amountBox!.x, unitBox!.x);
  const overlapHeight = Math.min(amountBox!.y + amountBox!.height, unitBox!.y + unitBox!.height) - Math.max(amountBox!.y, unitBox!.y);
  if (overlapWidth > 1 && overlapHeight > 1) layoutFailures.push('320px inspector Timeout amount overlaps its unit selector');
  await page.screenshot({ path: test.info().outputPath('text-editor-inspector-320.png'), fullPage: true });
  expect([...new Set(layoutFailures)], 'all workflow-card glyphs stay visible and detail, badges and ports do not overlap').toEqual([]);
  expect(problems).toEqual([]);
});

const runCard = (page: Page, id: string) => page.locator(`.rn-canvas .react-flow__node[data-id="${id}"] .rn-card`);

async function selectRunNode(page: Page, id: string): Promise<Locator> {
  const node = page.locator(`.rn-canvas .react-flow__node[data-id="${id}"]`);
  await expect(node).toBeVisible();
  await node.focus();
  await page.keyboard.press('Enter');
  const panel = page.getByTestId('node-panel');
  await expect(panel).toBeVisible();
  return panel;
}

test('TEXT rich run cards keep long ids, statuses and compensation notes readable', async ({ page }) => {
  test.setTimeout(180_000);
  const expectSystemFonts = await blockBundledFonts(page);
  const api = new Api('admin');
  const name = 'text-layout-rich-runtime-cards';
  const workflow = await api.publish(name, richRunWorkflow(name));
  const run = await api.start(workflow.id);
  const worker = await startWorker(
    'text-layout-worker',
    {
      'credit.reserve_customer_limit_before_submitting_the_final_payment': () => ({ reservation: 'reserved' }),
      'payment.submit_customer_payment_to_the_primary_processing_provider': () => {
        throw ActivityFailure.permanent(FAILURE, 'the provider declined the payment after risk review');
      },
    },
    { 'credit.release_customer_limit_after_declined_payment': () => ({ released: true }) },
  );
  try {
    await api.until(run, 'COMPENSATED');
  } finally {
    await worker.stop();
  }

  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Compensated', { exact: true }).first()).toBeVisible();
  await expectSystemFonts();
  const problems = failOnBrowserErrors(page);
  await page.screenshot({ path: test.info().outputPath('text-run-reading-light.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  await page.screenshot({ path: test.info().outputPath('text-run-reading-dark.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'none' });
  await page.getByRole('button', { name: 'Fit View', exact: true }).click();

  const compensationId = `${RESERVE}~c`;
  const saga = runCard(page, SAGA);
  const failed = runCard(page, SUBMIT);
  const compensation = runCard(page, compensationId);
  await expect(saga).toContainText('Failed');
  await expect(saga).toContainText(SAGA);
  await expect(failed).toContainText('Failed');
  await expect(failed).toContainText(SUBMIT);
  await expect(failed).toContainText('Failure details available');
  await expect(compensation).toContainText('Succeeded');
  await expect(compensation).toContainText(new RegExp(`Undo of ${RESERVE}`));

  const cards = [
    ['completed saga', saga],
    ['failed activity', failed],
    ['compensation', compensation],
  ] as const;
  const cardSelectors = ['.rn-card-label', '.rn-id', '.rn-kind', '.rn-card-status .badge', '.rn-attention'];
  const cardGroups = ['.rn-id', '.rn-kind', '.rn-card-status .badge', '.rn-attention'];
  const layoutFailures = await measureCards(cards, cardSelectors, cardGroups, 'light fallback-font');
  await page.screenshot({ path: test.info().outputPath('text-run-overview-light.png'), fullPage: true });

  let panel = await selectRunNode(page, SAGA);
  await expect(panel.locator('dt').filter({ hasText: /^Saga$/ })).toBeVisible();
  await expect(panel).toContainText(/saga done, compensated/i);
  panel = await selectRunNode(page, SUBMIT);
  await expect(panel.locator('dt').filter({ hasText: /^Failure code$/ })).toBeVisible();
  await expect(panel).toContainText(FAILURE);
  panel = await selectRunNode(page, compensationId);
  await expect(panel.locator('dt').filter({ hasText: /^Kind$/ })).toBeVisible();
  await expect(panel).toContainText(`Undo of ${RESERVE}`);
  layoutFailures.push(...(await glyphContainmentFailures(panel, ['.rn-selected-step strong', '.rn-selected-step dt', '.rn-selected-step dd', '.rn-facts dt', '.rn-facts dd'], 'selected compensation details')));
  await page.screenshot({ path: test.info().outputPath('text-run-panel-light.png'), fullPage: true });

  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  layoutFailures.push(...(await measureCards(cards, cardSelectors, cardGroups, 'dark fallback-font')));
  await page.screenshot({ path: test.info().outputPath('text-run-panel-dark.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'active' });
  expect(await page.evaluate(() => matchMedia('(forced-colors: active)').matches)).toBe(true);
  layoutFailures.push(...(await measureCards(cards, cardSelectors, cardGroups, 'forced-colors fallback-font')));
  await page.screenshot({ path: test.info().outputPath('text-run-panel-forced.png'), fullPage: true });
  expect([...new Set(layoutFailures)], 'run-card ids, status words, attention and selected details stay visible without text collisions').toEqual([]);
  expect(problems).toEqual([]);
});
