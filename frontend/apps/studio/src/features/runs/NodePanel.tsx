import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { Link } from 'react-router';
import type { RuntimeEdge, RuntimeNode } from '../../api/types';
import type { RunModel } from '../../model/runGraph';
import { kindOf } from '../../model/registry';
import { nodeStatus } from '../../model/status';
import { Banner } from '../../ui/Banner';
import { ErrorState } from '../../ui/ErrorState';
import { StatusBadge } from '../../ui/StatusBadge';
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

function detailedKind(node: RuntimeNode): string {
  const summary = kindView(node);
  if (summary.variant === 'forward') return kindOf(node.type)?.label ?? node.type;
  const suffix = summary.variant === 'undo' ? /~c$/ : /~r$/;
  const forward = node.forward ?? node.id.replace(suffix, '');
  return summary.variant === 'undo' ? `Undo of ${forward}` : `Lookup for ${forward}`;
}

function AttemptsTable({ history, running }: { history: StepHistory; running: boolean }) {
  const now = useNow();
  if (history.attempts.length === 0) {
    return <p className="hint">No attempt of this step is in the journal events read so far.</p>;
  }
  return (
    <table className="data rn-attempts">
      <caption className="visually-hidden">Attempts of this step, from the journal</caption>
      <thead>
        <tr>
          <th scope="col">Attempt</th>
          <th scope="col">Outcome</th>
          <th scope="col">Started</th>
          <th scope="col">Ended</th>
          <th scope="col">Duration</th>
        </tr>
      </thead>
      <tbody>
        {history.attempts.map((attempt) => (
          <AttemptRow key={attempt.number} attempt={attempt} now={now} running={running} />
        ))}
      </tbody>
    </table>
  );
}

function AttemptRow({ attempt, now, running }: { attempt: Attempt; now: number; running: boolean }) {
  const started = attempt.startedAt ?? attempt.leasedAt;
  const open = attempt.endedAt === undefined && running ? openDuration(attempt, now) : attempt.durationMs;
  return (
    <tr>
      <td>{attempt.number}</td>
      <td>{outcomeWords(attempt.outcome)}</td>
      <td>{started ? <Moment at={started} /> : '—'}</td>
      <td>{attempt.endedAt ? <Moment at={attempt.endedAt} /> : '—'}</td>
      <td>{open === undefined ? '—' : `${formatDuration(open)}${attempt.endedAt === undefined ? ' so far' : ''}`}</td>
    </tr>
  );
}

function Facts({ node }: { node: RuntimeNode }) {
  const saga = sagaWords(node);
  return (
    <dl className="rn-facts">
      {node.nextAttemptAt ? (
        <>
          <dt>Next attempt</dt>
          <dd>
            <Relative at={node.nextAttemptAt} /> (<Moment at={node.nextAttemptAt} />)
          </dd>
        </>
      ) : null}
      {node.dueAt ? (
        <>
          <dt>Due</dt>
          <dd>
            <Relative at={node.dueAt} /> (<Moment at={node.dueAt} />)
          </dd>
        </>
      ) : null}
      {node.cancelRequested ? (
        <>
          <dt>Stop</dt>
          <dd>The step was asked to stop.</dd>
        </>
      ) : null}
      {saga ? (
        <>
          <dt>Saga</dt>
          <dd>
            {saga}
            {node.saga?.cause ? <> (cause: {clip(node.saga.cause, 60)})</> : null}
          </dd>
        </>
      ) : null}
      {node.childRunId ? (
        <>
          <dt>Child run</dt>
          <dd>
            <Link to={childPath(node.childRunId)}>{node.childRunId}</Link>
          </dd>
        </>
      ) : null}
    </dl>
  );
}

