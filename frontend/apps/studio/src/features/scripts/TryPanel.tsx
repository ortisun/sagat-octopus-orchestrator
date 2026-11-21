import { useMemo } from 'react';
import { Banner, Button, Code, ColumnHeader, Empty, ErrorState, Heading, Hint, Loading, Row, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text, VisuallyHidden } from '../../ui';
import type { Script, ScriptTestRunSummary } from './api';
import { CodeEditor } from './CodeEditor';
import { checkInputText, checkSchemaText, valueToText } from './jsonText';
import { failureHelp, testedness, testRunStatus } from './status';
import type { Testedness } from './status';
import { TestRunResult } from './TestRunResult';
import type { useTestRun } from './useTestRun';
import { clipLabel, formatDuration, formatTime } from './text';

export interface TryPanelProps {
  script: Script;
  /** The author's copy of the input schema, as text: a sample input is checked against it. */
  inputSchemaText: string;
  inputText: string;
  onInputText: (text: string) => void;
  test: ReturnType<typeof useTestRun>;
  runs: ScriptTestRunSummary[] | undefined;
  runsError: unknown;
  onRetryRuns: () => void;
  onUseSchema: (which: 'inputSchema' | 'outputSchema', schema: Record<string, unknown>) => void;
  draftUnsaved: boolean;
  draftUpdatedAt: string;
}

/** What the test says about the draft, in words, for the publish dialog and the panel. */
export function testednessText(t: Testedness): string {
  switch (t.kind) {
    case 'passed':
      return `A test run of this draft succeeded (${formatTime(t.run.createdAt)}).`;
    case 'failed':
      return `The last test run of this draft failed (${t.run.failure ?? 'no code'}).`;
    case 'pending':
      return 'A test run of this draft has not finished.';
    case 'untested':
      if (t.reason === 'unsaved') return 'The draft has changes that are not saved, so no test run covers them.';
      if (t.reason === 'draft-changed') return 'The draft changed after the last test run.';
      return 'This draft has not been tried in the sandbox.';
  }
}

export function TryPanel({ script, inputSchemaText, inputText, onInputText, test, runs, runsError, onRetryRuns, onUseSchema, draftUnsaved, draftUpdatedAt }: TryPanelProps) {
  const input = useMemo(() => checkInputText(inputText), [inputText]);
  const schema = useMemo(() => {
    const check = checkSchemaText(inputSchemaText);
    return check.kind === 'ok' ? ({ kind: 'schema', schema: check.value } as const) : undefined;
  }, [inputSchemaText]);
  const busy = test.state.phase === 'starting' || test.state.phase === 'loading' || test.state.phase === 'watching';
  const tested = useMemo(() => (runs ? testedness(runs, draftUpdatedAt, draftUnsaved) : undefined), [runs, draftUpdatedAt, draftUnsaved]);
  const run = () => {
    if (input.kind === 'invalid') return;
    void test.start(input.kind === 'empty' ? undefined : input.value);
  };
  return (
    <Stack gap={3}>
      <Text>
        Runs the draft in the sandbox without publishing it. The Studio saves your changes first, because the sandbox runs the draft as the platform has it. A test run has no secrets of the namespace (<Code>octopus.secret</Code> is always refused), and reaches the network only if the script&apos;s policy
        and the namespace&apos;s policy both allow it.
      </Text>
      <Stack gap={1}>
        <Text>
          <strong>Test input (JSON)</strong>
        </Text>
        <Hint id="test-input-hint">
          {schema ? 'Checked against the input schema of the draft as you type. ' : 'No input schema yet, so only the JSON is checked. '}
          Sent as the input of the run (at most 240 KiB). Empty: the script receives null.
        </Hint>
        <CodeEditor label="Test input (JSON)" describedBy="test-input-hint" model="testInput" language="json" value={inputText} onChange={onInputText} jsonSchema={schema} height="10rem" />
        {input.kind === 'invalid' ? <Banner tone="error">{input.message}</Banner> : null}
      </Stack>
      <Row>
        <Button variant="primary" onClick={run} disabled={busy || input.kind === 'invalid'}>
          {test.state.phase === 'starting' ? 'Starting…' : 'Run in the sandbox'}
        </Button>
        {tested ? <Hint>{testednessText(tested)}</Hint> : null}
      </Row>

      {test.state.phase === 'blocked' ? (
        <Banner tone="warn" title="Not run.">
          The draft could not be saved first, so the sandbox would have run an older copy. See the messages about the draft above.
        </Banner>
      ) : null}
      {test.state.phase === 'loading' ? <Loading>Loading the test run…</Loading> : null}
      {test.state.phase === 'failed' ? <ErrorState error={test.state.error} what="The test run" /> : null}
      {test.state.run ? (
        <TestRunResult
          run={test.state.run}
          phase={test.state.phase}
          waitedMs={test.state.waitedMs}
          policy={script.policy}
          canUseSchemas
          onUseInput={(value) => onInputText(`${valueToText(value)}\n`)}
          onUseSchema={onUseSchema}
          onStop={test.stop}
          onAgain={() => void test.again()}
        />
      ) : null}

      <Heading level={3}>Earlier test runs</Heading>
      {runsError !== undefined && runsError !== null ? (
        <ErrorState error={runsError} what="Reading the test runs" onRetry={onRetryRuns} />
      ) : runs === undefined ? (
        <Loading>Loading the test runs…</Loading>
      ) : runs.length === 0 ? (
        <Empty title="No test runs yet." headingLevel={4} />
      ) : (
        <Table label="Test runs of this script, newest first">
          <TableHead>
            <TableRow>
              <ColumnHeader scope="col">Asked</ColumnHeader>
              <ColumnHeader scope="col">Result</ColumnHeader>
              <ColumnHeader scope="col">Time</ColumnHeader>
              <ColumnHeader scope="col">By</ColumnHeader>
              <ColumnHeader scope="col">
                <VisuallyHidden>Actions</VisuallyHidden>
              </ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {runs.map((item) => (
              <TableRow key={item.id}>
                <ColumnHeader scope="row">
                  <time dateTime={item.createdAt}>{formatTime(item.createdAt)}</time>
                </ColumnHeader>
                <TableCell>
                  <StatusBadge view={testRunStatus(item.status)} />
                  {item.failure ? (
                    <>
                      {' '}
                      <span title={failureHelp(item.failure)}>
                        <Code>{clipLabel(item.failure, 40)}</Code>
                      </span>
                    </>
                  ) : null}
                </TableCell>
                <TableCell>{formatDuration(item.wallMs)}</TableCell>
                <TableCell>{clipLabel(item.createdBy)}</TableCell>
                <TableCell>
                  <Button onClick={() => void test.open(item.id)} aria-label={`Open the test run of ${formatTime(item.createdAt)}`}>
                    Open
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </Stack>
  );
}
