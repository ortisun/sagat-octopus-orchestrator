import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderApp } from '../../../test/render';
import { ScriptsPage } from './ScriptsPage';
import { DEVELOPER, NAMESPACE_ID, OTHER_SCRIPT_ID, SCRIPT_ID, VIEWER, aSummary, json, problem, scriptsServer, signedIn, stubStorage } from './testing';

vi.mock('./editorModule', () => ({ loadEditor: () => Promise.reject(new Error('no Monaco in jsdom')) }));

beforeEach(() => {
  stubStorage();
});
afterEach(() => {
  vi.unstubAllGlobals();
});

async function open(permissions: string[] = DEVELOPER, options: Parameters<typeof scriptsServer>[0] = {}) {
  const server = scriptsServer(options);
  const app = await signedIn(server.fetcher, permissions);
  renderApp(
    <Routes>
      <Route path="/scripts" element={<ScriptsPage />} />
      <Route path="/scripts/:scriptId" element={<p>the editor of a script</p>} />
    </Routes>,
    { app, path: '/scripts' },
  );
  return { server, user: userEvent.setup() };
}

describe('ScriptsPage: the list', () => {
  it('lists the scripts of the namespace, asking for them by the namespace id', async () => {
    const { server } = await open(DEVELOPER, {
      scripts: [aSummary({ name: 'double-it', latestVersion: 3 }), aSummary({ id: OTHER_SCRIPT_ID, name: 'send-mail', language: 'python', sourceBytes: 2048 })],
    });
    const table = await screen.findByRole('table', { name: 'Scripts in the namespace, by name' });
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows).toHaveLength(2);
    expect(within(rows[0]!).getByRole('link', { name: 'double-it' })).toHaveAttribute('href', `/scripts/${SCRIPT_ID}`);
    expect(rows[0]).toHaveTextContent('TypeScript');
    expect(rows[0]).toHaveTextContent('Version 3');
    expect(rows[1]).toHaveTextContent('Python');
    expect(rows[1]).toHaveTextContent('not published');
    expect(rows[1]).toHaveTextContent('2 KiB');
    const asked = server.calls.find((c) => c.url.startsWith(`/v1/namespaces/${NAMESPACE_ID}/scripts`));
    expect(asked?.url).toBe(`/v1/namespaces/${NAMESPACE_ID}/scripts?limit=50`);
  });

  it('says there are none, and how to make one', async () => {
    await open(DEVELOPER, { scripts: [] });
    expect(await screen.findByText(/No scripts in this namespace yet/)).toHaveTextContent('Make one with “New script”');
  });

  it('shows a name as text, whatever it holds', async () => {
    await open(DEVELOPER, { scripts: [aSummary({ name: '<img src=x onerror=alert(1)>' })] });
    expect(await screen.findByRole('link', { name: '<img src=x onerror=alert(1)>' })).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
  });

  it('continues after the last name when a page is full', async () => {
    const page = Array.from({ length: 50 }, (_, i) => aSummary({ id: `66666666-6666-4666-8666-${String(i).padStart(12, '0')}`, name: `script-${String(i).padStart(2, '0')}` }));
    const { server, user } = await open(DEVELOPER, { scripts: page });
    await screen.findByRole('link', { name: 'script-49' });
    server.override(`GET ${server.paths.base}`, (_init, url) => (url.includes('after=script-49') ? { items: [aSummary({ id: OTHER_SCRIPT_ID, name: 'script-50' })] } : { items: page }));
    await user.click(screen.getByRole('button', { name: 'Load more scripts' }));
    expect(await screen.findByRole('link', { name: 'script-50' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Load more scripts' })).not.toBeInTheDocument();
  });

  it('shows an error with its trace id when the list cannot be read', async () => {
    const server = scriptsServer();
    server.override(`GET ${server.paths.base}`, () => problem(403, 'FORBIDDEN', 'you may not read scripts here'));
    const app = await signedIn(server.fetcher);
    renderApp(<ScriptsPage />, { app, path: '/scripts' });
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('you may not read scripts here');
    expect(alert).toHaveTextContent('a'.repeat(32));
  });
});

describe('ScriptsPage: permissions', () => {
  it('lets a reader look but not make or delete', async () => {
    await open(VIEWER);
    await screen.findByRole('link', { name: 'double-it' });
    expect(screen.queryByRole('button', { name: 'New script' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Delete/ })).not.toBeInTheDocument();
    expect(screen.getByText(/needs the permission script:write/)).toBeInTheDocument();
  });

  it('does not even ask the platform when the role cannot read scripts', async () => {
    const { server } = await open([]);
    expect(await screen.findByRole('alert')).toHaveTextContent('script:read');
    expect(server.calls.filter((c) => c.url.includes('/scripts'))).toHaveLength(0);
  });
});

describe('ScriptsPage: making a script', () => {
  it('offers the five languages, and disables the two the platform does not build, saying so', async () => {
    const { user } = await open();
    await user.click(await screen.findByRole('button', { name: 'New script' }));
    const dialog = screen.getByRole('dialog', { name: 'New script' });
    const select = within(dialog).getByRole('combobox', { name: 'Language' });
    const options = within(select).getAllByRole('option');
    expect(options.map((o) => o.textContent)).toEqual(['TypeScript', 'Python', 'C#', 'Go (not built by this platform yet)', 'Java (not built by this platform yet)']);
    expect(options.map((o) => (o as HTMLOptionElement).disabled)).toEqual([false, false, false, true, true]);
  });

  it('makes a script from the sample of the language and opens its editor', async () => {
    const { server, user } = await open();
    await user.click(await screen.findByRole('button', { name: 'New script' }));
    const dialog = screen.getByRole('dialog', { name: 'New script' });
    const create = within(dialog).getByRole('button', { name: 'Create script' });
    expect(create).toBeDisabled();
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'Bad Name');
    expect(within(dialog).getByText(/Use lower-case letters, digits and hyphens/)).toBeInTheDocument();
    expect(create).toBeDisabled();
    await user.clear(within(dialog).getByRole('textbox', { name: 'Name' }));
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'send-mail');
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Language' }), 'python');
    await user.click(create);
    expect(await screen.findByText('the editor of a script')).toBeInTheDocument();
    const posted = server.callsTo('POST', server.paths.base)[0];
    const body = JSON.parse(posted?.init.body as string) as { name: string; language: string; source: string };
    expect(body).toMatchObject({ name: 'send-mail', language: 'python' });
    expect(body.source).toMatch(/def run\(input, octopus\)/);
    expect((posted?.init.headers as Record<string, string>)['Idempotency-Key']).toBeTruthy();
  });

  it('creates a C# script with the isolated runner contract as its starter', async () => {
    const { server, user } = await open();
    await user.click(await screen.findByRole('button', { name: 'New script' }));
    const dialog = screen.getByRole('dialog', { name: 'New script' });
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'csharp-double');
    await user.selectOptions(within(dialog).getByRole('combobox', { name: 'Language' }), 'csharp');
    await user.click(within(dialog).getByRole('button', { name: 'Create script' }));
    expect(await screen.findByText('the editor of a script')).toBeInTheDocument();
    const posted = server.callsTo('POST', server.paths.base)[0];
    const body = JSON.parse(posted?.init.body as string) as { language: string; source: string };
    expect(body.language).toBe('csharp');
    expect(body.source).toContain('IInlineScript');
    expect(body.source).toContain('ValueTask<JsonElement> RunAsync');
    expect(body.source).toContain('IOctopus octopus');
  });

  it('puts the problem with a taken name on the name', async () => {
    const { user } = await open();
    await user.click(await screen.findByRole('button', { name: 'New script' }));
    const dialog = screen.getByRole('dialog', { name: 'New script' });
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'double-it');
    await user.click(within(dialog).getByRole('button', { name: 'Create script' }));
    expect(await within(dialog).findByText('A script with this name already exists in this namespace.')).toBeInTheDocument();
    expect(within(dialog).getByRole('textbox', { name: 'Name' })).toBeInvalid();
    // The dialog stays open and can be tried again.
    expect(within(dialog).getByRole('button', { name: 'Create script' })).toBeEnabled();
  });

  it('puts the refusal of a language that is not built on the language, if the platform says so', async () => {
    const { server, user } = await open();
    server.override(`POST ${server.paths.base}`, () => problem(422, 'VALIDATION_FAILED', 'the language is not built yet: typescript and python are', { reason: 'LANGUAGE_NOT_BUILT' }));
    await user.click(await screen.findByRole('button', { name: 'New script' }));
    const dialog = screen.getByRole('dialog', { name: 'New script' });
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'later');
    await user.click(within(dialog).getByRole('button', { name: 'Create script' }));
    expect(await within(dialog).findByText(/This language is not built by this platform yet/)).toBeInTheDocument();
  });

  it('shows any other refusal with its trace id and keeps the dialog', async () => {
    const { server, user } = await open();
    server.override(`POST ${server.paths.base}`, () => problem(429, 'QUOTA_EXCEEDED', 'a limit was reached; release something and retry', { limit: 'scripts' }));
    await user.click(await screen.findByRole('button', { name: 'New script' }));
    const dialog = screen.getByRole('dialog', { name: 'New script' });
    await user.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'one-more');
    await user.click(within(dialog).getByRole('button', { name: 'Create script' }));
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('a limit was reached');
    expect(within(dialog).getByRole('alert')).toHaveTextContent('a'.repeat(32));
  });
});

