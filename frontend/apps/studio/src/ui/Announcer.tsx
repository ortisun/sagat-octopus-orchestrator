import { createContext, useCallback, useContext, useMemo, useState } from 'react';
import type { ReactNode } from 'react';

const Announce = createContext<(message: string) => void>(() => undefined);

/** A polite live region: what changed on the page without moving focus ("Saved", "3 problems"). */
export function AnnouncerProvider({ children }: { children: ReactNode }) {
  const [message, setMessage] = useState('');
  const announce = useCallback((next: string) => {
    // The same text twice in a row is announced again only if the region changes in between.
    setMessage('');
    setTimeout(() => setMessage(next), 50);
  }, []);
  const value = useMemo(() => announce, [announce]);
  return (
    <Announce.Provider value={value}>
      {children}
      <div className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
        {message}
      </div>
    </Announce.Provider>
  );
}

export function useAnnounce(): (message: string) => void {
  return useContext(Announce);
}
