import { expect, test } from '@playwright/test';
import type { Locator, Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';

const DESKTOP_HEIGHT = 812;

function fanOut(name: string): Record<string, unknown> {
  const branches = Array.from({ length: 6 }, (_, index) => `branch-${index + 1}`);
  return {
    ...IR_BASE,
    metadata: { name, description: 'A compact seven-node graph used to exercise the initial viewport.' },
    entrypoint: 'fan-out',
    nodes: [
      { id: 'fan-out', type: 'parallel', branches, join: 'all' },
      ...branches.map((id) => ({ id, type: 'activity', activity: `visual.${id}`, timeoutMs: 5_000 })),
    ],
  };
}

async function expectNoPageHorizontalOverflow(page: Page, where: string): Promise<void> {
  await expect.poll(
    () => page.evaluate(() => {
      const main = document.querySelector('.shell-frame > main');
      const pageOverflow = Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - window.innerWidth;
      const mainOverflow = main instanceof HTMLElement ? main.scrollWidth - main.clientWidth : 0;
      return Math.max(pageOverflow, mainOverflow);
    }),
    { message: `${where}: the page must not require horizontal scrolling` },
  ).toBeLessThanOrEqual(1);
}

async function expectHorizontallyInside(container: Locator, items: Locator, where: string): Promise<void> {
  const outer = await container.boundingBox();
  expect(outer, `${where}: container has bounds`).not.toBeNull();
  const count = await items.count();
  expect(count, `${where}: controls under test`).toBeGreaterThan(0);
  for (let index = 0; index < count; index += 1) {
    const item = items.nth(index);
    if (!(await item.isVisible())) continue;
    const inner = await item.boundingBox();
    expect(inner, `${where}: control ${index} has bounds`).not.toBeNull();
    expect(inner!.x, `${where}: control ${index} starts inside`).toBeGreaterThanOrEqual(outer!.x - 1);
    expect(inner!.x + inner!.width, `${where}: control ${index} ends inside`).toBeLessThanOrEqual(outer!.x + outer!.width + 1);
  }
}

async function expectChildrenDoNotOverlap(container: Locator, where: string): Promise<void> {
  const boxes = await container.locator(':scope > *').evaluateAll((elements) =>
    elements
      .filter((element) => {
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
      })
      .map((element) => {
        const rect = element.getBoundingClientRect();
        return { text: (element.textContent ?? '').trim().slice(0, 60), left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
      }),
  );
  for (let left = 0; left < boxes.length; left += 1) {
    for (let right = left + 1; right < boxes.length; right += 1) {
      const a = boxes[left]!;
      const b = boxes[right]!;
      const overlapWidth = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const overlapHeight = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      expect(overlapWidth > 1 && overlapHeight > 1, `${where}: “${a.text}” overlaps “${b.text}”`).toBe(false);
    }
  }
}

async function expectCanvasViewport(page: Page, canvasSelector: string, minimumScale: number): Promise<void> {
  await expect.poll(
    () => page.evaluate((selector) => {
      const canvas = document.querySelector(selector);
      const viewport = canvas?.querySelector('.react-flow__viewport');
      const controls = canvas?.querySelector('.react-flow__controls');
      if (!(canvas instanceof HTMLElement) || !(viewport instanceof HTMLElement) || !(controls instanceof HTMLElement)) return null;
      const frame = canvas.getBoundingClientRect();
      const furniture = controls.getBoundingClientRect();
      const scale = new DOMMatrixReadOnly(getComputedStyle(viewport).transform).a;
      const completeCard = Array.from(canvas.querySelectorAll('.react-flow__node')).some((node) => {
        const card = node.getBoundingClientRect();
        return card.width > 0 && card.height > 0 && card.left >= frame.left && card.right <= frame.right && card.top >= frame.top && card.bottom <= frame.bottom && card.top >= 0 && card.bottom <= window.innerHeight;
      });
      return {
        completeCard,
        scale,
        controlsInside:
          furniture.width > 0 &&
          furniture.height > 0 &&
          furniture.left >= frame.left &&
          furniture.right <= frame.right &&
          furniture.top >= frame.top &&
          furniture.bottom <= frame.bottom &&
          furniture.top >= 0 &&
          furniture.bottom <= window.innerHeight,
      };
    }, canvasSelector),
    { message: `${canvasSelector}: initial camera keeps a complete card and its controls visible` },
  ).toMatchObject({ completeCard: true, controlsInside: true });

  const scale = await page.locator(`${canvasSelector} .react-flow__viewport`).evaluate((element) => new DOMMatrixReadOnly(getComputedStyle(element).transform).a);
  expect(scale, `${canvasSelector}: readable initial zoom`).toBeGreaterThanOrEqual(minimumScale);
  expect(scale, `${canvasSelector}: initial zoom does not enlarge the graph excessively`).toBeLessThanOrEqual(1.1);
}

async function expectForcedControlInk(page: Page, control: Locator): Promise<void> {
  expect(await page.evaluate(() => matchMedia('(forced-colors: active)').matches)).toBe(true);
  const colors = await control.evaluate((element) => {
    const style = getComputedStyle(element);
    return { ink: style.color, background: style.backgroundColor };
  });
  expect(colors.ink, 'forced-colour control ink differs from its background').not.toBe(colors.background);
}

async function expectNamespaceInsideShell(page: Page): Promise<void> {
  const select = page.getByRole('combobox', { name: 'Namespace' });
  await expect(select).toHaveValue(env.namespace);
  await expectHorizontallyInside(page.locator('.shell-sidebar'), select, 'namespace selector');
}

test('VISUAL an editable workflow keeps its canvas, inspector and dialogs usable from desktop to 320px', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = 'visual-layout-editor-with-a-heading-that-wraps-cleanly';
  const created = await api.call<{ id: string }>('POST', '/workflows', { name, ir: fanOut(name) }, 201);
  await page.setViewportSize({ width: 1470, height: DESKTOP_HEIGHT });
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'none' });
  await signIn(page, env.tokens.admin);
  await goTo(page, `/workflows/${created.body.id}`);
  await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
  await expect(page.locator('.wf-canvas .react-flow__node')).toHaveCount(7);
  await expectCanvasViewport(page, '.wf-canvas', 0.85);
  await page.locator('.wf-canvas .react-flow__node[data-id="fan-out"]').click();

  const inspector = page.getByRole('complementary', { name: 'Inspector' });
  await expect(inspector.getByRole('heading').first()).not.toHaveText('Workflow settings');
  await expect(page).toHaveTitle('Sagat Studio');
  await expectNamespaceInsideShell(page);
  await expectHorizontallyInside(inspector, inspector.locator('input, select, textarea, button'), 'desktop inspector');
  expect((await page.locator('.wf-canvas').boundingBox())!.width).toBeGreaterThanOrEqual(400);
  await page.screenshot({ path: test.info().outputPath('visual-editor-light-1470x812.png'), fullPage: true });

  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  await page.screenshot({ path: test.info().outputPath('visual-editor-dark-1470x812.png'), fullPage: true });

  await page.setViewportSize({ width: 1280, height: DESKTOP_HEIGHT });
  expect((await page.locator('.wf-canvas').boundingBox())!.width).toBeGreaterThanOrEqual(360);
  await expectHorizontallyInside(inspector, inspector.locator('input, select, textarea, button'), '1280px inspector');

  for (const width of [1024, 768, 320]) {
    await page.setViewportSize({ width, height: DESKTOP_HEIGHT });
    await expectNoPageHorizontalOverflow(page, `${width}px workflow editor`);
    await expectHorizontallyInside(inspector, inspector.locator('input, select, textarea, button'), `${width}px inspector`);
    await expectNamespaceInsideShell(page);
  }

  await page.getByRole('button', { name: 'Keyboard help' }).click();
  const dialog = page.getByRole('dialog', { name: 'Keyboard help' });
  await expect(dialog).toBeVisible();
  await expectHorizontallyInside(dialog, dialog.locator('button, input, select, textarea'), '320px dialog');
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'active' });
  await expectForcedControlInk(page, dialog.getByRole('button').first());
  await expectForcedControlInk(page, page.locator('.wf-toggle .btn[aria-pressed="true"]'));
  await expectForcedControlInk(page, page.locator('.shell-mark'));
  await page.screenshot({ path: test.info().outputPath('visual-editor-forced-320x812.png'), fullPage: true });
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  expect(problems).toEqual([]);
});

