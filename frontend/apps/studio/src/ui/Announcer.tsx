import { createContext, useCallback, useContext, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';

const Announce = createContext<(message: string) => void>(() => undefined);

const KEPT = 5;
const LIVES_MS = 120_000;

/**
 * A polite live region: what changed on the page without moving focus ("Saved", "3 problems").
 * Every message is its own element, so the ones that come together are all announced, in order, and
 * the same text twice is announced twice; a message leaves the page after two minutes.
 */
export function AnnouncerProvider({ children }: { children: ReactNode }) {
  const [messages, setMessages] = useState<{ id: number; text: string }[]>([]);
  const next = useRef(0);
  const announce = useCallback((text: string) => {
    const id = next.current++;
    setMessages((current) => [...current.slice(-(KEPT - 1)), { id, text }]);
    setTimeout(() => setMessages((current) => current.filter((m) => m.id !== id)), LIVES_MS);
  }, []);
  const value = useMemo(() => announce, [announce]);
  return (
    <Announce.Provider value={value}>
      {children}
      <div className="visually-hidden" role="status" aria-live="polite" aria-atomic="false" aria-relevant="additions">
        {messages.map((m) => (
          <p key={m.id}>{m.text}</p>
        ))}
      </div>
    </Announce.Provider>
  );
}

export function useAnnounce(): (message: string) => void {
  return useContext(Announce);
}
