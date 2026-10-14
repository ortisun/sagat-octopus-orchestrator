import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { useApp } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { eventLabel, eventTone } from '../../model/events';
import { ErrorState } from '../../ui/ErrorState';
import { CopyButton } from './CopyButton';
import { digestView } from './display';
import { filterEntries } from './journal';
import type { Entry, EntryFilter, JournalStore } from './journal';
import { Moment } from './Now';
import { ScrollRegion } from './ScrollRegion';
import { clip } from './time';
import { useWindowedRows } from './useWindow';
import { PayloadInspector } from './PayloadInspection';

const ROW_ESTIMATE = 48;
const COLUMNS = 9;
/** Pages are read on their own as the list is scrolled, up to this many events; beyond it a person asks for more. */
const AUTO_LOAD_UP_TO = 10_000;
/** How close to the end of what is read the list is before the next page is asked for. */
const NEAR_END = 25;

/** What the kind of a cause is called to a person. */
const CAUSE_WORDS: Record<string, string> = {
  failure: 'failed',
  retry: 'retried',
  review: 'waits for a person',
  abort: 'saga stopped going forward',
  unknown_outcome: 'outcome unknown',
  reconciliation: 'looked up at the provider',
  stop: 'run stopped',
  timeout: 'timed out',
  cancel: 'cancelled',
  decision: 'decided by a person',
  request: 'asked for',
  outcome: 'how the run ended',
};

const TONES: Record<string, { word: string; symbol: string; tone: string } | undefined> = {
  problem: { word: 'Problem', symbol: '⚠', tone: 'bad' },
  decision: { word: 'Decision', symbol: '✎', tone: 'waiting' },
};

/**
 * The journal of the run, which is the source of truth, read a page at a time and drawn a window at
 * a time. The payload of an event is never shown (the journal holds a digest only): the digest is
 * shown shortened with the whole of it to copy.
 */
export function TimelineView({ runId, journal, filter, onFilter, onSelect, selectedId }: { runId: string; journal: JournalStore; filter: EntryFilter; onFilter: (filter: EntryFilter) => void; onSelect: (id: string) => void; selectedId: string | undefined }) {
  const snapshot = useSyncExternalStore(journal.subscribe, journal.getSnapshot);
  useEffect(() => {
    journal.start();
  }, [journal]);
  const shown = useMemo(() => filterEntries(snapshot.entries, filter), [snapshot.entries, filter]);
  const scroller = useRef<HTMLDivElement>(null);
  const { items, padTop, padBottom, measure, virtualizer } = useWindowedRows({ count: shown.length, estimate: ROW_ESTIMATE, scroller });
  const last = items.at(-1)?.index ?? -1;

  // Near the end of what is read, read the next page.
  const { complete, loading, error } = snapshot;
  const total = snapshot.entries.length;
  useEffect(() => {
    if (complete || loading || error !== undefined || total >= AUTO_LOAD_UP_TO) return;
    if (last >= shown.length - NEAR_END) void journal.loadMore();
  }, [last, shown.length, complete, loading, error, total, journal]);

  const hasLive = snapshot.entries.some((entry) => entry.detail === 'live');
  const filtered = filter.problems || filter.decisions || filter.stepId.trim() !== '';

  return (
    <div className="rn-timeline">
      <div className="toolbar">
        <label className="rn-check">
          <input type="checkbox" checked={filter.problems} onChange={(event) => onFilter({ ...filter, problems: event.target.checked })} /> Only problems
        </label>
        <label className="rn-check">
          <input type="checkbox" checked={filter.decisions} onChange={(event) => onFilter({ ...filter, decisions: event.target.checked })} /> Only decisions
        </label>
        <label className="rn-find">
          <span>Step</span>
          <input type="search" className="input" value={filter.stepId} placeholder="a step id" autoComplete="off" spellCheck={false} onChange={(event) => onFilter({ ...filter, stepId: event.target.value })} />
        </label>
        {selectedId !== undefined && filter.stepId !== selectedId ? (
          <button type="button" className="btn" onClick={() => onFilter({ ...filter, stepId: selectedId })}>
            Only {clip(selectedId, 30)}
          </button>
        ) : null}
        <span className="spacer" />
        <button type="button" className="btn" disabled={shown.length === 0} onClick={() => virtualizer.scrollToIndex(shown.length - 1)}>
          Go to the latest event
        </button>
        <button type="button" className="btn" onClick={() => journal.reload()} title="Reads the journal again from the start">
          Read the journal again
        </button>
      </div>

      <TelemetryNote runId={runId} />

      <p className="hint">
        {snapshot.complete ? `All ${total} journal events are read.` : `${total} journal events read so far.`}
        {filtered ? ` ${shown.length} match the filters.` : ''}
        {snapshot.hidden > 0 ? ` ${snapshot.hidden} newer events arrived: read the rest of the journal to see them.` : ''}
        {snapshot.droppedLive ? ' Some live events were let go to save memory: read the journal again to see them.' : ''}
        {hasLive ? ' Events marked live show who did them and the trace when the journal is read again.' : ''}
      </p>
      {snapshot.error !== undefined ? <ErrorState error={snapshot.error} what="Reading the journal" onRetry={() => void journal.loadMore()} /> : null}

      {shown.length === 0 ? (
        <p className="empty" role={snapshot.loading ? 'status' : undefined}>
          {snapshot.loading ? 'Reading the journal…' : !snapshot.started ? 'The journal is not read yet.' : filtered ? 'No event matches the filters.' : 'The journal has no events yet.'}
        </p>
      ) : (
        <ScrollRegion label="Journal events" scroller={scroller}>
          <table className="data rn-fixed rn-journal" aria-rowcount={shown.length + 1}>
            <caption className="visually-hidden">Journal events, oldest first</caption>
            <thead>
              <tr aria-rowindex={1}>
                <th scope="col">Seq</th>
                <th scope="col">Time</th>
                <th scope="col">Event</th>
                <th scope="col">Cause</th>
                <th scope="col">Step</th>
                <th scope="col">Attempt</th>
                <th scope="col">Actor</th>
                <th scope="col">Trace</th>
                <th scope="col">Payload</th>
              </tr>
            </thead>
            <tbody>
              {padTop > 0 ? (
                <tr aria-hidden="true" className="rn-pad">
                  <td colSpan={COLUMNS} style={{ height: padTop }} />
                </tr>
              ) : null}
              {items.map((item) => {
                const entry = shown[item.index];
                return entry === undefined ? null : <EntryRow key={entry.seq} runId={runId} entry={entry} index={item.index} measure={measure} selected={entry.stepId !== null && entry.stepId === selectedId} onSelect={onSelect} />;
              })}
              {padBottom > 0 ? (
                <tr aria-hidden="true" className="rn-pad">
                  <td colSpan={COLUMNS} style={{ height: padBottom }} />
                </tr>
              ) : null}
            </tbody>
          </table>
        </ScrollRegion>
      )}

      {!snapshot.complete ? (
        <p>
          <button type="button" className="btn" disabled={snapshot.loading} onClick={() => void journal.loadMore()}>
            {snapshot.loading ? 'Reading…' : 'Load more events'}
          </button>
        </p>
      ) : null}
    </div>
  );
}

