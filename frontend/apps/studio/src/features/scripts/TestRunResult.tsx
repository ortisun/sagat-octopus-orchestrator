import { useRef } from 'react';
import { StatusBadge } from '../../ui/StatusBadge';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import type { ScriptPolicy, ScriptTestRun } from './api';
import { LogView } from './LogView';
import type { TestPhase } from './useTestRun';
import { valueToText } from './jsonText';
import { failureHelp, testRunStatus } from './status';
import { clipLabel, clipText, formatBytes, formatDuration, formatTime, sanitizeLog } from './text';

/** The result of a test run: only what the platform has said, labelled as the platform's, never filled in. */
export function TestRunResult({
  run,
  phase,
  waitedMs,
  policy,
  canUseSchemas,
  onUseInput,
  onUseSchema,
  onStop,
  onAgain,
}: {
  run: ScriptTestRun;
  phase: TestPhase;
  waitedMs: number | undefined;
  policy: ScriptPolicy;
  canUseSchemas: boolean;
  onUseInput: (input: unknown) => void;
  onUseSchema: (which: 'inputSchema' | 'outputSchema', schema: Record<string, unknown>) => void;
  onStop: () => void;
  onAgain: () => void;
}) {
  const final = run.status === 'SUCCEEDED' || run.status === 'FAILED';
  const heading = useRef<HTMLHeadingElement>(null);
  // The button that was pressed ("Stop watching", "Check again") goes away with the state it belonged to:
  // focus goes to the result, which stays, instead of to nowhere.
  const keepFocus = (action: () => void) => () => {
    action();
    heading.current?.focus();
  };
  const result = run.result === undefined ? undefined : clipText(sanitizeLog(valueToText(run.result)), { maxChars: 64 * 1024, maxLines: 1000, maxLineChars: 4000 });
  return (
    <div className="test-result" data-testid="test-result">
      <div className="toolbar">
        <h3 ref={heading} tabIndex={-1}>
          Result
        </h3>
        <StatusBadge view={testRunStatus(run.status)} />
        {phase === 'watching' ? <Button onClick={keepFocus(onStop)}>Stop watching</Button> : null}
      </div>

      {run.status === 'QUEUED' ? <p>Waiting for a sandbox runner to take the run. Nothing has run yet.</p> : null}
      {run.status === 'RUNNING' ? <p>A runner has it: it builds the draft and runs it. There is no result yet.</p> : null}
      {phase === 'expired' || phase === 'stopped' ? (
        <Banner tone="warn" title={phase === 'expired' ? 'Still not final.' : 'Not watching any more.'}>
          {phase === 'expired' ? `The Studio looked for ${formatDuration(waitedMs)} and stopped. ` : ''}
          The run is {run.status === 'QUEUED' ? 'still queued: a sandbox runner may be busy or not running' : 'still going'} as far as the platform last said. <Button onClick={keepFocus(onAgain)}>Check again</Button>
        </Banner>
      ) : null}

      {run.status === 'FAILED' ? (
        <Banner tone="error" title={`Failed: ${run.failure ?? 'no failure code'}.`}>
          {failureHelp(run.failure)}
          {run.retryable === true ? ' The platform marks this kind of failure as one that may pass on another try.' : ''}
        </Banner>
      ) : null}
      {run.status === 'SUCCEEDED' ? (
        <>
          <h4>Returned</h4>
          <LogView text={result?.text ?? ''} label="Value the script returned" empty="(the platform answer has no result field)" />
        </>
      ) : null}

      {final ? (
        <>
          <h4>Log</h4>
          <p className="hint">The platform keeps one filtered log (what the script wrote with console.log or octopus.log and what it printed); it does not keep standard output and standard error apart. Credentials and secret values are replaced by [REDACTED].</p>
          <LogView text={run.log} label="Log of the run" platformCut={run.logTruncated} empty="(the script wrote nothing)" />

          <h4>Refused</h4>
          {run.denied.length === 0 ? (
            <p>The script was not refused anything.</p>
          ) : (
            <>
              <p>The sandbox refused these requests of the script (also audited, once each):</p>
              <ul className="denied">
                {run.denied.slice(0, 32).map((entry, index) => (
                  <li key={`${index}:${entry.slice(0, 40)}`}>
                    <code>{clipLabel(entry, 200)}</code>
                  </li>
                ))}
              </ul>
            </>
          )}

          <h4>What it used</h4>
          <dl className="numbers">
            <dt>Time</dt>
            <dd>
              {formatDuration(run.wallMs)} of {formatDuration(policy.timeoutMs)} allowed
            </dd>
            <dt>Start</dt>
            <dd>{formatDuration(run.startMs)} from asking for a container to the script&apos;s first instruction</dd>
            <dt>Memory (peak)</dt>
            <dd>{run.memoryPeakBytes === undefined ? '—' : `${formatBytes(run.memoryPeakBytes)} of ${policy.memoryMib} MiB allowed`}</dd>
            <dt>CPU</dt>
            <dd>
              The platform does not report CPU time (G111); the limit is {policy.cpuMillis} millicores.
            </dd>
          </dl>

          {canUseSchemas && (run.inferredInputSchema || run.inferredOutputSchema) ? (
            <>
              <h4>Schemas the platform inferred from this run</h4>
              <p className="hint">Inferred from one input and one result: a starting point, not a contract. Using one puts it in the schema editor, where you can change it.</p>
              <div className="toolbar">
                {run.inferredInputSchema ? <Button onClick={() => onUseSchema('inputSchema', run.inferredInputSchema as Record<string, unknown>)}>Use as input schema</Button> : null}
                {run.inferredOutputSchema ? <Button onClick={() => onUseSchema('outputSchema', run.inferredOutputSchema as Record<string, unknown>)}>Use as output schema</Button> : null}
              </div>
            </>
          ) : null}
        </>
      ) : null}

      <dl className="numbers">
        <dt>Test run</dt>
        <dd>
          <code>{run.id}</code> (quote it, and the trace id of any error, when asking for help)
        </dd>
        <dt>Asked by</dt>
        <dd>
          {clipLabel(run.createdBy)} at <time dateTime={run.createdAt}>{formatTime(run.createdAt)}</time>
        </dd>
      </dl>
      {run.input !== undefined ? (
        <div className="toolbar">
          <Button onClick={() => onUseInput(run.input)}>Use this run&apos;s input</Button>
        </div>
      ) : null}
    </div>
  );
}
