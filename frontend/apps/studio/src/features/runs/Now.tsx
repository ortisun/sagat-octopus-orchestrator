import { createContext, useContext, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { formatAbsolute, formatDuration, formatLocal, formatRelative, parseTime } from './time';

/** The clock the page reads, ticking on its own schedule so a screenful of times does not redraw every second. */
const NowContext = createContext<number>(0);

/**
 * Provides the time to everything below, and moves it on every `intervalMs` while the tab is
 * visible (a hidden tab does not tick). With `enabled` false the time is fixed at the moment it
 * was first read: a finished run has nothing to count.
 */
export function NowProvider({ intervalMs, enabled = true, children }: { intervalMs: number; enabled?: boolean; children: ReactNode }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!enabled) return;
    const tick = () => {
      if (document.visibilityState !== 'hidden') setNow(Date.now());
    };
    const timer = setInterval(tick, intervalMs);
    document.addEventListener('visibilitychange', tick);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', tick);
    };
  }, [intervalMs, enabled]);
  return <NowContext value={now}>{children}</NowContext>;
}

export function useNow(): number {
  return useContext(NowContext);
}

/** A moment, relative to now in the text and absolute (UTC) in the tooltip. Nothing if it is not a date. */
export function Relative({ at }: { at: string | null | undefined }) {
  const now = useNow();
  if (parseTime(at) === undefined) return <span>unknown time</span>;
  return (
    <time dateTime={at ?? undefined} title={formatAbsolute(at)}>
      {formatRelative(at, now)}
    </time>
  );
}

/** A moment in the reader's own zone, with the UTC time in the tooltip. */
export function Moment({ at }: { at: string | null | undefined }) {
  if (parseTime(at) === undefined) return <span>unknown time</span>;
  return (
    <time dateTime={at ?? undefined} title={formatAbsolute(at)}>
      {formatLocal(at)}
    </time>
  );
}

/** How long something has lasted: fixed when it has an end, ticking with the page's clock when it has not. */
export function Elapsed({ from, to }: { from: string | null | undefined; to?: string | null | undefined }) {
  const now = useNow();
  const start = parseTime(from);
  if (start === undefined) return <span>unknown</span>;
  const end = to === null || to === undefined ? now : parseTime(to);
  if (end === undefined) return <span>unknown</span>;
  return <span>{formatDuration(Math.max(0, end - start))}</span>;
}
