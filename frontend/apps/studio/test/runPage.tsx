import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode } from 'react';
import { Route, Routes } from 'react-router';
import { RunDetailPage } from '../src/features/runs/RunDetailPage';
import { renderApp } from './render';
import { OPERATOR, RUN_ID, runPlatform, signedInApp } from './runs';
import type { Pipe, RunPlatformInit } from './runs';

/** The run page over a scripted platform, signed in with the permissions given. */
export async function openRunPage(init: RunPlatformInit = {}, permissions: string[] = OPERATOR, options: { strict?: boolean } = {}) {
  const platform = runPlatform(init);
  const app = await signedInApp(platform.fetcher, permissions);
  const routes = (
    <Routes>
      <Route path="/runs/:runId" element={<RunDetailPage />} />
      <Route path="/workflows/:workflowId" element={<p>the editor</p>} />
    </Routes>
  );
  // In development React runs every effect twice; `strict` does the same here.
  const view = renderApp(options.strict ? <StrictMode>{routes}</StrictMode> : routes, { app, path: `/runs/${RUN_ID}` });
  await screen.findByTestId('follow-state');
  return { platform, ...view, user: userEvent.setup() };
}

/** Waits until the page has opened the stream, so a test can write to it. */
export async function openStream(platform: ReturnType<typeof runPlatform>): Promise<Pipe> {
  await waitFor(() => expect(platform.pipes.length).toBeGreaterThan(0));
  return platform.pipes[platform.pipes.length - 1] as Pipe;
}