test('VISUAL a seven-node run opens at a readable scale and its header never collides with its controls', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const name = 'visual-layout-run-with-a-heading-that-wraps-cleanly';
  const workflow = await api.publish(name, fanOut(name));
  const run = await api.start(workflow.id, {}, { correlationId: 'visual-layout-correlation-with-a-long-readable-value' });

  const openRun = async (width: number) => {
    await page.setViewportSize({ width, height: DESKTOP_HEIGHT });
    await goTo(page, '/runs');
    await expect(page.getByRole('heading', { name: 'Runs', level: 1 })).toBeVisible();
    await goTo(page, `/runs/${run}`);
    await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
    await expect(page.locator('.rn-canvas .react-flow__node[data-id="fan-out"]')).toBeVisible();
  };

  await page.setViewportSize({ width: 1470, height: DESKTOP_HEIGHT });
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'none' });
  await signIn(page, env.tokens.operator);
  await openRun(1470);
  await expectCanvasViewport(page, '.rn-canvas', 0.85);
  await expectChildrenDoNotOverlap(page.locator('.rn-title'), '1470px run header');

  const details = page.getByText('Run details', { exact: true });
  await details.click();
  const copyTrace = page.getByRole('button', { name: 'Copy trace id' });
  await expect(copyTrace).toBeVisible();
  await copyTrace.click();
  await expect(page.getByRole('status').filter({ hasText: /Copied trace id|Could not copy trace id/ }).first()).toBeVisible();
  await details.click();
  await expect(copyTrace).toBeHidden();

  const actions = page.getByRole('button', { name: 'Run actions' });
  await actions.focus();
  await page.keyboard.press('Enter');
  await expect(actions).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('button', { name: 'Pause run' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(actions).toHaveAttribute('aria-expanded', 'false');
  await expect(actions).toBeFocused();
  await page.screenshot({ path: test.info().outputPath('visual-run-light-1470x812.png'), fullPage: true });
  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  await page.screenshot({ path: test.info().outputPath('visual-run-dark-1470x812.png'), fullPage: true });

  await openRun(1280);
  await expectCanvasViewport(page, '.rn-canvas', 0.85);
  await expectChildrenDoNotOverlap(page.locator('.rn-title'), '1280px run header');

  await openRun(320);
  await expectNoPageHorizontalOverflow(page, '320px run detail');
  await expectChildrenDoNotOverlap(page.locator('.rn-title'), '320px run header');
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'active' });
  await expectForcedControlInk(page, page.locator('.rn-canvas .react-flow__controls button').first());
  await page.screenshot({ path: test.info().outputPath('visual-run-forced-320x812.png'), fullPage: true });
  expect(problems).toEqual([]);
});

