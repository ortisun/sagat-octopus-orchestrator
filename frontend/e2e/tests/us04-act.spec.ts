import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, paymentSaga } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';

test.use({ viewport: { width: 1440, height: 900 } });

/** A run whose undo of the reservation failed for good: it waits for a person. */
async function stuckRun(name: string) {
  const api = new Api('admin');
  const wf = await api.publish(name, paymentSaga(name));
  let release = 0;
  const host = await startWorker(`${name}-worker`, {
    'payment.validate': () => ({ valid: true }),
    'limit.reserve': () => ({ reservation: 'r-1' }),
    'payment.submit': () => { throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer'); },
  }, {
    'limit.release': () => {
      release += 1;
      // The limit service is down for the first attempt; the retry the operator asks for finds it back.
      if (release === 1) throw ActivityFailure.retryable('LIMIT_SERVICE_DOWN', 'the limit service is down');
      return { released: true };
    },
  });
  const run = await api.start(wf.id, { paymentId: 'p-9', amount: 25 });
  await api.until(run, 'MANUAL_REVIEW');
  return { api, run, host };
}

/** An ordinary activity with no high-risk label whose provider outcome is unknown. */
async function unknownOriginalRun(name: string) {
  const api = new Api('admin');
  const wf = await api.publish(name, paymentSaga(name));
  const host = await startWorker(
    `${name}-worker`,
    {
      'payment.validate': () => ({ valid: true }),
      'limit.reserve': () => ({ reservation: 'r-unknown' }),
      'payment.submit': () => {
        throw ActivityFailure.outcomeUnknown('the provider accepted the request but its answer was lost');
      },
    },
    { 'limit.release': () => ({ released: true }) },
  );
  const run = await api.start(wf.id, { paymentId: 'p-unknown', amount: 25 });
  await api.until(run, 'MANUAL_REVIEW');
  return { api, run, host };
}

/** A known forward failure followed by a compensation whose provider outcome is unknown. */
async function unknownCompensationRun(name: string) {
  const api = new Api('admin');
  const wf = await api.publish(name, paymentSaga(name));
  const host = await startWorker(
    `${name}-worker`,
    {
      'payment.validate': () => ({ valid: true }),
      'limit.reserve': () => ({ reservation: 'r-compensation-unknown' }),
      'payment.submit': () => {
        throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer');
      },
    },
    {
      'limit.release': () => {
        throw ActivityFailure.outcomeUnknown('the provider accepted the release but its answer was lost');
      },
    },
  );
  const run = await api.start(wf.id, { paymentId: 'p-compensation-unknown', amount: 25 });
  await api.until(run, 'MANUAL_REVIEW');
  return { api, run, host };
}

interface AuditRecord {
  actor: string;
  action: string;
  resourceId: string;
  reason?: string | null;
}

async function resolutionAudit(api: Api, run: string, step: string): Promise<AuditRecord[]> {
  const query = new URLSearchParams({
    action: 'saga.step_resolved',
    resourceType: 'run',
    resourceId: `${run}/${step}`,
  });
  return (await api.call<{ items: AuditRecord[] }>('GET', `/audit?${query.toString()}`, undefined, 200)).body.items;
}

async function openRun(page: Page, who: string, run: string): Promise<void> {
  await signIn(page, who);
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);
}

async function openStepActions(page: Page, step: string): Promise<void> {
  await page.getByRole('tab', { name: 'Table' }).click();
  await page.getByRole('button', { name: `Actions for ${step}`, exact: true }).click();
}

