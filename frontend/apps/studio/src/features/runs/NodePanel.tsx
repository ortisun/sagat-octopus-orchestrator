import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type { RuntimeEdge, RuntimeNode } from '../../api/types';
import type { RunModel } from '../../model/runGraph';
import { nodeStatus } from '../../model/status';
import { Banner, Button, Code, ColumnHeader, ErrorState, Facts, Heading, Hint, Link, Loading, Row, Stack, StatusBadge, Table, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
import { openDuration, outcomeWords, stepHistory } from './attempts';
import type { Attempt, StepHistory } from './attempts';
import { neighbours, stateWords } from './canvas';
import { CopyButton } from './CopyButton';
import { attemptWords, kindView, waitingWords } from './display';
import { eventLabel } from '../../model/events';
import type { JournalStore } from './journal';
import { Moment, Relative, useNow } from './Now';
import { OperationList } from './Operations';
import type { Operation } from './ops';
import { childPath, sagaWords } from './RunNodeCard';
import { clip, formatDuration } from './time';
import { DatabaseResultViewer } from '../database/DatabaseResultViewer';
import { PayloadInspector } from './PayloadInspection';
import { PayloadDisclosureView } from './PayloadDisclosure';

function detailedKind(node: RuntimeNode): string {
  const summary = kindView(node);
  if (summary.variant === 'forward') return summary.label;
  const suffix = summary.variant === 'undo' ? /~c$/ : /~r$/;
  const forward = node.forward ?? node.id.replace(suffix, '');
  return summary.variant === 'undo' ? `Undo of ${forward}` : `Lookup for ${forward}`;
}

function AttemptsTable({ history, running }: { history: StepHistory; running: boolean }) {
  const now = useNow();
  if (history.attempts.length === 0) {
    return <Hint>No attempt of this step is in the journal events read so far.</Hint>;
  }
  return (
    <Table label="Attempts of this step, from the journal">
      <TableHead>
        <TableRow>
          <ColumnHeader scope="col">Attempt</ColumnHeader>
          <ColumnHeader scope="col">Outcome</ColumnHeader>
          <ColumnHeader scope="col">Started</ColumnHeader>
          <ColumnHeader scope="col">Ended</ColumnHeader>
          <ColumnHeader scope="col">Duration</ColumnHeader>
        </TableRow>
      </TableHead>
      <TableBody>
        {history.attempts.map((attempt) => (
          <AttemptRow key={attempt.number} attempt={attempt} now={now} running={running} />
        ))}
      </TableBody>
    </Table>
  );
}

function AttemptRow({ attempt, now, running }: { attempt: Attempt; now: number; running: boolean }) {
  const started = attempt.startedAt ?? attempt.leasedAt;
  const open = attempt.endedAt === undefined && running ? openDuration(attempt, now) : attempt.durationMs;
  return (
    <TableRow>
      <TableCell>{attempt.number}</TableCell>
      <TableCell>{outcomeWords(attempt.outcome)}</TableCell>
      <TableCell>{started ? <Moment at={started} /> : '—'}</TableCell>
      <TableCell>{attempt.endedAt ? <Moment at={attempt.endedAt} /> : '—'}</TableCell>
      <TableCell>{open === undefined ? '—' : `${formatDuration(open)}${attempt.endedAt === undefined ? ' so far' : ''}`}</TableCell>
    </TableRow>
  );
}

/** What the platform says of the step beyond its state: when it is tried next, when it is due, its saga, its child run. */
function StepFacts({ node }: { node: RuntimeNode }) {
  const saga = sagaWords(node);
  const items = [
    ...(node.nextAttemptAt
      ? [
          {
            term: 'Next attempt',
            value: (
              <>
                <Relative at={node.nextAttemptAt} /> (<Moment at={node.nextAttemptAt} />)
              </>
            ),
          },
        ]
      : []),
    ...(node.dueAt
      ? [
          {
            term: 'Due',
            value: (
              <>
                <Relative at={node.dueAt} /> (<Moment at={node.dueAt} />)
              </>
            ),
          },
        ]
      : []),
    ...(node.cancelRequested ? [{ term: 'Stop', value: 'The step was asked to stop.' }] : []),
    ...(saga
      ? [
          {
            term: 'Saga',
            value: (
              <>
                {saga}
                {node.saga?.cause ? <> (cause: {clip(node.saga.cause, 60)})</> : null}
              </>
            ),
          },
        ]
      : []),
    ...(node.childRunId ? [{ term: 'Child run', value: <Link to={childPath(node.childRunId)}>{node.childRunId}</Link> }] : []),
  ];
  return items.length === 0 ? null : <Facts items={items} />;
}

function Links({ edges, model, id, onSelect }: { edges: readonly RuntimeEdge[]; model: RunModel; id: string; onSelect: (id: string) => void }) {
  const { from, to } = useMemo(() => neighbours(edges, model.nodes, id), [edges, model.nodes, id]);
  const list = (items: typeof from, label: string) => (
    <div>
      <Heading level={4}>{label}</Heading>
      {items.length === 0 ? (
        <Hint>None.</Hint>
      ) : (
        <ul className="rn-plain-list">
          {items.map((item, i) => (
            <li key={`${item.id}-${i}`}>
              <Button variant="ghost" size="sm" onClick={() => onSelect(item.id)}>
                {item.id}
              </Button>{' '}
              <span className="rn-quiet">
                {item.kind}, {stateWords(item.state)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
  return (
    <div className="rn-links">
      {list(from, 'Comes after')}
      {list(to, 'Leads to')}
    </div>
  );
}

/**
 * What the Studio knows of one step: its state, its attempts and their durations (told from the
 * journal), how it connects to the others, the decisions made about it, and the actions an
 * operator may take. Journal entries carry only payload references; retained values are fetched
 * separately through the classified disclosure operation.
 */
export function NodePanel({
  node,
  runId,
  selectedId,
  model,
  journal,
  runActive,
  operations,
  onChoose,
  onSelect,
  onClose,
  onShowInTimeline,
}: {
  node: RuntimeNode | undefined;
  runId: string;
  selectedId: string;
  model: RunModel;
  journal: JournalStore;
  runActive: boolean;
  operations: readonly Operation[];
  onChoose: (operation: Operation, opener: HTMLElement | null) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
  onShowInTimeline: (id: string) => void;
}) {
  const snapshot = useSyncExternalStore(journal.subscribe, journal.getSnapshot);
  const panel = useRef<HTMLElement>(null);
  // Beside the list there is nothing to do; under it (a narrow window) the details are brought into view.
  useEffect(() => {
    panel.current?.scrollIntoView?.({ block: 'nearest' });
  }, [selectedId]);
  // The attempts are told from the journal: opening a step reads it (up to a limit of pages).
  useEffect(() => {
    void journal.loadAll();
  }, [journal]);
  const history = useMemo(() => stepHistory(snapshot.entries, selectedId), [snapshot.entries, selectedId]);
  const retainedPayloads = snapshot.entries.filter((entry) => entry.stepId === selectedId && entry.payloadRef !== null);
  const traces = [...new Set(snapshot.entries.filter((e) => e.stepId === selectedId && e.detail === 'journal' && e.traceId).map((e) => e.traceId as string))];
  const knowsTraces = snapshot.entries.some((e) => e.stepId === selectedId && e.detail === 'journal');
  const kind = node === undefined ? undefined : detailedKind(node);
  const waiting = node === undefined ? undefined : waitingWords(node);
  return (
    <aside ref={panel} className="rn-panel" aria-label={`Details of ${selectedId}`} data-testid="node-panel">
      <Stack gap={3}>
        <Row justify="between" align="start" wrap={false}>
          <Heading level={2} look={3} className="rn-id">
            {selectedId}
          </Heading>
          <Button onClick={onClose}>Close details</Button>
        </Row>
        {node === undefined || kind === undefined ? (
          <Banner tone="warn">This step is not in the graph any more. It may have been removed when the graph was read again.</Banner>
        ) : (
          <>
            <div className="rn-selected-step" role="group" aria-label="Selected step">
              <div className="rn-selected-step-name">
                <span className="rn-micro-label">Selected step</span>
                <strong>{selectedId}</strong>
                <StatusBadge view={nodeStatus(node.status)} />
              </div>
              <Facts
                columns="multi"
                items={[
                  { term: 'Kind', value: kind },
                  { term: 'Attempts', value: node.attempt && node.attempt > 0 ? (attemptWords(node) ?? 'attempt 1') : 'not started' },
                  ...(node.failureCode ? [{ term: 'Failure code', value: node.failureCode }] : []),
                  ...(waiting ? [{ term: 'Waiting', value: waiting }] : []),
                ]}
              />
            </div>
            <StepFacts node={node} />

            <Heading level={3}>Actions</Heading>
            <OperationList operations={operations} label={`Actions for ${selectedId}`} onChoose={onChoose} />

            <Heading level={3}>Attempts</Heading>
            {snapshot.error !== undefined ? <ErrorState error={snapshot.error} what="Reading the journal" onRetry={() => void journal.loadMore()} /> : null}
            {!snapshot.started || (snapshot.loading && snapshot.entries.length === 0) ? <Loading>Reading the journal…</Loading> : null}
            {snapshot.started ? <AttemptsTable history={history} running={runActive} /> : null}
            {snapshot.started && !snapshot.complete && !snapshot.loading ? (
              <Row>
                <Hint>Attempts are told from the {snapshot.entries.length} journal events read so far; the journal has more.</Hint>
                <Button onClick={() => void journal.loadAll()}>Read more of the journal</Button>
              </Row>
            ) : null}
            {history.marks.length > 0 ? (
              <>
                <Heading level={3}>Decisions and notes</Heading>
                <ul className="rn-plain-list">
                  {history.marks.map((mark) => (
                    <li key={mark.seq}>
                      <Moment at={mark.at} /> {eventLabel(mark.eventType)}
                      {mark.actor ? <span className="rn-quiet"> by {clip(mark.actor, 60)}</span> : null}
                    </li>
                  ))}
                </ul>
              </>
            ) : null}

            <Heading level={3}>Trace</Heading>
            {traces.length > 0 ? (
              <ul className="rn-plain-list">
                {traces.slice(0, 5).map((trace) => (
                  <li key={trace}>
                    <Code>{clip(trace, 64)}</Code> <CopyButton text={trace} what="trace id" />
                  </li>
                ))}
              </ul>
            ) : knowsTraces ? (
              <Text>no trace (the worker did not send one for this step).</Text>
            ) : (
              <Hint>Trace ids come with the journal events, which are not read yet.</Hint>
            )}
            <Hint>The journal event holds only a payload digest. Retained values are read separately under the workflow&apos;s classification policy.</Hint>
            {retainedPayloads.length > 0 ? (
              <Stack gap={2}>
                <Heading level={3}>Retained payloads</Heading>
                {retainedPayloads.map((entry) => (
                  <Stack gap={1} key={entry.seq}>
                    <PayloadDisclosureView
                      runId={runId}
                      selector={{ source: 'journal', sequence: entry.seq }}
                      label={`View permitted values from event ${entry.seq}`}
                      compact
                    />
                    <PayloadInspector
                      runId={runId}
                      selector={{ source: 'journal', sequence: entry.seq }}
                      label={`Inspect redacted payload from event ${entry.seq}`}
                      compact
                    />
                  </Stack>
                ))}
              </Stack>
            ) : null}

            {node.type === 'database' && node.status === 'SUCCEEDED' ? <DatabaseResultViewer runId={runId} nodeId={selectedId} /> : null}

            <Heading level={3}>Structure</Heading>
            <Links edges={model.edges} model={model} id={selectedId} onSelect={onSelect} />
            {node.forward ? (
              <Row gap={1}>
                <Text as="span">About step</Text>
                <Button variant="ghost" size="sm" onClick={() => onSelect(node.forward as string)}>
                  {node.forward}
                </Button>
              </Row>
            ) : null}
            {node.saga && node.saga.irreversible.length > 0 ? (
              <Row gap={1}>
                <Text as="span">Cannot be undone:</Text>
                {node.saga.irreversible.slice(0, 20).map((id) => (
                  <Button key={id} variant="ghost" size="sm" onClick={() => onSelect(id)}>
                    {id}
                  </Button>
                ))}
              </Row>
            ) : null}

            <div>
              <Button onClick={() => onShowInTimeline(selectedId)}>Show only this step in the timeline</Button>
            </div>
          </>
        )}
      </Stack>
    </aside>
  );
}
