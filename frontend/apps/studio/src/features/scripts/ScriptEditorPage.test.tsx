import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderApp } from '../../../test/render';
import { alerts, openEditor, patchesOf, sourceBox } from './testingEditor';
import { scriptsConfig } from './config';
import { ScriptEditorPage } from './ScriptEditorPage';
import { DEVELOPER, NAMESPACE_ID, OTHER_SCRIPT_ID, SCRIPT_ID, VIEWER, aScript, problem, scriptsServer, signedIn, stubStorage } from './testing';

vi.mock('./editorModule', () => ({ loadEditor: () => Promise.reject(new Error('no Monaco in jsdom')) }));

beforeEach(() => {
  stubStorage();
  scriptsConfig.autosaveMs = 30;
});
afterEach(() => {
  scriptsConfig.autosaveMs = 1500;
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('ScriptEditorPage: opening a script', () => {
  it('reads the script by its id and shows its draft in the editor, with what is known about it', async () => {
    const { server } = await openEditor();
    expect(await screen.findByRole('heading', { level: 1, name: 'double-it' })).toBeInTheDocument();
    expect(screen.getByText(/TypeScript · draft revision 1 · not published yet/)).toBeInTheDocument();
    const box = await sourceBox();
    expect(box).toHaveValue(aScript().source);
    expect(box).not.toHaveAttribute('readonly');
    expect(screen.getByText(/of 256 KiB available/)).toBeInTheDocument();
    expect(screen.getByText(/advice only/)).toBeInTheDocument();
    expect(screen.getByTestId('save-status')).toHaveTextContent('All changes saved.');
    expect(server.callsTo('GET', server.paths.one)).toHaveLength(1);
    expect(screen.getByRole('tab', { name: 'Try it in the sandbox', selected: true })).toBeInTheDocument();
  });

  it('says it for Python: highlighting only, nothing is checked in the browser', async () => {
    await openEditor(DEVELOPER, { script: aScript({ language: 'python', source: 'def run(input, octopus):\n    return 1\n' }) });
    expect(await screen.findByRole('textbox', { name: 'Source of double-it (Python)' })).toBeInTheDocument();
    expect(screen.getByText('Syntax highlighting only; the platform checks the source when it builds.')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: /Problems/ })).not.toBeInTheDocument();
  });

  it('does not go to the platform with an id that is not an id', async () => {
    const { server } = await openEditor(DEVELOPER, {}, '/scripts/..%2F..%2Fsecrets');
    expect(await screen.findByRole('heading', { name: 'Script not found' })).toBeInTheDocument();
    expect(server.calls.filter((c) => c.url.includes('/scripts/'))).toHaveLength(0);
  });

  it('says so when there is no such script', async () => {
    const { server } = await openEditor();
    server.override(`GET ${server.paths.one}`, () => problem(404, 'NOT_FOUND', 'it does not exist, or it is not yours to see'));
    // A second visit reads it again.
    const second = await openEditor(DEVELOPER, {}, `/scripts/${OTHER_SCRIPT_ID}`);
    expect(await screen.findAllByRole('heading', { name: 'Script not found' })).not.toHaveLength(0);
    expect(second.server.callsTo('GET', `/namespaces/${NAMESPACE_ID}/scripts/${OTHER_SCRIPT_ID}`)).toHaveLength(1);
  });

  it('shows an error with its trace id when the script cannot be read', async () => {
    const server = scriptsServer();
    server.override(`GET ${server.paths.one}`, () => problem(503, 'UNAVAILABLE', 'the platform is not available right now'));
    const app = await signedIn(server.fetcher);
    renderApp(
      <Routes>
        <Route path="/scripts/:scriptId" element={<ScriptEditorPage />} />
      </Routes>,
      { app, path: `/scripts/${SCRIPT_ID}` },
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('the platform is not available right now');
    expect(screen.getByRole('alert')).toHaveTextContent('a'.repeat(32));
  });
});

describe('ScriptEditorPage: a reader', () => {
  it('can read the source but not change it, try it or publish it', async () => {
    const { server, user } = await openEditor(VIEWER);
    const box = await sourceBox();
    expect(box).toHaveAttribute('readonly');
    expect(screen.getByText('Read only.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Save now' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Publish version…' })).not.toBeInTheDocument();
    expect(screen.getByText(/Publishing needs the permission script:publish/)).toBeInTheDocument();
    expect(screen.getByText(/needs the permission script:write/)).toBeInTheDocument();
    await user.type(box, 'tampering');
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(0);
    // Test runs need script:write: they are not even asked for.
    expect(server.calls.filter((c) => c.url.includes('/test-runs'))).toHaveLength(0);
  });

  it('can read the schemas, read-only too', async () => {
    const { user } = await openEditor(VIEWER, { script: aScript({ inputSchema: { type: 'object' } as never }) });
    await user.click(await screen.findByRole('tab', { name: 'Input and output schemas' }));
    expect(screen.getByRole('textbox', { name: 'Input schema (JSON)' })).toHaveAttribute('readonly');
    expect(screen.getByRole('textbox', { name: 'Input schema (JSON)' })).toHaveValue('{\n  "type": "object"\n}\n');
  });

  it('does not read anything when the role cannot read scripts', async () => {
    const { server } = await openEditor([]);
    expect(await screen.findByRole('alert')).toHaveTextContent('script:read');
    expect(server.calls.filter((c) => c.url.includes('/scripts/'))).toHaveLength(0);
  });
});

describe('ScriptEditorPage: autosave', () => {
  it('saves the source a moment after the typing stops, on the revision it read, and shows the new revision', async () => {
    const { server, user } = await openEditor();
    const box = await sourceBox();
    await user.type(box, '// hello');
    await waitFor(() => expect(patchesOf(server)).toHaveLength(1));
    expect(patchesOf(server)[0]).toEqual({ revision: 1, source: `${aScript().source}// hello` });
    await waitFor(() => expect(screen.getByText(/draft revision 2/)).toBeInTheDocument());
    expect(screen.getByTestId('save-status')).toHaveTextContent('All changes saved.');
    // The next edit is based on the revision the first one made.
    await user.type(box, '!');
    await waitFor(() => expect(patchesOf(server)).toHaveLength(2));
    expect(patchesOf(server)[1]).toMatchObject({ revision: 2 });
  });

  it('shows how much of the limit is used as the text grows', async () => {
    const { user } = await openEditor(DEVELOPER, { script: aScript({ source: 'ab' }) });
    const box = await sourceBox();
    expect(screen.getByText('2 B of 256 KiB available.')).toBeInTheDocument();
    await user.type(box, 'cd');
    expect(screen.getByText('4 B of 256 KiB available.')).toBeInTheDocument();
  });

  it('does not send a source that is over the limit, and says by how much', async () => {
    const { server } = await openEditor();
    const box = await sourceBox();
    // userEvent is slow with this much text; the value is set at once, as a paste would.
    const user = userEvent.setup();
    await user.click(box);
    await user.paste('x'.repeat(256 * 1024 + 10));
    expect(await screen.findByText(/over the limit; the platform would refuse it, so it is not saved/)).toBeInTheDocument();
    expect(screen.getByText(/Not saved:/)).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(0);
    expect(screen.getByTestId('save-status')).toHaveTextContent('Changes not saved: see below.');
  });

  it('saves now when asked, and says so', async () => {
    scriptsConfig.autosaveMs = 60_000;
    const { server, user } = await openEditor();
    const box = await sourceBox();
    await user.type(box, 'x');
    const save = screen.getByRole('button', { name: 'Save now' });
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() => expect(patchesOf(server)).toHaveLength(1));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save now' })).toBeDisabled());
    expect(await screen.findByText('Draft saved.')).toBeInTheDocument();
  });

  it('says that a save was refused, with the reason and the trace id, and does not call it saved', async () => {
    const { server, user } = await openEditor();
    server.override(`PATCH ${server.paths.one}`, () => problem(422, 'VALIDATION_FAILED', 'the source contains a NUL character', { reason: 'SOURCE' }));
    await user.type(await sourceBox(), 'x');
    expect(await screen.findByText('Not saved: the platform refused the draft.')).toBeInTheDocument();
    expect(alerts()).toContain('the source contains a NUL character');
    expect(alerts()).toContain('a'.repeat(32));
  });

  it('keeps asking the platform about a failure it cannot fix only once (forbidden)', async () => {
    const { server, user } = await openEditor();
    server.override(`PATCH ${server.paths.one}`, () => problem(403, 'FORBIDDEN', 'you may not change this script'));
    await user.type(await sourceBox(), 'x');
    expect(await screen.findByText('Not saved: the save failed.')).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
  });

  it('asks before the page is closed while there are changes the platform does not have', async () => {
    scriptsConfig.autosaveMs = 60_000;
    const { user } = await openEditor();
    const clean = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(clean);
    expect(clean.defaultPrevented).toBe(false);
    await user.type(await sourceBox(), 'x');
    const dirty = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(dirty);
    expect(dirty.defaultPrevented).toBe(true);
  });

  it('sends what is waiting when the person leaves the page', async () => {
    scriptsConfig.autosaveMs = 60_000;
    const { server, user, view } = await openEditor();
    await user.type(await sourceBox(), 'last words');
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(0);
    view.unmount();
    await waitFor(() => expect(patchesOf(server)).toHaveLength(1));
    expect(patchesOf(server)[0]).toMatchObject({ source: `${aScript().source}last words` });
  });
});

describe('ScriptEditorPage: the schemas', () => {
  it('saves a schema that is a JSON object and removes one that is emptied', async () => {
    const { server, user } = await openEditor(DEVELOPER, { script: aScript({ outputSchema: { type: 'string' } as never }) });
    await user.click(await screen.findByRole('tab', { name: 'Input and output schemas' }));
    const input = screen.getByRole('textbox', { name: 'Input schema (JSON)' });
    await user.click(input);
    await user.paste('{"type":"object"}');
    await user.clear(screen.getByRole('textbox', { name: 'Output schema (JSON)' }));
    await waitFor(() => expect(patchesOf(server)).toHaveLength(1));
    expect(patchesOf(server)[0]).toEqual({ revision: 1, inputSchema: { type: 'object' }, outputSchema: null });
  });

  it('underlines nothing it can not, but says when a schema is not JSON, and does not send it', async () => {
    const { server, user } = await openEditor();
    await user.click(await screen.findByRole('tab', { name: 'Input and output schemas' }));
    await user.click(screen.getByRole('textbox', { name: 'Input schema (JSON)' }));
    await user.paste('{"type": ');
    expect(await screen.findByText(/^Not valid JSON/)).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Input schema (JSON)' })).toHaveAccessibleDescription(/Not valid JSON/);
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(0);
    // The text is kept when the tab is left and come back to.
    await user.click(screen.getByRole('tab', { name: 'Versions' }));
    await user.click(screen.getByRole('tab', { name: 'Input and output schemas' }));
    expect(screen.getByRole('textbox', { name: 'Input schema (JSON)' })).toHaveValue('{"type": ');
  });

  it('shows the platform\'s refusal of a schema where the schemas are', async () => {
    const { server, user } = await openEditor();
    server.override(`PATCH ${server.paths.one}`, () => problem(422, 'VALIDATION_FAILED', 'a schema is a JSON Schema object of at most 64 KiB', { reason: 'SCHEMA' }));
    await user.click(await screen.findByRole('tab', { name: 'Input and output schemas' }));
    await user.click(screen.getByRole('textbox', { name: 'Output schema (JSON)' }));
    await user.paste('{"type":"nonsense"}');
    expect(await screen.findByText('The platform did not accept a schema.')).toBeInTheDocument();
  });
});

describe('ScriptEditorPage: two people editing', () => {
  async function conflicted() {
    const ctx = await openEditor();
    const box = await sourceBox();
    // Somebody else saved first: the platform is at revision 4 with a different source.
    ctx.server.state.script = aScript({ source: 'their line 1\nshared\n', revision: 4, updatedAt: '2026-10-10T11:00:00Z' });
    await ctx.user.click(box);
    await ctx.user.clear(box);
    await ctx.user.paste('my line 1\nshared\n');
    return ctx;
  }

  it('keeps the author\'s text, shows both, and overwrites nothing until they choose', async () => {
    const { server } = await conflicted();
    expect(await screen.findByRole('heading', { name: 'Someone else changed this draft' })).toBeInTheDocument();
    expect(alerts()).toMatch(/not saved/);
    expect(alerts()).toMatch(/revision 4/);
    expect(screen.getByTestId('save-status')).toHaveTextContent('Not saved: someone else changed the draft first. Your changes are kept.');
    // The diff is on screen: their line against mine.
    const diff = await screen.findByRole('region', { name: 'Differences in the source' });
    expect(diff).toHaveTextContent('Theirs: - their line 1');
    expect(diff).toHaveTextContent('Yours: + my line 1');
    expect(screen.getByRole('textbox', { name: /Source of double-it/ })).toHaveValue('my line 1\nshared\n');
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(1);
    expect(server.state.script.source).toBe('their line 1\nshared\n');
  });

  it('can load theirs, which drops the author\'s changes on purpose', async () => {
    const { user, server } = await conflicted();
    await user.click(await screen.findByRole('button', { name: 'Load theirs (drop my changes)' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: /Source of double-it/ })).toHaveValue('their line 1\nshared\n'));
    expect(screen.queryByRole('heading', { name: 'Someone else changed this draft' })).not.toBeInTheDocument();
    expect(screen.getByText(/draft revision 4/)).toBeInTheDocument();
    expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(1);
  });

  it('can keep mine and save over theirs, based on their revision', async () => {
    const { user, server } = await conflicted();
    await user.click(await screen.findByRole('button', { name: 'Keep mine and save over theirs' }));
    await waitFor(() => expect(server.callsTo('PATCH', server.paths.one)).toHaveLength(2));
    expect(patchesOf(server)[1]).toEqual({ revision: 4, source: 'my line 1\nshared\n' });
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Someone else changed this draft' })).not.toBeInTheDocument());
    expect(server.state.script.source).toBe('my line 1\nshared\n');
    expect(screen.getByText(/draft revision 5/)).toBeInTheDocument();
  });

  it('can save the author\'s version as a script of its own, and leaves the conflict to decide', async () => {
    const { user, server } = await conflicted();
    await user.click(await screen.findByRole('button', { name: 'Save mine as a new script…' }));
    const dialog = screen.getByRole('dialog', { name: 'Save my version as a new script' });
    expect(within(dialog).getByRole('textbox', { name: 'Name' })).toHaveValue('double-it-copy');
    await user.click(within(dialog).getByRole('button', { name: 'Save as a new script' }));
    expect(await screen.findByText(/Saved as/)).toHaveTextContent('This draft still has the conflict');
    const created = server.callsTo('POST', server.paths.base)[0];
    expect(JSON.parse(created?.init.body as string)).toMatchObject({ name: 'double-it-copy', language: 'typescript', source: 'my line 1\nshared\n' });
    expect(screen.getByRole('heading', { name: 'Someone else changed this draft' })).toBeInTheDocument();
  });

  it('does not run a test, or publish, while the two copies are undecided', async () => {
    const { user, server } = await conflicted();
    await screen.findByRole('heading', { name: 'Someone else changed this draft' });
    expect(screen.getByRole('button', { name: 'Publish version…' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Run in the sandbox' }));
    expect(await screen.findByText('Not run.')).toBeInTheDocument();
    expect(server.calls.filter((c) => c.method === 'POST' && c.url.includes('/test-runs'))).toHaveLength(0);
  });
});
