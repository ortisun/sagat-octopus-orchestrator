import { useRef } from 'react';
import { Banner, Button, Code, Facts, FocusHeading, Heading, Hint, Row, Stack, StatusBadge, Text } from '../../ui';
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
    <div data-testid="test-result">
      <Stack gap={3}>
        <Row>
          <FocusHeading level={3} ref={heading}>
            Result
          </FocusHeading>
          <StatusBadge view={testRunStatus(run.status)} />
          {phase === 'watching' ? <Button onClick={keepFocus(onStop)}>Stop watching</Button> : null}
        </Row>

        {run.status === 'QUEUED' ? <Text>Waiting for a sandbox runner to take the run. Nothing has run yet.</Text> : null}
        {run.status === 'RUNNING' ? <Text>A runner has it: it builds the draft and runs it. There is no result yet.</Text> : null}
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
          <Stack gap={1}>
            <Heading level={4}>Returned</Heading>
            <LogView text={result?.text ?? ''} label="Value the script returned" empty="(the platform answer has no result field)" />
          </Stack>
        ) : null}

        {final ? (
          <>
            <Stack gap={1}>
              <Heading level={4}>Log</Heading>
              <Hint>The platform keeps one filtered log (what the script wrote with console.log or octopus.log and what it printed); it does not keep standard output and standard error apart. Credentials and secret values are replaced by [REDACTED].</Hint>
              <LogView text={run.log} label="Log of the run" platformCut={run.logTruncated} empty="(the script wrote nothing)" />
            </Stack>

            <Stack gap={1}>
              <Heading level={4}>Refused</Heading>
              {run.denied.length === 0 ? (
                <Text>The script was not refused anything.</Text>
              ) : (
                <>
                  <Text>The sandbox refused these requests of the script (also audited, once each):</Text>
                  <ul className="denied">
                    {run.denied.slice(0, 32).map((entry, index) => (
                      <li key={`${index}:${entry.slice(0, 40)}`}>
                        <Code>{clipLabel(entry, 200)}</Code>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </Stack>

            <Stack gap={1}>
              <Heading level={4}>What it used</Heading>
              <Facts
                items={[
                  { term: 'Time', value: `${formatDuration(run.wallMs)} of ${formatDuration(policy.timeoutMs)} allowed` },
                  { term: 'Start', value: `${formatDuration(run.startMs)} from asking for a container to the script’s first instruction` },
                  { term: 'Memory (peak)', value: run.memoryPeakBytes === undefined ? '—' : `${formatBytes(run.memoryPeakBytes)} of ${policy.memoryMib} MiB allowed` },
                  { term: 'CPU', value: `The platform does not report CPU time (G111); the limit is ${policy.cpuMillis} millicores.` },
                ]}
              />
            </Stack>

            {canUseSchemas && (run.inferredInputSchema || run.inferredOutputSchema) ? (
              <Stack gap={1}>
                <Heading level={4}>Schemas the platform inferred from this run</Heading>
                <Hint>Inferred from one input and one result: a starting point, not a contract. Using one puts it in the schema editor, where you can change it.</Hint>
                <Row>
                  {run.inferredInputSchema ? <Button onClick={() => onUseSchema('inputSchema', run.inferredInputSchema as Record<string, unknown>)}>Use as input schema</Button> : null}
                  {run.inferredOutputSchema ? <Button onClick={() => onUseSchema('outputSchema', run.inferredOutputSchema as Record<string, unknown>)}>Use as output schema</Button> : null}
                </Row>
              </Stack>
            ) : null}
          </>
        ) : null}

        <Facts
          items={[
            {
              term: 'Test run',
              value: (
                <>
                  <Code>{run.id}</Code> (quote it, and the trace id of any error, when asking for help)
                </>
              ),
            },
            {
              term: 'Asked by',
              value: (
                <>
                  {clipLabel(run.createdBy)} at <time dateTime={run.createdAt}>{formatTime(run.createdAt)}</time>
                </>
              ),
            },
          ]}
        />
        {run.input !== undefined ? (
          <Row>
            <Button onClick={() => onUseInput(run.input)}>Use this run&apos;s input</Button>
          </Row>
        ) : null}
      </Stack>
    </div>
  );
}
