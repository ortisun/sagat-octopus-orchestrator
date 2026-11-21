import '@astryxdesign/core/reset.css';
import '@astryxdesign/core/astryx.css';
import { Theme } from '@astryxdesign/core/theme';
import { useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import { sagatTheme } from './theme/sagatTheme';

type Mode = 'light' | 'dark' | 'system';

/** `data-theme` on the document is the explicit override of the system's colour scheme (Decision 157). */
function readMode(): Mode {
  const value = document.documentElement.dataset['theme'];
  return value === 'light' || value === 'dark' ? value : 'system';
}

function watchMode(changed: () => void): () => void {
  const observer = new MutationObserver(changed);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  return () => observer.disconnect();
}

/**
 * What every screen of the Studio is drawn inside: the Astryx stylesheets in their layer order (reset,
 * then components) and the `sagat` theme, which the design system writes into the page as a style
 * element when the application starts (Architecture decision 0101). The Studio is not rendered on a server, so there is
 * no first paint without the theme.
 */
export function UiProvider({ children }: { children: ReactNode }) {
  const mode = useSyncExternalStore(watchMode, readMode, () => 'system' as const);
  return (
    <Theme theme={sagatTheme} mode={mode}>
      {children}
    </Theme>
  );
}
