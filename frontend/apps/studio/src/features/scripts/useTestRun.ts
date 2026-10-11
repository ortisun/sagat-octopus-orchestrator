import { useCallback, useEffect, useRef, useState } from 'react';
import { useAnnounce } from '../../ui/Announcer';
import type { ScriptTestRun, ScriptsApi } from './api';
import { startPolling } from './polling';
import type { Poller } from './polling';
import { failureHelp, isTestRunFinal } from './status';

export type TestPhase =
  | 'idle'
  /** Saving the draft, then asking the platform for the run. */
  | 'starting'
  /** Reading a run of the history; nothing is shown until it is read, so it is not taken for another. */
  | 'loading'
  /** The run exists and is being looked at once a second or so. */
  | 'watching'
  | 'finished'
  /** The person stopped looking; the run itself goes on. */
  | 'stopped'
  /** The time given for the run passed without a final answer. */
  | 'expired'
  /** Not run: the draft could not be saved first. */
  | 'blocked'
  /** Asking, or looking, failed. */
  | 'failed';

export interface TestState {
  phase: TestPhase;
  run: ScriptTestRun | undefined;
  error: unknown;
  /** How long the Studio watched before giving up, for the message. */
  waitedMs: number | undefined;
}

const IDLE: TestState = { phase: 'idle', run: undefined, error: undefined, waitedMs: undefined };

export interface TestRunOptions {
  api: ScriptsApi;
  scriptId: string;
  /** Saves the draft and says whether the platform now has all of it. A test run uses the platform's copy. */
  flush: () => Promise<boolean>;
  /** The wall-clock limit of the script, which bounds how long a run can last. */
  timeoutMs: number;
  onFinished?: () => void;
}

/** A test run can wait for a runner (two minutes at most), build and then run for up to the script's limit. */
export function watchLimitMs(timeoutMs: number): number {
  return Math.min(25 * 60_000, timeoutMs + 5 * 60_000);
}

/**
 * One test run at a time: save the draft, ask for the run, then look at it until the platform says
 * it is final, the time given is up, the person stops looking, or the page is left. The state lives
 * above the tabs, so changing tab does not stop a run that is being watched.
 */
export function useTestRun({ api, scriptId, flush, timeoutMs, onFinished }: TestRunOptions) {
  const announce = useAnnounce();
  const [state, setState] = useState<TestState>(IDLE);
  const poller = useRef<Poller | undefined>(undefined);
  const alive = useRef(true);
  const lastStatus = useRef<string | undefined>(undefined);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      poller.current?.stop();
    };
  }, []);

  const watch = useCallback(
    (first: ScriptTestRun) => {
      poller.current?.stop();
      const startedAt = Date.now();
      lastStatus.current = first.status;
      if (isTestRunFinal(first.status)) {
        setState({ phase: 'finished', run: first, error: undefined, waitedMs: 0 });
        return;
      }
      setState({ phase: 'watching', run: first, error: undefined, waitedMs: undefined });
      poller.current = startPolling<ScriptTestRun>({
        fetch: (signal) => api.testRun(scriptId, first.id, signal),
        isDone: (run) => isTestRunFinal(run.status),
        onValue: (run) => {
          if (!alive.current) return;
          setState((current) => ({ ...current, run }));
          if (run.status !== lastStatus.current) {
            lastStatus.current = run.status;
            if (run.status === 'RUNNING') announce('Test run is running.');
          }
        },
        onSettled: (outcome) => {
          if (!alive.current) return;
          if (outcome.kind === 'done') {
            setState({ phase: 'finished', run: outcome.value, error: undefined, waitedMs: Date.now() - startedAt });
            announce(outcome.value.status === 'SUCCEEDED' ? 'Test run succeeded.' : `Test run failed. ${failureHelp(outcome.value.failure)}`);
            onFinished?.();
          } else if (outcome.kind === 'expired') {
            setState((current) => ({ phase: 'expired', run: outcome.last ?? current.run, error: undefined, waitedMs: Date.now() - startedAt }));
            announce('Still no final answer from the test run. The Studio stopped checking.');
          } else {
            setState((current) => ({ phase: 'failed', run: current.run, error: outcome.error, waitedMs: undefined }));
            announce('Could not read the test run.');
          }
        },
        maxDurationMs: watchLimitMs(timeoutMs),
      });
    },
    [api, scriptId, timeoutMs, announce, onFinished],
  );

  const start = useCallback(
    async (input: unknown) => {
      poller.current?.stop();
      setState({ phase: 'starting', run: undefined, error: undefined, waitedMs: undefined });
      const saved = await flush();
      if (!alive.current) return;
      if (!saved) {
        setState({ phase: 'blocked', run: undefined, error: undefined, waitedMs: undefined });
        announce('The draft could not be saved, so it was not run.');
        return;
      }
      try {
        const run = await api.startTestRun(scriptId, input);
        if (!alive.current) return;
        announce('Test run queued.');
        watch(run);
      } catch (error) {
        if (!alive.current) return;
        setState({ phase: 'failed', run: undefined, error, waitedMs: undefined });
        announce('The test run could not be started.');
      }
    },
    [api, scriptId, flush, watch, announce],
  );

  /** Shows a run of the history (and watches it if it has not ended). */
  const open = useCallback(
    async (testRunId: string) => {
      poller.current?.stop();
      setState({ phase: 'loading', run: undefined, error: undefined, waitedMs: undefined });
      try {
        const run = await api.testRun(scriptId, testRunId);
        if (!alive.current) return;
        watch(run);
      } catch (error) {
        if (!alive.current) return;
        setState({ phase: 'failed', run: undefined, error, waitedMs: undefined });
      }
    },
    [api, scriptId, watch],
  );

  /**
   * Looks again at the run on screen, for after the Studio gave up or was told to stop. The run stays
   * where it is while it is read again (it is the same run, and what it said last is still what the
   * platform said last), and watching starts over.
   */
  const again = useCallback(async () => {
    const shown = state.run;
    if (shown === undefined) return;
    poller.current?.stop();
    setState((current) => ({ ...current, phase: 'watching', error: undefined, waitedMs: undefined }));
    try {
      const fresh = await api.testRun(scriptId, shown.id);
      if (!alive.current) return;
      watch(fresh);
    } catch (error) {
      if (!alive.current) return;
      setState((current) => ({ phase: 'failed', run: current.run, error, waitedMs: undefined }));
    }
  }, [state.run, api, scriptId, watch]);

  const stop = useCallback(() => {
    poller.current?.stop();
    setState((current) => ({ ...current, phase: 'stopped' }));
    announce('Stopped checking the test run. It goes on in the platform.');
  }, [announce]);

  return { state, start, open, again, stop };
}