test('VISUAL a populated Scripts list remains reachable at 320px', async ({ page }) => {
  const problems = failOnBrowserErrors(page);
  await page.setViewportSize({ width: 1280, height: DESKTOP_HEIGHT });
  await signIn(page, env.tokens.admin);
  await page.getByRole('link', { name: 'Scripts' }).click();
  await page.getByRole('button', { name: 'New script' }).click();
  await page.getByRole('dialog', { name: 'New script' }).getByRole('textbox', { name: 'Name' }).fill('visual-layout-script-with-a-long-name');
  await page.getByRole('dialog', { name: 'New script' }).getByRole('button', { name: 'Create script' }).click();
  await expect(page.getByRole('heading', { name: 'visual-layout-script-with-a-long-name', level: 1 })).toBeVisible();
  await page.getByRole('link', { name: 'All scripts' }).click();
  const table = page.getByRole('table', { name: 'Scripts in the namespace, by name' });
  await expect(table).toContainText('visual-layout-script-with-a-long-name');

  await page.setViewportSize({ width: 320, height: DESKTOP_HEIGHT });
  await expectNoPageHorizontalOverflow(page, '320px populated Scripts list');
  const reachable = await table.evaluate((element) => {
    if (element.scrollWidth <= window.innerWidth + 1) return true;
    for (let parent = element.parentElement; parent !== null; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      if (['auto', 'scroll'].includes(style.overflowX) && parent.scrollWidth > parent.clientWidth) return parent.getBoundingClientRect().width <= window.innerWidth + 1;
    }
    return false;
  });
  expect(reachable, 'the wide Scripts table has a horizontal scroll container instead of clipped columns').toBe(true);
  await expectNamespaceInsideShell(page);
  await page.screenshot({ path: test.info().outputPath('visual-scripts-populated-light-320x812.png'), fullPage: true });
  expect(problems).toEqual([]);
});
