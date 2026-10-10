import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Route, Routes } from 'react-router';
import { vi } from 'vitest';
import { renderApp } from '../../../test/render';
import { ScriptEditorPage } from './ScriptEditorPage';
import { DEVELOPER, SCRIPT_ID, scriptsServer, signedIn } from './testing';

/** The editor page of a script on a scripted platform, as `permissions` would see it. */
export async function openEditor(permissions: string[] = DEVELOPER, options: Parameters<typeof scriptsServer>[0] = {}, path: string = `/scripts/${SCRIPT_ID}`) {
  const server = scriptsServer(options);
  const app = await signedIn(server.fetcher, permissions);
  const view = renderApp(
    <Routes>
      <Route path="/scripts" element={<p>the list of scripts</p>} />
      <Route path="/scripts/:scriptId" element={<ScriptEditorPage />} />
    </Routes>,
    { app, path },
  );
  // On a clock that only the test moves, user-event must not wait on it.
  return { server, app, view, user: userEvent.setup(vi.isFakeTimers() ? { delay: null } : {}) };
}

/** The text of every alert on the page (the editor's own fallback notice is one of them in these tests). */
export const alerts = () => screen.getAllByRole('alert').map((alert) => alert.textContent ?? '').join(' | ');

export const sourceBox = () => screen.findByRole('textbox', { name: 'Source of double-it (TypeScript)' });

export const patchesOf = (server: ReturnType<typeof scriptsServer>) => server.callsTo('PATCH', server.paths.one).map((c) => JSON.parse(c.init.body as string) as Record<string, unknown>);
