import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import type { ActivityContext } from '@sagat-octopus-orchestrator/sdk';
import { env } from '../support/env';
import { expectAccessible } from '../support/accessibility';
import { activate, choose, chooseRadio, closeWithEscape, inspectDialog, landedOn, openDialog, openFromMenu, setChecked, signInByKeyboard, typeInto } from '../support/keyboard';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors } from '../support/studio';
import { bindWorker, cancelAfterwards, cancelLeftovers, openWorkflow, ownNamespace, recordRefusals, setQuota, unique, withoutRefusals } from '../support/world';
import { ActivityFailure, startWorker } from '../support/worker';

/**
 * The reset of a run in the Studio in a real browser, with the keyboard alone and the accessibility rules on every state
 * (T018-074, FR-018-047, SC-018-015). A run that ended is reset to a point at once; a run that has not ended is cancelled and
 * reset when it has (`CANCEL_SOURCE`), which the engine's reset pass completes (`SAGAT_RESET_CANCEL_SOURCE_ENABLED`, on in
 * tests/e2e/workflow-studio). The wait that ends in `SOURCE_DID_NOT_END` is an hour at the least and is not waited for here: its
 * words are tested in `ResetRun.test.tsx`.
 */
test.use({ viewport: { width: 1470, height: 900 } });
test.afterEach(cancelLeftovers);

/**
 * How long the attempt a journey holds in flight may take before the platform times it out: above the budget of any journey (240 s),
 * so that however slow the machine the held attempt is let go by the journey, not ended by its timeout (which would be an unknown
 * outcome and a run in review, not a cancelled one).
 */
const HOLD_MS = 600_000;
/** What the engine's passes are given to do (the reset pass polls every 100 ms; a loaded machine is slower): a bound, not a wait. */
const ENGINE = { timeout: 60_000 };

/** Three steps in a row (a, b, c), with the activities of the journey's own tag so that no other worker serves them. */
function threeSteps(name: string, tag: string, hold = false): Record<string, unknown> {
  const step = (id: string, next: string) => ({ id, type: 'activity', activity: `${tag}.${id}`, timeoutMs: hold ? HOLD_MS : 5000, retry: { maxAttempts: 1 }, next });
  return {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'a',
    triggers: [{ kind: 'manual' }],
    nodes: [step('a', 'b'), step('b', 'c'), step('c', 'done'), { id: 'done', type: 'end' }],
  };
}

/** The page of a run, opened the way a person does: the run list, narrowed to the workflow, and the link of the run. */
async function openRun(page: Page, workflow: string, runId: string): Promise<void> {
  await openFromMenu(page, 'Runs', 'Runs');
  await choose(page, page.getByLabel('Workflow', { exact: true }), workflow);
  await activate(page, page.getByRole('link', { name: runId, exact: true }));
  await expect(page.getByRole('heading', { level: 1 })).toContainText(runId);
}

