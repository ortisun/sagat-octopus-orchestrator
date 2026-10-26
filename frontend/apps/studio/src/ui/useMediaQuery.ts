import { useSyncExternalStore } from 'react';

/** Whether a media query matches, kept up to date. Where the browser cannot tell (a test), it does not match. */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (listener) => {
      const list = globalThis.matchMedia?.(query);
      list?.addEventListener('change', listener);
      return () => list?.removeEventListener('change', listener);
    },
    () => globalThis.matchMedia?.(query).matches ?? false,
    () => false,
  );
}