function Links({ edges, model, id, onSelect }: { edges: readonly RuntimeEdge[]; model: RunModel; id: string; onSelect: (id: string) => void }) {
  const { from, to } = useMemo(() => neighbours(edges, model.nodes, id), [edges, model.nodes, id]);
  const list = (items: typeof from, label: string) => (
    <div>
      <h4>{label}</h4>
      {items.length === 0 ? (
        <p className="hint">None.</p>
      ) : (
        <ul>
          {items.map((item, i) => (
            <li key={`${item.id}-${i}`}>
              <button type="button" className="link" onClick={() => onSelect(item.id)}>
                {item.id}
              </button>{' '}
              <span className="hint">
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
 * operator may take. It shows no data of the step: the journal has none (G144).
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
      <div className="rn-panel-head">
        <h2>{selectedId}</h2>
        <button type="button" className="btn" onClick={onClose}>
          Close details
        </button>
      </div>
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
            <dl>
              <div>
                <dt>Kind</dt>
                <dd>{kind}</dd>
              </div>
              <div>
                <dt>Attempts</dt>
                <dd>{node.attempt && node.attempt > 0 ? (attemptWords(node) ?? 'attempt 1') : 'not started'}</dd>
              </div>
              {node.failureCode ? (
                <div>
                  <dt>Failure code</dt>
                  <dd>{node.failureCode}</dd>
                </div>
              ) : null}
              {waiting ? (
                <div>
                  <dt>Waiting</dt>
                  <dd>{waiting}</dd>
                </div>
              ) : null}
            </dl>
          </div>
          <Facts node={node} />

          <h3>Actions</h3>
          <OperationList operations={operations} label={`Actions for ${selectedId}`} onChoose={onChoose} />

          <h3>Attempts</h3>
          {snapshot.error !== undefined ? <ErrorState error={snapshot.error} what="Reading the journal" onRetry={() => void journal.loadMore()} /> : null}
          {!snapshot.started || (snapshot.loading && snapshot.entries.length === 0) ? <p role="status">Reading the journal…</p> : null}
          {snapshot.started ? <AttemptsTable history={history} running={runActive} /> : null}
          {snapshot.started && !snapshot.complete && !snapshot.loading ? (
            <p className="hint">
              Attempts are told from the {snapshot.entries.length} journal events read so far; the journal has more.{' '}
              <button type="button" className="btn" onClick={() => void journal.loadAll()}>
                Read more of the journal
              </button>
            </p>
          ) : null}
          {history.marks.length > 0 ? (
            <>
              <h3>Decisions and notes</h3>
              <ul className="rn-marks">
                {history.marks.map((mark) => (
                  <li key={mark.seq}>
                    <Moment at={mark.at} /> {eventLabel(mark.eventType)}
                    {mark.actor ? <span className="hint"> by {clip(mark.actor, 60)}</span> : null}
                  </li>
                ))}
              </ul>
            </>
          ) : null}

          <h3>Trace</h3>
          {traces.length > 0 ? (
            <ul className="rn-traces">
              {traces.slice(0, 5).map((trace) => (
                <li key={trace}>
                  <code>{clip(trace, 64)}</code> <CopyButton text={trace} what="trace id" />
                </li>
              ))}
            </ul>
          ) : knowsTraces ? (
            <p>no trace (the worker did not send one for this step).</p>
          ) : (
            <p className="hint">Trace ids come with the journal events, which are not read yet.</p>
          )}
          <p className="hint">The journal keeps no input or output of a step, only a digest: the payload is redacted.</p>
          {retainedPayloads.length > 0 ? (
            <div className="rn-payload-events">
              <h3>Retained payloads</h3>
              {retainedPayloads.map((entry) => (
                <PayloadInspector
                  key={entry.seq}
                  runId={runId}
                  selector={{ source: 'journal', sequence: entry.seq }}
                  label={`Inspect redacted payload from event ${entry.seq}`}
                  compact
                />
              ))}
            </div>
          ) : null}

          {node.type === 'database' && node.status === 'SUCCEEDED' ? <DatabaseResultViewer runId={runId} nodeId={selectedId} /> : null}

          <h3>Structure</h3>
          <Links edges={model.edges} model={model} id={selectedId} onSelect={onSelect} />
          {node.forward ? (
            <p>
              About step{' '}
              <button type="button" className="link" onClick={() => onSelect(node.forward as string)}>
                {node.forward}
              </button>
              .
            </p>
          ) : null}
          {node.saga && node.saga.irreversible.length > 0 ? (
            <p>
              Cannot be undone:{' '}
              {node.saga.irreversible.slice(0, 20).map((id) => (
                <button key={id} type="button" className="link" onClick={() => onSelect(id)}>
                  {id}
                </button>
              ))}
            </p>
          ) : null}

          <p>
            <button type="button" className="btn" onClick={() => onShowInTimeline(selectedId)}>
              Show only this step in the timeline
            </button>
          </p>
        </>
      )}
    </aside>
  );
}