test('US04 an operator finds the step that waits for a decision, decides with a reason that is recorded, and the history is not overwritten', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const { api, run, host } = await stuckRun('payment-us04');
  try {
    const before = await api.journal(run);
    await openRun(page, env.tokens.operator, run);
    await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Needs a person', { exact: true }).first()).toBeVisible();

    // Only what the role and the state allow is offered, and what is not is explained.
    await openStepActions(page, 'reserve~c');
    await expect(page.getByRole('button', { name: 'Retry compensation' })).toBeEnabled();
    await expect(page.getByRole('button', { name: 'Skip compensation' })).toBeDisabled();
    await expect(page.locator('[data-action="SKIP_COMPENSATION"]')).toContainText('You need the saga:override permission to do this.');
    await page.getByRole('button', { name: 'Retry compensation' }).click();

    // The dialog says what will happen, and a reason is required before anything is sent.
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText(/another attempt/i);
    await expect(dialog).toContainText(/journal/i);
    const confirm = dialog.getByRole('button', { name: 'Confirm: Retry compensation' });
    await dialog.getByRole('textbox', { name: 'Reason' }).fill('no');
    await confirm.click();
    await expect(dialog.getByText(/Write at least 8 characters/)).toBeVisible();
    expect(await api.journal(run)).toHaveLength(before.length); // nothing was sent
    await dialog.getByRole('textbox', { name: 'Reason' }).fill('The limit service is back (ticket INC-4411).');
    await confirm.click();

    // The effect is the platform's, and arrives through the live view: the run ends compensated.
    await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Compensated', { exact: true }).first()).toBeVisible({ timeout: 30_000 });
    expect(await api.until(run, 'COMPENSATED')).toBe('COMPENSATED');

    // Nothing was erased: what the journal said before it still says, in the same order, and the decision is in it.
    const after = await api.journal(run);
    expect(after.slice(0, before.length).map((e) => [e.seq, e.eventType, e.stepId])).toEqual(before.map((e) => [e.seq, e.eventType, e.stepId]));
    expect(after.map((e) => e.eventType)).toEqual(expect.arrayContaining(['step.needs_review', 'step.retry_requested']));
    const decision = after.find((e) => e.eventType === 'step.retry_requested');
    expect(decision?.actor).toBe('user:olivia');
  } finally {
    await host.stop();
  }
  expect(problems).toEqual([]);
});

