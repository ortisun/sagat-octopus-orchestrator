import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { scriptsConfig } from './config';
import type { ScriptVersion, ScriptVersionSummary } from './api';
import type { scriptsServer} from './testing';
import { DIGEST, DEVELOPER, VIEWER, aRunSummary, aScript, aVersion, aVersionSummary, json, problem, stubStorage } from './testing';
import { alerts, openEditor, patchesOf } from './testingEditor';
import { eventually, jump, manualTime, restoreTime } from './testingTime';

vi.mock('./editorModule', () => ({ loadEditor: () => Promise.reject(new Error('no Monaco in jsdom')) }));

beforeEach(() => {
  stubStorage();
  scriptsConfig.autosaveMs = 30;
  manualTime();
});
afterEach(() => {
  scriptsConfig.autosaveMs = 1500;
  restoreTime();
  vi.unstubAllGlobals();
});


const sourceBox = () => eventually(() => screen.getByRole('textbox', { name: 'Source of double-it (TypeScript)' }));

type Server = ReturnType<typeof scriptsServer>;

/** The platform builds version 1 and answers each look at it with `looks` in order, the last for ever. */
function publishing(server: Server, looks: ScriptVersion[], created = true) {
  let n = 0;
  const first = looks[0] ?? aVersion(1, 'BUILDING');
  server.override(`POST ${server.paths.one}/versions`, () => json(created ? 202 : 200, { version: first, created }));
  server.override(`GET ${server.paths.one}/versions/${first.number}`, () => looks[Math.min(n++, looks.length - 1)]);
  return { looks: () => server.callsTo('GET', `${server.paths.one}/versions/${first.number}`).length };
}

const ready = (number = 1, extra: Partial<ScriptVersion> = {}) => aVersion(number, 'READY', { builtAt: '2026-10-10T10:00:05Z', ...extra });
const building = (number = 1) => aVersion(number, 'BUILDING');

async function openDialog(permissions: string[] = DEVELOPER, options: Parameters<typeof scriptsServer>[0] = {}) {
  const ctx = await openEditor(permissions, options);
  const user = userEvent.setup({ delay: null });
  await user.click(await eventually(() => screen.getByRole('button', { name: 'Publish version…' })));
  return { ...ctx, user, dialog: screen.getByRole('dialog', { name: 'Publish a version' }) };
}