function EntryRow({ runId, entry, index, measure, selected, onSelect }: { runId: string; entry: Entry; index: number; measure: (element: Element | null) => void; selected: boolean; onSelect: (id: string) => void }) {
  const tone = TONES[eventTone(entry.eventType)];
  const digest = digestView(entry.payloadRef);
  const live = entry.detail === 'live';
  return (
    <tr ref={measure} data-index={index} aria-rowindex={index + 2} data-tone={tone?.tone} data-selected={selected ? 'true' : undefined}>
      <td>{entry.seq}</td>
      <td>
        <Moment at={entry.occurredAt} />
      </td>
      <td>
        {eventLabel(entry.eventType)}
        {tone ? (
          <>
            {' '}
            <span className="badge" data-tone={tone.tone}>
              <span aria-hidden="true">{tone.symbol}</span> {tone.word}
            </span>
          </>
        ) : null}
        <br />
        <code className="hint">{clip(entry.eventType, 40)}</code>
        {live ? <span className="hint"> (live)</span> : null}
      </td>
      <td>
        {entry.cause !== null ? (
          <>
            <code>{clip(entry.cause.code, 40)}</code>
            <br />
            <span className="hint">{CAUSE_WORDS[entry.cause.kind] ?? entry.cause.kind}</span>
          </>
        ) : live ? (
          '—'
        ) : (
          <span className="hint">—</span>
        )}
      </td>
      <td>
        {entry.stepId !== null ? (
          <button type="button" className="link rn-id" title={entry.stepId} onClick={() => onSelect(entry.stepId as string)}>
            {clip(entry.stepId, 40)}
          </button>
        ) : (
          '—'
        )}
      </td>
      <td>{entry.attempt ?? '—'}</td>
      <td>{live ? '—' : entry.actor ? clip(entry.actor, 40) : '—'}</td>
      <td>
        {live ? (
          '—'
        ) : entry.traceId ? (
          <>
            <code>{clip(entry.traceId, 64)}</code> <CopyButton text={entry.traceId} what="trace id" />
            {entry.spanId !== null ? (
              <>
                <br />
                <span className="hint">attempt span</span> <code>{entry.spanId}</code> <CopyButton text={entry.spanId} what="span id" />
              </>
            ) : null}
          </>
        ) : (
          'no trace'
        )}
      </td>
      <td>
        {live ? (
          '—'
        ) : digest ? (
          <>
            <span>payload (redacted) {digest.short}</span> <CopyButton text={digest.full} what="payload digest" />
            <PayloadInspector runId={runId} selector={{ source: 'journal', sequence: entry.seq }} compact />
          </>
        ) : (
          <span className="hint">no payload</span>
        )}
      </td>
    </tr>
  );
}

/**
 * How far the spans of this run are behind its journal. The journal below is whole whatever this says:
 * a run whose trace is missing steps is still read to the last event.
 */
function TelemetryNote({ runId }: { runId: string }) {
  const { ep } = useApp();
  const telemetry = useQuery({
    queryKey: ['run', runId, 'telemetry'],
    queryFn: ({ signal }) => ep.telemetry(runId, signal),
    // A platform that does not know the endpoint, or a person who may not read it, is not an error to show.
    retry: false,
    refetchInterval: (query) => (query.state.data?.state === 'CATCHING_UP' || query.state.data?.state === 'BEHIND' ? 10_000 : false),
  });
  const data = telemetry.data;
  if (data === undefined) return null;
  const behind = Math.max(1, Math.round(data.lagMs / 1000));
  if (data.state === 'BEHIND') {
    return (
      <Banner tone="warn" title="The trace of this run is behind its journal.">
        Its spans reach the trace backend about {behind} seconds late: steps may be missing from the trace. Everything in the journal below is complete.
      </Banner>
    );
  }
  if (data.state === 'CATCHING_UP') {
    return <p className="hint">The trace is catching up with the journal ({behind} seconds behind).</p>;
  }
  if (data.state === 'NO_TRACE') {
    return <p className="hint">This run belongs to no trace: nothing is exported for it.</p>;
  }
  return null;
}