test('RESET a run that ended is reset to a point: the points and the plan are read in words, the acknowledgement is asked for, and the new run links back to its source', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = unique('reset-ended');
  const tag = name;
  const workflow = await api.publish(name, threeSteps(name, tag));
  let againStep = 0;
  const host = await startWorker(`${name}-worker`, {
    [`${tag}.a`]: () => ({ a: true }),
    [`${tag}.b`]: () => ({ b: true }),
    [`${tag}.c`]: () => {
      // the third step is declined the first time, and goes through when the new run runs it again
      againStep += 1;
      if (againStep === 1) throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer');
      return { c: true };
    },
  });
  // the worker stays up for the new run: it is stopped at the end of the journey
  try {
    const source = await api.start(workflow.id, { order: 'o-1' });
    await api.until(source, 'FAILED');
    await signInByKeyboard(page, env.tokens.admin, env.namespace);
    await openRun(page, name, source);
    await expectAccessible(page, 'the run that ended');

    // The dialog: focus goes in, the points are in words, nothing is asked of the platform before the person has read them.
    const opener = page.getByRole('button', { name: 'Reset to a point…', exact: true });
    const dialog = await openDialog(page, opener, 'Reset to a point');
    const points = dialog.getByRole('group', { name: 'Reset to' });
    await expect(points.getByRole('radio')).toHaveCount(3);
    await expect(points.getByRole('radio', { name: /^The start of the run/ })).toBeVisible();
    await expect(points.getByRole('radio', { name: /^After step a/ })).toBeVisible();
    await expect(points.getByRole('radio', { name: /^After step b/ })).toBeVisible();
    await expectAccessible(page, 'the reset dialog, points read');

    // Confirming before choosing: told, in words, what is missing.
    await activate(page, dialog.getByRole('button', { name: 'Confirm: reset the run' }));
    await expect(dialog.getByRole('alert').filter({ hasText: 'Choose where to reset the run to.' })).toBeVisible();
    await expectAccessible(page, 'the reset dialog, no point chosen');
    await closeWithEscape(page, dialog, opener);

    // The plan of a point after step a: b and c run again, b completed in this run, c had not.
    await openDialog(page, opener, 'Reset to a point');
    await chooseRadio(page, points.getByRole('radio', { name: /^After step a/ }));
    const plan = dialog.getByRole('region', { name: 'What the reset does' });
    await expect(plan.getByText(/^Kept as it was: a$/)).toBeVisible();
    await expect(plan.getByRole('listitem').filter({ hasText: /^b \(it completed in this run: running it again repeats what it did\)/ })).toBeVisible();
    await expect(plan.getByRole('listitem').filter({ hasText: /^c \(it had not completed\)/ })).toBeVisible();
    await expectAccessible(page, 'the reset dialog, plan read');

    // The acknowledgement and the reason are asked for in words, then given.
    await activate(page, dialog.getByRole('button', { name: 'Confirm: reset the run' }));
    await expect(dialog.getByRole('alert').filter({ hasText: 'Confirm that you know these steps will run again.' })).toBeVisible();
    await expect(dialog.getByRole('alert').filter({ hasText: /8 to 2000|at least 8/ })).toBeVisible();
    await expectAccessible(page, 'the reset dialog, acknowledgement and reason missing');
    await setChecked(page, dialog.getByRole('checkbox', { name: /I know that these steps ran before/ }), true);
    await typeInto(page, dialog.getByLabel('Reason'), 'the issuer fixed the decline, run the payment again');
    const order = await inspectDialog(page, dialog);
    // Tab visits the radio that is chosen of each group (the arrows are for the others)
    expect(order.some((reached) => reached.startsWith('radio: After step a'))).toBe(true);
    expect(order.some((reached) => reached.startsWith('radio: Nothing: the new run does not receive them'))).toBe(true);
    expect(order).toEqual(expect.arrayContaining(['checkbox: I know that these steps ran before and will run again, and that running them again repeats what they did.', 'textbox: Reason', 'button: Cancel', 'button: Confirm: reset the run']));
    await activate(page, dialog.getByRole('button', { name: 'Confirm: reset the run' }));

    // The new run: the person lands on it, it says where it came from, and it finishes now that the issuer accepts.
    await expect(dialog).toBeHidden();
    const made = await landedOn(page, source);
    await expect(page.getByRole('heading', { level: 1 })).toContainText(made);
    // a reset says it is one, and how much of the source it keeps (`resetFrom`), in place of "started again" (G2057)
    await expect(page.getByText('Reset from')).toBeVisible();
    await expect(page.getByText('Started again from')).toBeHidden();
    await expect(page.getByRole('link', { name: /^run / }).and(page.locator(`a[href$="/runs/${source}"]`))).toBeVisible();
    await api.until(made, 'SUCCEEDED');
    const created = (await api.call<{ rerunOf?: string; resetFrom?: { runId: string; sequence: number } }>('GET', `/runs/${made}`, undefined, 200)).body;
    expect(created.rerunOf).toBe(source);
    expect(created.resetFrom).toMatchObject({ runId: source });
    await expect(page.getByText(new RegExp(`this run keeps the first ${created.resetFrom!.sequence} events? of it`))).toBeVisible();
    await expectAccessible(page, 'the new run, with the link back to its source');

    // The source says what was made from it, with a link to it.
    await activate(page, page.getByRole('link', { name: /^run / }).and(page.locator(`a[href$="/runs/${source}"]`)));
    await expect(page.getByRole('heading', { level: 1 })).toContainText(source);
    const resets = page.getByRole('region', { name: 'Resets made from this run' });
    await expect(resets.getByRole('link', { name: /^A run reset to point/ })).toHaveAttribute('href', new RegExp(`/runs/${made}$`));
    await expect(resets).toContainText('created');
    await expectAccessible(page, 'the source run, with the reset made from it');
    expect(problems).toEqual([]);
  } finally {
    await host.stop();
  }
});

