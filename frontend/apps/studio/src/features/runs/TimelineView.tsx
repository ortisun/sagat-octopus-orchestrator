import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { useApp } from '../../app/context';
import { Badge, Banner, Button, Checkbox, Code, ColumnHeader, Empty, ErrorState, Hint, Loading, WindowedTable, Row, Stack, TableBody, TableCell, TableHead, TableRow, TextInput } from '../../ui';
import type { Tone } from '../../ui';
import { eventLabel, eventTone } from '../../model/events';
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

const TONES: Record<string, { word: string; symbol: string; tone: Tone } | undefined> = {
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
    <Stack gap={3}>
      <Row gap={3} align="end" justify="between">
        <Row gap={3} align="end">
          <TextInput type="search" label="Step" width="16rem" value={filter.stepId} placeholder="a step id" autoComplete="off" spellCheck={false} onChange={(stepId) => onFilter({ ...filter, stepId })} />
          <Checkbox label="Only problems" checked={filter.problems} onChange={(problems) => onFilter({ ...filter, problems })} />
          <Checkbox label="Only decisions" checked={filter.decisions} onChange={(decisions) => onFilter({ ...filter, decisions })} />
          {selectedId !== undefined && filter.stepId !== selectedId ? <Button onClick={() => onFilter({ ...filter, stepId: selectedId })}>Only {clip(selectedId, 30)}</Button> : null}
        </Row>
        <Row>
          <Button disabled={shown.length === 0} onClick={() => virtualizer.scrollToIndex(shown.length - 1)}>
            Go to the latest event
          </Button>
          <Button onClick={() => journal.reload()} title="Reads the journal again from the start">
            Read the journal again
          </Button>
        </Row>
      </Row>

      <TelemetryNote runId={runId} />

      <Hint>
        {snapshot.complete ? `All ${total} journal events are read.` : `${total} journal events read so far.`}
        {filtered ? ` ${shown.length} match the filters.` : ''}
        {snapshot.hidden > 0 ? ` ${snapshot.hidden} newer events arrived: read the rest of the journal to see them.` : ''}
        {snapshot.droppedLive ? ' Some live events were let go to save memory: read the journal again to see them.' : ''}
        {hasLive ? ' Events marked live show who did them and the trace when the journal is read again.' : ''}
      </Hint>
      {snapshot.error !== undefined ? <ErrorState error={snapshot.error} what="Reading the journal" onRetry={() => void journal.loadMore()} /> : null}

      {shown.length === 0 ? (
        snapshot.loading ? (
          <Loading>Reading the journal…</Loading>
        ) : (
          <Empty title={!snapshot.started ? 'The journal is not read yet.' : filtered ? 'No event matches the filters.' : 'The journal has no events yet.'} />
        )
      ) : (
        <ScrollRegion label="Journal events" scroller={scroller}>
          <WindowedTable className="rn-fixed rn-journal" label="Journal events, oldest first" rowCount={shown.length + 1}>
            <TableHead>
              <TableRow aria-rowindex={1}>
                <ColumnHeader scope="col">Seq</ColumnHeader>
                <ColumnHeader scope="col">Time</ColumnHeader>
                <ColumnHeader scope="col">Event</ColumnHeader>
                <ColumnHeader scope="col">Cause</ColumnHeader>
                <ColumnHeader scope="col">Step</ColumnHeader>
                <ColumnHeader scope="col">Attempt</ColumnHeader>
                <ColumnHeader scope="col">Actor</ColumnHeader>
                <ColumnHeader scope="col">Trace</ColumnHeader>
                <ColumnHeader scope="col">Payload</ColumnHeader>
              </TableRow>
            </TableHead>
            <TableBody>
              {padTop > 0 ? (
                <TableRow aria-hidden="true" className="rn-pad">
                  <TableCell colSpan={COLUMNS} style={{ height: padTop }} />
                </TableRow>
              ) : null}
              {items.map((item) => {
                const entry = shown[item.index];
                return entry === undefined ? null : <EntryRow key={entry.seq} runId={runId} entry={entry} index={item.index} measure={measure} selected={entry.stepId !== null && entry.stepId === selectedId} onSelect={onSelect} />;
              })}
              {padBottom > 0 ? (
                <TableRow aria-hidden="true" className="rn-pad">
                  <TableCell colSpan={COLUMNS} style={{ height: padBottom }} />
                </TableRow>
              ) : null}
            </TableBody>
          </WindowedTable>
        </ScrollRegion>
      )}

      {!snapshot.complete ? (
        <div>
          <Button disabled={snapshot.loading} onClick={() => void journal.loadMore()}>
            {snapshot.loading ? 'Reading…' : 'Load more events'}
          </Button>
        </div>
      ) : null}
    </Stack>
  );
}