test('US04 a viewer watches but is not offered a decision, and giving up on an undo needs an administrator who types the step id', async ({ browser, page }) => {
  test.setTimeout(180_000);
  const { api, run, host } = await stuckRun('payment-us04b');
  try {
    // A viewer: the actions exist as information, disabled, with the reason.
    await openRun(page, env.tokens.viewer, run);
    await page.getByRole('button', { name: 'Run actions' }).click();
    await expect(page.getByRole('button', { name: 'Pause run' })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Cancel run' })).toBeDisabled();
    await expect(page.getByText('You need the run:operate permission to do this.').first()).toBeVisible();
    await page.keyboard.press('Escape');
    await openStepActions(page, 'reserve~c');
    await expect(page.getByRole('button', { name: 'Retry compensation' })).toBeDisabled();
    await expect(page.getByText('You need the run:operate permission to do this.').first()).toBeVisible();

    // The administrator may give up on the undo, after typing the step id back.
    const admin = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const adminPage = await admin.newPage();
    await openRun(adminPage, env.tokens.admin, run);
    await openStepActions(adminPage, 'reserve~c');
    await adminPage.getByRole('button', { name: 'Skip compensation' }).click();
    const dialog = adminPage.getByRole('dialog');
    await expect(dialog).toContainText(/partly undone/i);
    await dialog.getByRole('textbox', { name: 'Reason' }).fill('The limit was released by hand in the provider console.');
    const confirm = dialog.getByRole('button', { name: 'Confirm: Skip compensation' });
    await dialog.getByRole('textbox', { name: 'Type the step id to confirm' }).fill('reserve');
    await confirm.click();
    await expect(dialog.getByText('Type the step id exactly as shown to confirm.')).toBeVisible();
    expect(await api.until(run, 'MANUAL_REVIEW', 5_000)).toBe('MANUAL_REVIEW'); // still waiting: nothing was sent
    await dialog.getByRole('textbox', { name: 'Type the step id to confirm' }).fill('reserve~c');
    await confirm.click();
    await expect(adminPage.getByRole('region', { name: 'Run detail' }).getByText('Partly undone', { exact: true }).first()).toBeVisible({ timeout: 30_000 });
    expect(await api.until(run, 'PARTIAL')).toBe('PARTIAL');
    await admin.close();
  } finally {
    await host.stop();
  }
});

test('T037 an admin cannot retry an unknown ordinary effect, cancellation preserves its review, and resolution stays human-audited', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const { api, run, host } = await unknownOriginalRun('payment-us04-unknown-original');
  const reason = 'Provider support confirmed request p-unknown never applied.';
  try {
    const before = await api.journal(run);
    const beforeGraph = await api.graph(run);
    expect(beforeGraph.nodes.find((candidate) => candidate.id === 'submit')).toMatchObject({
      status: 'WAITING',
      waitingOn: 'operator',
      failureCode: 'OUTCOME_UNKNOWN',
      attempt: 1,
    });

    await openRun(page, env.tokens.admin, run);
    await openStepActions(page, 'submit');
    await expect(page.getByText('OUTCOME_UNKNOWN', { exact: true }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Retry step' })).toBeDisabled();
    await expect(page.locator('[data-action="RETRY_STEP"]')).toContainText('OUTCOME_UNKNOWN');
    await expect(page.locator('[data-action="RETRY_STEP"]')).toContainText(/resolve/i);
    await expect(page.getByRole('button', { name: 'Resolve step' })).toBeEnabled();

    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Run actions' }).click();
    await page.getByRole('button', { name: 'Cancel run' }).click();
    let dialog = page.getByRole('dialog');
    await expect(dialog).toContainText(/unknown|unresolved/i);
    await expect(dialog).toContainText(/preserv.*review|review.*preserv/i);
    await dialog.getByRole('textbox', { name: 'Reason' }).fill('Customer cancelled while provider evidence was pending.');
    await dialog.getByRole('button', { name: 'Confirm: Cancel run' }).click();
    await expect(dialog).toBeHidden();

    await expect.poll(async () => (await api.journal(run)).some((event) => event.eventType === 'run.stop_requested')).toBe(true);
    expect(await api.until(run, 'MANUAL_REVIEW', 5_000)).toBe('MANUAL_REVIEW');
    const stoppedGraph = await api.graph(run);
    expect(stoppedGraph.nodes.find((candidate) => candidate.id === 'submit')).toMatchObject({
      status: 'WAITING',
      waitingOn: 'operator',
      failureCode: 'OUTCOME_UNKNOWN',
      attempt: 1,
    });
    const stoppedJournal = await api.journal(run);
    expect(stoppedJournal.filter((event) => event.stepId === 'submit' && event.eventType === 'step.leased')).toHaveLength(1);
    expect(stoppedJournal.some((event) => event.stepId === 'submit' && event.eventType === 'step.retry_requested')).toBe(false);

    // API-only checks do not refresh the browser projection. Read the committed cancellation
    // before deciding, preserving the UI's optimistic-concurrency protection.
    const detail = page.getByRole('region', { name: 'Run detail' });
    const refreshed = page.waitForResponse((response) => response.url().endsWith(`/runs/${run}/graph`) && response.status() === 200);
    await detail.getByRole('button', { name: 'Refresh', exact: true }).click();
    await refreshed;
    await expect(detail.getByRole('button', { name: 'Refresh', exact: true })).toBeEnabled();
    await openStepActions(page, 'submit');
    await expect(page.getByRole('button', { name: 'Retry step' })).toBeDisabled();
    await page.getByRole('button', { name: 'Resolve step' }).click();
    dialog = page.getByRole('dialog');
    await dialog.getByRole('radio', { name: /It did not happen/ }).click();
    await dialog.getByRole('textbox', { name: 'Error code (optional)' }).fill('NOT_APPLIED_CONFIRMED');
    await dialog.getByRole('textbox', { name: 'Reason' }).fill(reason);
    const confirmation = dialog.getByRole('textbox', { name: 'Type the step id to confirm' });
    await confirmation.fill('submi');
    await dialog.getByRole('button', { name: 'Confirm: Resolve step' }).click();
    await expect(dialog.getByText('Type the step id exactly as shown to confirm.')).toBeVisible();
    expect(await api.journal(run)).toHaveLength(stoppedJournal.length);
    await confirmation.fill('submit');
    await dialog.getByRole('button', { name: 'Confirm: Resolve step' }).click();

    await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Cancelled', { exact: true }).first()).toBeVisible({ timeout: 30_000 });
    expect(await api.until(run, 'CANCELLED')).toBe('CANCELLED');
    const after = await api.journal(run);
    expect(after.slice(0, before.length).map((event) => [event.seq, event.eventType, event.stepId])).toEqual(before.map((event) => [event.seq, event.eventType, event.stepId]));
    const decision = after.find((event) => event.eventType === 'step.resolved' && event.stepId === 'submit');
    // Journal actors include their kind; audit actors retain the authenticated subject.
    expect(decision?.actor).toBe('user:alice');
    const audit = await resolutionAudit(api, run, 'submit');
    expect(audit).toHaveLength(1);
    expect(audit[0]).toMatchObject({
      action: 'saga.step_resolved',
      actor: 'alice',
      resourceId: `${run}/submit`,
      reason: `declared FAILED(NOT_APPLIED_CONFIRMED): ${reason}`,
    });
  } finally {
    await host.stop();
  }
  expect(problems).toEqual([]);
});

test('T037 an admin cannot retry an unknown compensation and can resolve it with retained human provenance', async ({ page }) => {
  test.setTimeout(180_000);
  const problems = failOnBrowserErrors(page);
  const { api, run, host } = await unknownCompensationRun('payment-us04-unknown-compensation');
  const reason = 'Provider support confirmed the release completed under ticket INC-4437.';
  try {
    await openRun(page, env.tokens.admin, run);
    await openStepActions(page, 'reserve~c');
    await expect(page.getByText('OUTCOME_UNKNOWN', { exact: true }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Retry compensation' })).toBeDisabled();
    await expect(page.locator('[data-action="RETRY_COMPENSATION"]')).toContainText('OUTCOME_UNKNOWN');
    await expect(page.getByRole('button', { name: 'Skip compensation' })).toBeEnabled();
    await page.getByRole('button', { name: 'Resolve step' }).click();

    const dialog = page.getByRole('dialog');
    await dialog.getByRole('radio', { name: /It happened/ }).click();
    await dialog.getByRole('textbox', { name: 'Reason' }).fill(reason);
    await dialog.getByRole('textbox', { name: 'Type the step id to confirm' }).fill('reserve~c');
    await dialog.getByRole('button', { name: 'Confirm: Resolve step' }).click();

    await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Compensated', { exact: true }).first()).toBeVisible({ timeout: 30_000 });
    expect(await api.until(run, 'COMPENSATED')).toBe('COMPENSATED');
    const after = await api.journal(run);
    expect(after.filter((event) => event.stepId === 'reserve~c' && event.eventType === 'step.leased')).toHaveLength(1);
    expect(after.some((event) => event.stepId === 'reserve~c' && event.eventType === 'step.retry_requested')).toBe(false);
    const decision = after.find((event) => event.eventType === 'step.resolved' && event.stepId === 'reserve~c');
    // Journal actors include their kind; audit actors retain the authenticated subject.
    expect(decision?.actor).toBe('user:alice');
    const audit = await resolutionAudit(api, run, 'reserve~c');
    expect(audit).toHaveLength(1);
    expect(audit[0]).toMatchObject({
      action: 'saga.step_resolved',
      actor: 'alice',
      resourceId: `${run}/reserve~c`,
      reason: `declared SUCCEEDED: ${reason}`,
    });
  } finally {
    await host.stop();
  }
  expect(problems).toEqual([]);
});
