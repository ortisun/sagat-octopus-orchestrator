import { useEffect, useRef, useState } from 'react';
import { formatAbsolute, formatLocal } from '../runs/time';

export { useNamespaceId } from '../batch/queries';

/** A moment in the reader's zone, with the unambiguous UTC time for whoever hovers or reads the markup. */
export function Time({ value }: { value: string | null | undefined }) {
  if (value === null || value === undefined || value === '') return <span>—</span>;
  return (
    <time dateTime={value} title={formatAbsolute(value)}>
      {formatLocal(value)}
    </time>
  );
}

/**
 * Sends one command at a time. The idempotency key lives as long as the `fingerprint` of the request
 * does not change, so a retry of the same request (after a timeout, say) cannot do it twice and a
 * changed request is a new one. A request still in flight when the form goes away is abandoned.
 */
export function useCommand(fingerprint: string) {
  const key = useRef<{ fingerprint: string; key: string } | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<unknown>();
  useEffect(() => () => pending.current?.abort(), []);

  const send = <T,>(call: (key: string, signal: AbortSignal) => Promise<T>, done: (result: T) => void | Promise<void>) => {
    if (sending) return;
    setError(undefined);
    if (key.current?.fingerprint !== fingerprint) key.current = { fingerprint, key: crypto.randomUUID() };
    const controller = new AbortController();
    pending.current = controller;
    setSending(true);
    void call(key.current.key, controller.signal)
      .then(async (result) => {
        if (controller.signal.aborted) return;
        await done(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setError(cause);
      })
      .finally(() => {
        if (!controller.signal.aborted) setSending(false);
      });
  };
  return { send, sending, error };
}