test('RESET a reset the platform refuses is read in words: the business identifier is held by another open run', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const refusals = recordRefusals(page);
  const api = new Api('admin');
  const name = unique('reset-refused');
  const tag = name;
  const workflow = await api.publish(name, threeSteps(name, tag));
  const holderName = unique('reset-holder');
  // nobody serves the activities of this one: its runs stay open
  const holder = await api.publish(holderName, threeSteps(holderName, holderName));
  const host = await startWorker(`${name}-worker`, {
    [`${tag}.a`]: () => ({ a: true }),
    [`${tag}.b`]: () => ({ b: true }),
    [`${tag}.c`]: () => { throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer'); },
  });
  try {
    const businessId = unique('order');
    const source = await api.start(workflow.id, { order: 'o-2' }, { businessId });
    await api.until(source, 'FAILED');
    // the identifier is free once the run has ended: another run takes it and stays open (nobody serves its first step)
    const other = await api.start(holder.id, { order: 'o-3' }, { businessId });
    cancelAfterwards(api, other);
    expect(other).not.toBe(source);

    await signInByKeyboard(page, env.tokens.admin, env.namespace);
    await openRun(page, name, source);
    await expect(page.getByText('Business id', { exact: true })).toBeVisible();
    const dialog = await openDialog(page, page.getByRole('button', { name: 'Reset to a point…', exact: true }), 'Reset to a point');
    const keep = dialog.getByRole('radio', { name: /^Keep it on the new run/ });
    await expect(keep).toBeChecked();
    await expect(dialog.getByRole('group', { name: `Business identifier ${businessId}` })).toBeVisible();
    await chooseRadio(page, dialog.getByRole('radio', { name: /^After step b/ }));
    await expect(dialog.getByRole('region', { name: 'What the reset does' }).getByText(/^Kept as it was: a, b$/)).toBeVisible();
    await typeInto(page, dialog.getByLabel('Reason'), 'take the order again from the declined step');
    await activate(page, dialog.getByRole('button', { name: 'Confirm: reset the run' }));
    const refusal = dialog.getByRole('alert').filter({ hasText: 'Resetting the run failed.' });
    await expect(refusal).toBeVisible();
    await expect(refusal).toContainText('BUSINESS_ID_IN_USE');
    await expectAccessible(page, 'the reset dialog, refused: the identifier is held');
    // nothing was made, and the person is still in the dialog, where the way out is to start the new run without the identifier
    expect((await api.call<{ items: unknown[] }>('GET', `/runs/${source}/resets`, undefined, 200)).body.items).toHaveLength(0);
    await chooseRadio(page, dialog.getByRole('radio', { name: /^Start the new run without it/ }));
    await activate(page, dialog.getByRole('button', { name: 'Confirm: reset the run' }));
    await expect(dialog).toBeHidden();
    const made = await landedOn(page, source);
    // its first step waits for the worker that stops with the journey: it is cancelled with the other run this journey leaves
    cancelAfterwards(api, made);
    await expect(page.getByRole('heading', { level: 1 })).toContainText(made);
    expect((await api.call<{ businessId?: string | null }>('GET', `/runs/${made}`, undefined, 200)).body.businessId ?? null).toBeNull();
    expect(withoutRefusals(problems, refusals, [409])).toEqual([]);
  } finally {
    await host.stop();
  }
});