function EntryRow({ runId, entry, index, measure, selected, onSelect }: { runId: string; entry: Entry; index: number; measure: (element: Element | null) => void; selected: boolean; onSelect: (id: string) => void }) {
  const tone = TONES[eventTone(entry.eventType)];
  const digest = digestView(entry.payloadRef);
  const live = entry.detail === 'live';
  return (
    <TableRow ref={measure} data-index={index} aria-rowindex={index + 2} data-tone={tone?.tone} data-selected={selected ? 'true' : undefined}>
      <TableCell>{entry.seq}</TableCell>
      <TableCell>
        <Moment at={entry.occurredAt} />
      </TableCell>
      <TableCell>
        {eventLabel(entry.eventType)}
        {tone ? (
          <>
            {' '}
            <Badge tone={tone.tone}>
              <span aria-hidden="true">{tone.symbol}</span> {tone.word}
            </Badge>
          </>
        ) : null}
        <br />
        <span className="rn-quiet">
          <Code>{clip(entry.eventType, 40)}</Code>
          {live ? ' (live)' : null}
        </span>
      </TableCell>
      <TableCell>
        {entry.cause !== null ? (
          <>
            <Code>{clip(entry.cause.code, 40)}</Code>
            <br />
            <span className="rn-quiet">{CAUSE_WORDS[entry.cause.kind] ?? entry.cause.kind}</span>
          </>
        ) : live ? (
          '—'
        ) : (
          <span className="rn-quiet">—</span>
        )}
      </TableCell>
      <TableCell>
        {entry.stepId !== null ? (
          <Button variant="ghost" size="sm" className="rn-id" {...(entry.stepId.length > 40 ? { title: entry.stepId } : {})} onClick={() => onSelect(entry.stepId as string)}>
            {clip(entry.stepId, 40)}
          </Button>
        ) : (
          '—'
        )}
      </TableCell>
      <TableCell>{entry.attempt ?? '—'}</TableCell>
      <TableCell>{live ? '—' : entry.actor ? clip(entry.actor, 40) : '—'}</TableCell>
      <TableCell>
        {live ? (
          '—'
        ) : entry.traceId ? (
          <>
            <Code>{clip(entry.traceId, 64)}</Code> <CopyButton text={entry.traceId} what="trace id" />
            {entry.spanId !== null ? (
              <>
                <br />
                <span className="rn-quiet">attempt span</span> <Code>{entry.spanId}</Code> <CopyButton text={entry.spanId} what="span id" />
              </>
            ) : null}
          </>
        ) : (
          'no trace'
        )}
      </TableCell>
      <TableCell>
        {live ? (
          '—'
        ) : digest ? (
          <>
            <span>payload (redacted) {digest.short}</span> <CopyButton text={digest.full} what="payload digest" />
            <PayloadInspector runId={runId} selector={{ source: 'journal', sequence: entry.seq }} compact />
          </>
        ) : (
          <span className="rn-quiet">no payload</span>
        )}
      </TableCell>
    </TableRow>
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
    return <Hint>The trace is catching up with the journal ({behind} seconds behind).</Hint>;
  }
  if (data.state === 'NO_TRACE') {
    return <Hint>This run belongs to no trace: nothing is exported for it.</Hint>;
  }
  return null;
}
