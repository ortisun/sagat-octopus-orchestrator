import { useCallback, useEffect, useRef, useState } from 'react';
import type { Endpoints } from '../../api/endpoints';
import { nodeIdsOf, viewOf } from './validationModel';
import type { ValidationView } from './validationModel';

export interface ValidationState {
  /**
   * `checking`: the document has changed since the answer shown, and a newer answer is on its way.
   * `off`: nobody asked, because the platform answers this only to people who can change the workflow.
   */
  phase: 'checking' | 'done' | 'failed' | 'off';
  /** The last answer the platform gave, even if the document has moved on since. */
  view: ValidationView | undefined;
  /** The answer is about the document in the editor right now. */
  current: boolean;
  error: unknown;
  /** Asks again about the same document: the platform's answer also depends on what exists (a script built, a secret created). */
  recheck: () => void;
}

interface Answer {
  view: ValidationView | undefined;
  /** The document (canonical form) the answer or the failure is about. */
  forKey: string | undefined;
  failedKey: string | undefined;
  error: unknown;
}

/**
 * Asks the platform whether the document is valid: immediately on load, then after `delayMs` of
 * quiet following every change. A newer document cancels the request of an older one, and an answer
 * that arrives for a document that is no longer the current one is ignored, so a slow old answer can
 * never replace a newer one.
 */
/** An answer older than this is asked for again when the person comes back to the window. */
const STALE_MS = 30_000;

export function useValidation(ep: Pick<Endpoints, 'validate'>, workflowId: string, wire: Record<string, unknown>, key: string, delayMs: number, enabled = true): ValidationState {
  const [answer, setAnswer] = useState<Answer>({ view: undefined, forKey: undefined, failedKey: undefined, error: undefined });
  const latest = useRef({ wire, ep });
  const asked = useRef(false);
  const answeredKey = useRef<string | undefined>(undefined);
  const answeredAt = useRef(0);
  const immediate = useRef(false);
  const [again, setAgain] = useState(0);
  const recheck = useCallback(() => {
    answeredKey.current = undefined;
    immediate.current = true;
    setAnswer((a) => ({ ...a, forKey: undefined, failedKey: undefined }));
    setAgain((n) => n + 1);
  }, []);

  // Coming back to the window after a while: what the platform knows may have changed (the script was built in
  // another tab), and the document did not, so nothing else would ask.
  useEffect(() => {
    const back = () => {
      if (enabled && document.visibilityState !== 'hidden' && Date.now() - answeredAt.current > STALE_MS && answeredAt.current !== 0) recheck();
    };
    window.addEventListener('focus', back);
    document.addEventListener('visibilitychange', back);
    return () => {
      window.removeEventListener('focus', back);
      document.removeEventListener('visibilitychange', back);
    };
  }, [enabled, recheck]);

  useEffect(() => {
    latest.current = { wire, ep };
  });

  useEffect(() => {
    // Not for someone the platform would refuse (it would answer 403 to every change they cannot make anyway).
    if (!enabled) return;
    // The document is back to the one the last answer is about (a change that was undone): that answer stands.
    if (answeredKey.current === key) return;
    const controller = new AbortController();
    const timer = setTimeout(
      () => {
        asked.current = true;
        immediate.current = false;
        const { wire: document, ep: api } = latest.current;
        const ids = nodeIdsOf(document);
        api.validate(workflowId, document, controller.signal).then(
          (result) => {
            if (controller.signal.aborted) return;
            answeredKey.current = key;
            answeredAt.current = Date.now();
            setAnswer({ view: viewOf(result, ids), forKey: key, failedKey: undefined, error: undefined });
          },
          (error: unknown) => {
            if (controller.signal.aborted) return;
            answeredKey.current = undefined;
            setAnswer((a) => ({ view: a.view, forKey: undefined, failedKey: key, error }));
          },
        );
      },
      asked.current && !immediate.current ? delayMs : 0,
    );
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [key, workflowId, delayMs, enabled, again]);

  if (!enabled) return { phase: 'off', view: undefined, current: false, error: undefined, recheck };
  if (answer.failedKey === key) return { phase: 'failed', view: answer.view, current: false, error: answer.error, recheck };
  if (answer.forKey === key) return { phase: 'done', view: answer.view, current: true, error: undefined, recheck };
  return { phase: 'checking', view: answer.view, current: false, error: undefined, recheck };
}