describe('ScriptsPage: deleting', () => {
  const published = aSummary({ id: OTHER_SCRIPT_ID, name: 'already-used', latestVersion: 2 });

  it('allows it only for a script that has no version, and says why not for the others', async () => {
    await open(DEVELOPER, { scripts: [aSummary(), published] });
    expect(await screen.findByRole('button', { name: 'Delete double-it' })).toBeEnabled();
    const blocked = screen.getByRole('button', { name: /Delete already-used: it has versions/ });
    expect(blocked).toBeDisabled();
    expect(blocked).toHaveAttribute('title', expect.stringMatching(/a workflow may pin its artifact/));
  });

  it('asks first, deletes, and the script is gone from the list', async () => {
    const { server, user } = await open();
    await user.click(await screen.findByRole('button', { name: 'Delete double-it' }));
    const dialog = screen.getByRole('dialog', { name: 'Delete script' });
    expect(dialog).toHaveTextContent('This cannot be undone');
    expect(server.callsTo('DELETE', server.paths.one)).toHaveLength(0);
    await user.click(within(dialog).getByRole('button', { name: 'Delete script' }));
    await waitFor(() => expect(screen.queryByRole('link', { name: 'double-it' })).not.toBeInTheDocument());
    expect(server.callsTo('DELETE', server.paths.one)).toHaveLength(1);
    expect(await screen.findByText(/No scripts in this namespace yet/)).toBeInTheDocument();
  });

  it('can be given up on', async () => {
    const { server, user } = await open();
    await user.click(await screen.findByRole('button', { name: 'Delete double-it' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Delete script' })).getByRole('button', { name: 'Keep it' }));
    expect(server.callsTo('DELETE', server.paths.one)).toHaveLength(0);
    expect(screen.getByRole('link', { name: 'double-it' })).toBeInTheDocument();
  });

  it('says so when the platform keeps the script after all (someone published it meanwhile)', async () => {
    const { server, user } = await open();
    server.override(`DELETE ${server.paths.one}`, () => problem(409, 'CONFLICT', 'a script with a published version cannot be deleted', { reason: 'STATE_CONFLICT' }));
    await user.click(await screen.findByRole('button', { name: 'Delete double-it' }));
    const dialog = screen.getByRole('dialog', { name: 'Delete script' });
    await user.click(within(dialog).getByRole('button', { name: 'Delete script' }));
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('cannot be deleted');
    expect(screen.getByRole('link', { name: 'double-it' })).toBeInTheDocument();
  });
});

describe('ScriptsPage: the namespace', () => {
  it('waits for the namespaces, and says when there are none', async () => {
    const server = scriptsServer();
    server.override('GET /namespaces', () => ({ items: [] }));
    const app = await signedIn(server.fetcher);
    renderApp(<ScriptsPage />, { app, path: '/scripts' });
    expect(await screen.findByText('There is no namespace to hold scripts yet.')).toBeInTheDocument();
    expect(server.calls.filter((c) => c.url.includes('/scripts'))).toHaveLength(0);
  });

  it('shows an error when the namespaces cannot be read', async () => {
    const server = scriptsServer();
    server.override('GET /namespaces', () => json(503, { code: 'UNAVAILABLE', message: 'the platform is not available right now' }));
    const app = await signedIn(server.fetcher);
    renderApp(<ScriptsPage />, { app, path: '/scripts' });
    expect(await screen.findByRole('alert')).toHaveTextContent('the platform is not available right now');
  });
});