test('RESET a run that has not ended is cancelled and reset when it ends: the entry waits, says so, and is completed by the engine, with a link to the run it made', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = unique('reset-open');
  const tag = name;
  const workflow = await api.publish(name, threeSteps(name, tag, true));
  // The first attempt of step a is in flight and stays so until the journey lets it go: the run cannot end meanwhile.
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let inFlight!: () => void;
  const started = new Promise<void>((resolve) => { inFlight = resolve; });
  // The first attempt of the first run this worker is given is the one held: the run the reset makes runs the step too, and goes through.
  let held: { run: string; attempt: number } | undefined;
  const host = await startWorker(`${name}-worker`, {
    [`${tag}.a`]: async (_input: unknown, ctx: ActivityContext) => {
      held ??= { run: ctx.runId, attempt: ctx.attempt };
      if (ctx.runId === held.run && ctx.attempt === held.attempt) {
        inFlight();
        await gate;
      }
      return { a: true };
    },
    [`${tag}.b`]: () => ({ b: true }),
    [`${tag}.c`]: () => ({ c: true }),
  });
  try {
    const source = await api.start(workflow.id, { order: 'o-4' });
    await started;

    await signInByKeyboard(page, env.tokens.admin, env.namespace);
    await openRun(page, name, source);
    const opener = page.getByRole('button', { name: 'Cancel and reset to a point…', exact: true });
    await expect(page.getByRole('button', { name: 'Reset to a point…', exact: true })).toHaveCount(0);
    const dialog = await openDialog(page, opener, 'Cancel the run and reset it when it ends');
    await expect(dialog.getByRole('alert').filter({ hasText: 'This run has not ended.' })).toBeVisible();
    await expectAccessible(page, 'the cancel-and-reset dialog');
    await chooseRadio(page, dialog.getByRole('radio', { name: /^The start of the run/ }));
    await expect(dialog.getByRole('region', { name: 'What the reset does' })).toContainText('Kept as it was: nothing; the new run starts from the beginning.');
    expect(await inspectDialog(page, dialog)).toEqual(expect.arrayContaining(['checkbox: I know that steps that completed after the point, including any that complete before this run ends, will run again, and that running them again repeats what they did.', 'textbox: Reason', 'button: Do not cancel the run']));
    await typeInto(page, dialog.getByLabel('Reason'), 'the order was entered wrong, start it again');
    // The acknowledgement is always asked for here: the Studio cannot tell which step completes while the run is cancelled.
    await activate(page, dialog.getByRole('button', { name: 'Confirm: cancel the run and reset it when it ends' }));
    await expect(dialog.getByRole('alert').filter({ hasText: 'Confirm that you know steps may run again.' })).toBeVisible();
    await expectAccessible(page, 'the cancel-and-reset dialog, acknowledgement missing');
    await setChecked(page, dialog.getByRole('checkbox', { name: /I know that steps that completed after the point/ }), true);
    await expect(dialog.getByText('The Studio cannot tell which steps will complete while the run is being cancelled')).toBeVisible();
    await activate(page, dialog.getByRole('button', { name: 'Confirm: cancel the run and reset it when it ends' }));

    // The entry waits for the run to end, and the dialog says that nothing was made yet.
    await expect(dialog.getByText('The run is being cancelled.')).toBeVisible();
    await expectAccessible(page, 'the cancel-and-reset dialog, waiting for the run');
    await activate(page, dialog.getByRole('button', { name: 'Close' }));
    await expect(dialog).toBeHidden();
    await expect(opener).toBeFocused();
    const resets = page.getByRole('region', { name: 'Resets made from this run' });
    await expect(resets).toContainText('A reset to point');
    await expect(resets).toContainText('waiting for this run to end');
    await expect(resets.getByRole('link')).toHaveCount(0);
    await expectAccessible(page, 'the run being cancelled, with its reset waiting');
    const waiting = (await api.call<{ items: Array<{ state: string; newRunId: string | null }> }>('GET', `/runs/${source}/resets`, undefined, 200)).body.items;
    expect(waiting).toMatchObject([{ state: 'PENDING_SOURCE', newRunId: null }]);

    // The attempt in flight finishes, the run ends cancelled, and the engine makes the new run.
    release();
    await api.until(source, 'CANCELLED');
    await expect.poll(async () => (await api.call<{ items: Array<{ state: string; newRunId: string | null }> }>('GET', `/runs/${source}/resets`, undefined, 200)).body.items[0]?.state, ENGINE).toBe('CREATED');
    const made = (await api.call<{ items: Array<{ newRunId: string }> }>('GET', `/runs/${source}/resets`, undefined, 200)).body.items[0]!.newRunId;
    await expect(page.getByRole('heading', { level: 1 })).toContainText(source);
    // Nothing tells this page that the engine made the run: it reads the list again when the person comes back to the tab
    // (the Studio's design, `ResetsMade`), which the journey does with the event the browser sends then.
    await page.evaluate(() => window.dispatchEvent(new Event('visibilitychange')));
    await expect(resets.getByRole('link', { name: /^A run reset to point/ })).toHaveAttribute('href', new RegExp(`/runs/${made}$`));
    await expect(resets).toContainText('created');
    await expectAccessible(page, 'the cancelled run, with the reset made from it');
    await activate(page, resets.getByRole('link', { name: /^A run reset to point/ }));
    await expect(page.getByRole('heading', { level: 1 })).toContainText(made);
    await expect(page.getByText('Reset from')).toBeVisible();
    await api.until(made, 'SUCCEEDED');
    expect(problems).toEqual([]);
  } finally {
    release();
    await host.stop();
  }
});

