import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Root } from './app/Root';
import { createApp } from './app/context';
import { loadSettings } from './app/settings';
import '@fontsource/geist/latin-400.css';
import '@fontsource/geist/latin-500.css';
import '@fontsource/geist/latin-600.css';
import '@fontsource/geist-mono/latin-400.css';
import '@fontsource/geist-mono/latin-500.css';
import './styles/base.css';

const container = document.getElementById('root');
if (container === null) throw new Error('index.html has no #root');
void loadSettings().then((loaded) => {
  createRoot(container).render(
    <StrictMode>
      <Root app={createApp({ settings: loaded })} />
    </StrictMode>,
  );
});
