import { useEffect, useRef, useState } from 'react';
import type { Endpoints } from '../../api/endpoints';
import { nodeIdsOf, viewOf } from './validationModel';
import type { ValidationView } from './validationModel';

export interface ValidationState {
  /** `checking`: the document has changed since the answer shown, and a newer answer is on its way. */
  phase: 'checking' | 'done' | 'failed';
  /** The last answer the platform gave, even if the document has moved on since. */
  view: ValidationView | undefined;
  /** The answer is about the document in the editor right now. */
  current: boolean;
  error: unknown;
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
export function useValidation(ep: Pick<Endpoints, 'validate'>, workflowId: string, wire: Record<string, unknown>, key: string, delayMs: number): ValidationState {
  const [answer, setAnswer] = useState<Answer>({ view: undefined, forKey: undefined, failedKey: undefined, error: undefined });
  const latest = useRef({ wire, ep });
  const asked = useRef(false);
  const answeredKey = useRef<string | undefined>(undefined);

  useEffect(() => {
    latest.current = { wire, ep };
  });

  useEffect(() => {
    // The document is back to the one the last answer is about (a change that was undone): that answer stands.
    if (answeredKey.current === key) return;
    const controller = new AbortController();
    const timer = setTimeout(
      () => {
        asked.current = true;
        const { wire: document, ep: api } = latest.current;
        const ids = nodeIdsOf(document);
        api.validate(workflowId, document, controller.signal).then(
          (result) => {
            if (controller.signal.aborted) return;
            answeredKey.current = key;
            setAnswer({ view: viewOf(result, ids), forKey: key, failedKey: undefined, error: undefined });
          },
          (error: unknown) => {
            if (controller.signal.aborted) return;
            answeredKey.current = undefined;
            setAnswer((a) => ({ view: a.view, forKey: undefined, failedKey: key, error }));
          },
        );
      },
      asked.current ? delayMs : 0,
    );
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [key, workflowId, delayMs]);

  if (answer.failedKey === key) return { phase: 'failed', view: answer.view, current: false, error: answer.error };
  if (answer.forKey === key) return { phase: 'done', view: answer.view, current: true, error: undefined };
  return { phase: 'checking', view: answer.view, current: false, error: undefined };
}