test('RESET a reset that waited for its run and could not be made says why in words, and a namespace with no room for another waiting reset refuses with the Studio\'s own sentence', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const refusals = recordRefusals(page);
  const admin = new Api('admin');
  const own = await ownNamespace(admin, 'reset-failed');
  const api = new Api('admin', own.slug);
  const name = unique('reset-quota');
  const activity = `${name}.hold`;
  await bindWorker(admin, own.id, [activity]);
  const workflow = await api.publish(name, {
    ...IR_BASE,
    metadata: { name },
    entrypoint: 'hold',
    triggers: [{ kind: 'manual' }],
    nodes: [{ id: 'hold', type: 'activity', activity, timeoutMs: HOLD_MS, retry: { maxAttempts: 1 }, next: 'done' }, { id: 'done', type: 'end' }],
  });
  const bystanderName = unique('reset-bystander');
  const bystanderWorkflow = await openWorkflow(api, bystanderName);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let inFlight!: () => void;
  const started = new Promise<void>((resolve) => { inFlight = resolve; });
  const host = await startWorker(`${name}-worker`, {
    [activity]: async () => {
      inFlight();
      await gate;
      return { held: true };
    },
  }, {}, own.slug);
  try {
    const source = await api.start(workflow.id, {});
    await started;
    // another run of the namespace is open too, and the namespace is then allowed one active run: when the source has ended
    // that one still holds the place, and there is none for the run the reset would make
    const bystander = await api.start(bystanderWorkflow.id, {});
    cancelAfterwards(api, bystander);
    await setQuota(admin, own.id, 'active_runs', 1);

    await signInByKeyboard(page, env.tokens.admin, own.slug);
    await openRun(page, name, source);
    const dialog = await openDialog(page, page.getByRole('button', { name: 'Cancel and reset to a point…', exact: true }), 'Cancel the run and reset it when it ends');
    await chooseRadio(page, dialog.getByRole('radio', { name: /^The start of the run/ }));
    await typeInto(page, dialog.getByLabel('Reason'), 'start it again once the place is free');
    await setChecked(page, dialog.getByRole('checkbox', { name: /I know that steps that completed after the point/ }), true);
    await activate(page, dialog.getByRole('button', { name: 'Confirm: cancel the run and reset it when it ends' }));
    await expect(dialog.getByText('The run is being cancelled.')).toBeVisible();
    await activate(page, dialog.getByRole('button', { name: 'Close' }));
    await expect(dialog).toBeHidden();

    release();
    await api.until(source, 'CANCELLED');
    await expect.poll(async () => (await api.call<{ items: Array<{ state: string }> }>('GET', `/runs/${source}/resets`, undefined, 200)).body.items[0]?.state, ENGINE).toBe('FAILED');
    const entry = (await api.call<{ items: Array<{ failureReason?: string; newRunId: string | null }> }>('GET', `/runs/${source}/resets`, undefined, 200)).body.items[0]!;
    expect(entry.newRunId).toBeNull();
    // the Studio says it in words, not with the name of the reason
    const resets = page.getByRole('region', { name: 'Resets made from this run' });
    await page.evaluate(() => window.dispatchEvent(new Event('visibilitychange')));
    await expect(resets).toContainText('could not be made:');
    await expect(resets).toContainText('a quota or the platform refused the new run');
    await expect(resets).not.toContainText(entry.failureReason ?? 'ADMISSION_REFUSED');
    await expect(resets.getByRole('link')).toHaveCount(0);
    await expectAccessible(page, 'the cancelled run, with the reset that could not be made');
    expect(entry.failureReason).toBe('ADMISSION_REFUSED');

    // No room for another waiting reset: the person is told so in the Studio's words, and the other run is not cancelled.
    await setQuota(admin, own.id, 'pending_resets', 0);
    await openRun(page, bystanderName, bystander);
    const another = await openDialog(page, page.getByRole('button', { name: 'Cancel and reset to a point…', exact: true }), 'Cancel the run and reset it when it ends');
    await chooseRadio(page, another.getByRole('radio', { name: /^The start of the run/ }));
    await typeInto(page, another.getByLabel('Reason'), 'one more waiting reset than the namespace may have');
    await setChecked(page, another.getByRole('checkbox', { name: /I know that steps that completed after the point/ }), true);
    await activate(page, another.getByRole('button', { name: 'Confirm: cancel the run and reset it when it ends' }));
    const refusal = another.getByRole('alert').filter({ hasText: 'Resetting the run failed.' });
    await expect(refusal).toContainText('This namespace already has as many resets waiting for a run to end as it may.');
    await expect(refusal).toContainText('Wait until one of them has been made or has failed, then try again.');
    await expect(refusal).toContainText('PENDING_RESET_LIMIT');
    await expectAccessible(page, 'the cancel-and-reset dialog, no room for another waiting reset');
    expect((await api.run(bystander)).status).not.toBe('CANCELLING');
    expect((await api.call<{ items: unknown[] }>('GET', `/runs/${bystander}/resets`, undefined, 200)).body.items).toHaveLength(0);
    expect(withoutRefusals(problems, refusals, [429])).toEqual([]);
  } finally {
    release();
    await host.stop();
  }
});