describe('Publishing a version', () => {
  it('is for those who may publish, and for nobody else', async () => {
    await openEditor(VIEWER);
    await sourceBox();
    expect(screen.queryByRole('button', { name: 'Publish version…' })).not.toBeInTheDocument();
    expect(screen.getByText(/Publishing needs the permission script:publish/)).toBeInTheDocument();
  });

  it('warns that the draft was never tried, but does not stop the person', async () => {
    const { dialog } = await openDialog();
    expect(within(dialog).getByText('Not tested.')).toBeInTheDocument();
    expect(dialog).toHaveTextContent('This draft has not been tried in the sandbox.');
    expect(dialog).toHaveTextContent('The platform does not require a test run before publishing');
    expect(within(dialog).getByRole('button', { name: 'Publish version' })).toBeEnabled();
    expect(dialog).toHaveTextContent('TypeScript types are erased');
  });

  it('is calm about a draft that was tried and passed since its last change', async () => {
    const { dialog } = await openDialog(DEVELOPER, { runs: [aRunSummary({ status: 'SUCCEEDED', createdAt: '2026-10-10T09:30:00Z' })] });
    await eventually(() => expect(dialog).toHaveTextContent('A test run of this draft succeeded'));
    expect(within(dialog).queryByText('Not tested.')).not.toBeInTheDocument();
  });

  it('says a draft changed after its last test run is not tested', async () => {
    const { dialog } = await openDialog(DEVELOPER, { runs: [aRunSummary({ status: 'SUCCEEDED', createdAt: '2026-10-10T08:00:00Z' })] });
    await eventually(() => expect(dialog).toHaveTextContent('The draft changed after the last test run.'));
  });

  it('publishes, follows the build, and then shows the digest to use in a Script node, with a button to copy it', async () => {
    const { server, user, dialog } = await openDialog();
    const builds = publishing(server, [building(), building(), ready()]);
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    expect(await eventually(() => within(dialog).getByText('Building'))).toBeInTheDocument();
    expect(dialog).toHaveTextContent('Version 1');
    expect(dialog).not.toHaveTextContent(DIGEST);
    // Looked at once a second: still building, still building, ready.
    await jump(1000);
    await jump(1000);
    expect(within(dialog).getByText('Building')).toBeInTheDocument();
    await jump(1000);
    expect(await eventually(() => within(dialog).getByText('Ready'))).toBeInTheDocument();
    expect(dialog).toHaveTextContent(DIGEST);
    expect(dialog).toHaveTextContent('Use this digest in a Script node of a workflow.');
    expect(dialog).toHaveTextContent('Workers bind it by its activity name');
    // It is looked at no more once it is ready.
    const looked = builds.looks();
    expect(looked).toBeGreaterThanOrEqual(3);
    await jump(20_000);
    expect(builds.looks()).toBe(looked);
    await user.click(within(dialog).getByRole('button', { name: 'Copy digest' }));
    // What is on the clipboard is exactly the digest.
    expect(await navigator.clipboard.readText()).toBe(DIGEST);
    expect(await eventually(() => screen.getByText('Copied the digest of version 1.'))).toBeInTheDocument();
  });

  it('says it when the browser will not copy', async () => {
    const { server, user, dialog } = await openDialog();
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
    publishing(server, [ready()]);
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    await user.click(await eventually(() => within(dialog).getByRole('button', { name: 'Copy digest' })));
    expect(await eventually(() => within(dialog).getByText(/The browser did not allow copying/))).toBeInTheDocument();
    expect(dialog).toHaveTextContent(DIGEST);
  });

  it('saves the draft first and checks the platform\'s copy is the author\'s before it publishes', async () => {
    const { server, user, dialog } = await openDialog();
    scriptsConfig.autosaveMs = 60_000;
    publishing(server, [ready()]);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await user.type(await sourceBox(), '// new');
    await user.click(screen.getByRole('button', { name: 'Publish version…' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Publish a version' })).getByRole('button', { name: 'Publish version' }));
    await eventually(() => expect(server.callsTo('POST', `${server.paths.one}/versions`)).toHaveLength(1));
    const writes = server.calls.filter((c) => c.method !== 'GET').map((c) => c.method);
    expect(writes).toEqual(['PATCH', 'POST']);
    expect(String(patchesOf(server)[0]?.['source'])).toContain('// new');
  });

  it('does not publish when the draft cannot be saved first', async () => {
    const { server, user, dialog } = await openDialog();
    server.override(`PATCH ${server.paths.one}`, () => problem(422, 'VALIDATION_FAILED', 'the source is empty'));
    publishing(server, [ready()]);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await user.type(await sourceBox(), 'x');
    await user.click(screen.getByRole('button', { name: 'Publish version…' }));
    const again = screen.getByRole('dialog', { name: 'Publish a version' });
    await user.click(within(again).getByRole('button', { name: 'Publish version' }));
    expect(await eventually(() => within(again).getByText(/nothing was published/))).toBeInTheDocument();
    expect(server.callsTo('POST', `${server.paths.one}/versions`)).toHaveLength(0);
  });

  it('does not publish a draft that someone else has changed under the author: it would publish theirs', async () => {
    const { server, user, dialog } = await openDialog();
    publishing(server, [ready()]);
    server.state.script = aScript({ revision: 3, source: 'their code', updatedAt: '2026-10-10T11:00:00Z' });
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    expect(await eventually(() => within(dialog).getByText(/Someone else changed the draft after you last saved/))).toBeInTheDocument();
    expect(server.callsTo('POST', `${server.paths.one}/versions`)).toHaveLength(0);
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(await eventually(() => screen.getByRole('heading', { name: 'Someone else changed this draft' }))).toBeInTheDocument();
  });

  it('shows a build that failed with its reason and its log, and says nothing can use it', async () => {
    const { server, user, dialog } = await openDialog();
    publishing(server, [aVersion(1, 'FAILED', { failure: 'rejected: the script must export a default function', buildLog: 'SyntaxError: nope\n' })]);
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    expect(await eventually(() => within(dialog).getByText('Build failed'))).toBeInTheDocument();
    expect(alerts()).toContain('rejected: the script must export a default function');
    expect(alerts()).toContain('Nothing can use this version');
    expect(within(dialog).getByRole('region', { name: 'Build log' })).toHaveTextContent('SyntaxError: nope');
    expect(dialog).not.toHaveTextContent('Artifact digest');
  });

  it('says when the draft is the same as the latest version, so no new one was made', async () => {
    const { server, user, dialog } = await openDialog(DEVELOPER, { versions: [aVersionSummary(2)] });
    publishing(server, [ready(2)], false);
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    expect(await eventually(() => within(dialog).getByText(/the draft is the same as this version, so no new version was made/))).toBeInTheDocument();
    expect(dialog).toHaveTextContent('Version 2');
  });

  it('shows the refusal of the platform, with its trace id, and lets the person try again', async () => {
    const { server, user, dialog } = await openDialog();
    server.override(`POST ${server.paths.one}/versions`, () => problem(429, 'QUOTA_EXCEEDED', 'a limit was reached; release something and retry', { limit: 'script_jobs' }));
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    expect(await eventually(() => within(dialog).getByText('Publishing failed.'))).toBeInTheDocument();
    const text = within(dialog).getAllByRole('alert').map((alert) => alert.textContent).join(' ');
    expect(text).toContain('a limit was reached');
    expect(text).toContain('a'.repeat(32));
    expect(within(dialog).getByRole('button', { name: 'Publish version' })).toBeEnabled();
  });

  it('gives up looking at a build that does not end, and says the version stays in the list', async () => {
    const { server, user, dialog } = await openDialog();
    const builds = publishing(server, [building()]);
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    await eventually(() => within(dialog).getByText('Building'));
    await jump(11 * 60_000);
    expect(await eventually(() => within(dialog).getByText('The build has not finished.'))).toBeInTheDocument();
    const looks = builds.looks();
    await jump(10 * 60_000);
    expect(builds.looks()).toBe(looks);
  });

  it('stops looking at the build when the dialog is closed (the list goes on looking)', async () => {
    const { server, user, dialog } = await openDialog();
    const builds = publishing(server, [building()]);
    await user.click(within(dialog).getByRole('button', { name: 'Publish version' }));
    await eventually(() => within(dialog).getByText('Building'));
    await jump(2000);
    await user.click(within(dialog).getByRole('button', { name: 'Close' }));
    const looks = builds.looks();
    await jump(30_000);
    expect(builds.looks()).toBe(looks);
  });
});

describe('Versions', () => {
  const list: ScriptVersionSummary[] = [aVersionSummary(3, 'BUILDING'), aVersionSummary(2, 'FAILED', { failure: 'lockfile: pins left-pad' }), aVersionSummary(1)];

  it('lists them newest first with their state and digest', async () => {
    const { user } = await openEditor(DEVELOPER, { versions: list });
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Versions' })));
    const table = await eventually(() => screen.getByRole('table', { name: 'Versions of this script, newest first' }));
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows.map((row) => within(row).getAllByRole('rowheader')[0]?.textContent)).toEqual(['3', '2', '1']);
    expect(rows[0]).toHaveTextContent('Building');
    expect(rows[0]).toHaveTextContent('not built yet');
    expect(rows[1]).toHaveTextContent('Build failed');
    expect(rows[1]).toHaveTextContent('lockfile: pins left-pad');
    expect(rows[2]).toHaveTextContent('Ready');
    expect(rows[2]).toHaveTextContent(`${DIGEST.slice(0, 19)}…`);
    expect(screen.getByText(/latest version 3/)).toBeInTheDocument();
  });

  it('follows a build in the list until it ends, and then stops', async () => {
    const { server, user } = await openEditor(DEVELOPER, { versions: [aVersionSummary(1, 'BUILDING')] });
    const reads = () => server.callsTo('GET', `${server.paths.one}/versions`).length;
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Versions' })));
    await eventually(() => screen.getByText('Building'));
    const before = reads();
    server.state.versions = [aVersionSummary(1, 'READY')];
    await jump(1000);
    expect(await eventually(() => screen.getByText('Ready'))).toBeInTheDocument();
    const after = reads();
    expect(after).toBeGreaterThan(before);
    await jump(60_000);
    expect(reads()).toBe(after);
  });

  it('opens one read-only, as it was published, and goes back to the draft', async () => {
    const { server, user } = await openEditor(DEVELOPER, { versions: [aVersionSummary(1)] });
    server.override(`GET ${server.paths.one}/versions/1`, () => aVersion(1, 'READY', { source: 'export default () => "as published";\n', buildLog: 'built in 3 s\n', inputSchema: { type: 'object' } as never, lockfile: '' }));
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Versions' })));
    await user.click(await eventually(() => screen.getByRole('button', { name: 'Open version 1 read-only' })));
    expect(await eventually(() => screen.getByRole('heading', { name: /Version 1/ }))).toHaveFocus();
    expect(screen.getByText(/A version never changes; your draft is untouched/)).toBeInTheDocument();
    const source = await eventually(() => screen.getByRole('textbox', { name: 'Source of version 1 (read-only)' }));
    expect(source).toHaveValue('export default () => "as published";\n');
    expect(source).toHaveAttribute('readonly');
    expect(screen.getByRole('region', { name: 'Build log of version 1' })).toHaveTextContent('built in 3 s');
    expect(screen.getByRole('region', { name: 'Input schema' })).toHaveTextContent('"type": "object"');
    expect(screen.getByText(DIGEST)).toBeInTheDocument();
    expect(screen.getByText('Use this digest in a Script node of a workflow.')).toBeInTheDocument();
    // The draft is not in the way, and is back when asked for.
    expect(screen.queryByRole('textbox', { name: /Source of double-it/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Back to the draft' }));
    expect(await sourceBox()).toBeInTheDocument();
  });

  it('shows why a version failed when it is opened', async () => {
    const { server, user } = await openEditor(DEVELOPER, { versions: [aVersionSummary(2, 'FAILED')] });
    server.override(`GET ${server.paths.one}/versions/2`, () => aVersion(2, 'FAILED', { failure: 'unavailable: no runner', buildLog: '' }));
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Versions' })));
    await user.click(await eventually(() => screen.getByRole('button', { name: 'Open version 2 read-only' })));
    expect(await eventually(() => screen.getByText('The build failed.'))).toBeInTheDocument();
    expect(alerts()).toContain('unavailable: no runner');
    expect(alerts()).toContain('No runner could build it');
  });

  it('loads older versions when a page is full', async () => {
    const page = Array.from({ length: 20 }, (_, i) => aVersionSummary(30 - i));
    const { server, user } = await openEditor(DEVELOPER, { versions: page });
    server.override(`GET ${server.paths.one}/versions`, (_init, url) => (url.includes('before=11') ? { items: [aVersionSummary(10)] } : { items: page }));
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Versions' })));
    await user.click(await eventually(() => screen.getByRole('button', { name: 'Load older versions' })));
    expect(await eventually(() => screen.getByRole('rowheader', { name: '10' }))).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Load older versions' })).not.toBeInTheDocument();
  });

  it('says there is nothing published, and shows an error when the list cannot be read', async () => {
    const { server, user } = await openEditor();
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Versions' })));
    expect(await eventually(() => screen.getByText('Nothing published yet.'))).toBeInTheDocument();
    server.override(`GET ${server.paths.one}/versions`, () => problem(500, 'INTERNAL', 'internal error'));
    await user.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await eventually(() => screen.getByText('Reading the versions failed.'))).toBeInTheDocument();
  });
});

describe('Limits', () => {
  it('shows the limits the script is held to, and that the Studio does not change them', async () => {
    const { user } = await openEditor();
    await user.click(await eventually(() => screen.getByRole('tab', { name: 'Limits' })));
    const table = screen.getByRole('table', { name: 'Limits of the script' });
    expect(within(table).getByRole('row', { name: /Memory/ })).toHaveTextContent('256 MiB');
    expect(within(table).getByRole('row', { name: /Time/ })).toHaveTextContent('30 s');
    expect(within(table).getByRole('row', { name: /Network/ })).toHaveTextContent('none: the script has no way out');
    expect(within(table).getByRole('row', { name: /Records kept for/ })).toHaveTextContent('14 days');
    expect(screen.getByText(/The Studio does not change them/)).toBeInTheDocument();
  });
});
