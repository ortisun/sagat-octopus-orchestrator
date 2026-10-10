import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Root } from './app/Root';
import { createApp } from './app/context';
import type { StudioSettings } from './app/context';
import './styles/base.css';

/** The settings the deployment publishes; without them the Studio signs in with a pasted token. */
async function settings(): Promise<StudioSettings> {
  try {
    const response = await fetch('/studio/config.json', { credentials: 'omit', cache: 'no-store' });
    if (!response.ok) return { oidc: null };
    const body = (await response.json()) as { oidc?: StudioSettings['oidc'] };
    return { oidc: body.oidc ?? null };
  } catch {
    return { oidc: null };
  }
}

const container = document.getElementById('root');
if (container === null) throw new Error('index.html has no #root');
void settings().then((loaded) => {
  createRoot(container).render(
    <StrictMode>
      <Root app={createApp({ settings: loaded })} />
    </StrictMode>,
  );
});
