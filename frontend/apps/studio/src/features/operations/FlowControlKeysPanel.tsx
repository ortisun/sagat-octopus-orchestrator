import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { isApiError } from '../../api/errors';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Moment } from '../runs/Now';
import { keyLabelWords, noNextSlotWords, poolWords, queuedWords, scopeWords } from './flowControlKeys';
import type { ConcurrencyKey, FlowControlKeysApi, KeyQuery, RateLimitKey } from './flowControlKeys';
import './flowControlKeys.css';

const PAGE_SIZE = 50;
const HIDDEN_NOTE = 'A hidden key label: you may not reveal personal data in this namespace, so the platform does not show it, and gives the instants of that key to the minute.';

/** An instant of a key; the platform cuts the instants of a hidden key to the minute, and the cell says so. */
function Instant({ at, hidden, empty }: { at: string | null | undefined; hidden: boolean; empty: string }) {
  if (at === null || at === undefined) return <>{empty}</>;
  return (
    <>
      <Moment at={at} />
      {hidden ? ' (to the minute)' : null}
    </>
  );
}

interface Column<T> {
  header: string;
  cell: (item: T) => ReactNode;
}

interface Listing<T> {
  title: string;
  /** The plural noun, for the buttons and the caption. */
  noun: string;
  empty: string;
  columns: readonly Column<T>[];
  read: (query: KeyQuery, signal: AbortSignal) => Promise<{ queuedRuns: number; items: T[]; nextCursor?: string | null }>;
  hidden: (item: T) => boolean;
}

/** One listing: a group filter, a table of a page or more, and "Load more" while the platform has a next page. */
function KeyListing<T extends { group: string }>({ listing, namespace, queryKey }: { listing: Listing<T>; namespace: string; queryKey: readonly unknown[] }) {
  const [draft, setDraft] = useState('');
  const [group, setGroup] = useState('');
  const headingId = useId();
  const list = useInfiniteQuery({
    queryKey: [...queryKey, group],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => listing.read({ limit: PAGE_SIZE, ...(group === '' ? {} : { group }), ...(pageParam ? { cursor: pageParam } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const items = list.data?.pages.flatMap((page) => page.items) ?? [];
  const queued = list.data?.pages[0]?.queuedRuns;
  // A platform without the listing answers its route with a 404 (the namespace itself was resolved by the page): the
  // listing is unavailable there, which is said, not raised as a failure (FR-007-010).
  const unavailable = list.isError && isApiError(list.error) && list.error.status === 404;
  const caption = `${listing.title} of ${namespace}${group === '' ? '' : `, group ${group}`}, oldest wait first`;

  return (
    <section className="operations-keys" aria-labelledby={headingId}>
      <div className="operations-panel-heading">
        <div>
          <h3 id={headingId}>{listing.title}</h3>
          {queued !== undefined ? <p>{queuedWords(queued)}</p> : null}
        </div>
        {unavailable ? null : (
          <form
            role="search"
            aria-label={`Filter ${listing.noun}`}
            className="operations-filter operations-key-filter"
            onSubmit={(event) => {
              event.preventDefault();
              setGroup(draft.trim());
            }}
          >
            <label>
              <span>Group</span>
              <input type="text" autoComplete="off" spellCheck={false} value={draft} onChange={(event) => setDraft(event.target.value)} />
            </label>
            <Button type="submit">Filter</Button>
          </form>
        )}
      </div>
      {list.isPending ? <p role="status">Loading {listing.noun}…</p> : null}
      {unavailable ? <p role="status">Unavailable: this platform does not list {listing.noun}.</p> : null}
      {list.isError && !unavailable ? <ErrorState error={list.error} what={`Loading ${listing.noun}`} onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && items.length === 0 ? <p className="empty">{listing.empty}</p> : null}
      {items.length > 0 ? (
        <>
          {items.some(listing.hidden) ? <p className="hint">{HIDDEN_NOTE}</p> : null}
          {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- a scrollable region must take focus (WCAG 2.1.1) */}
          <div className="operations-key-scroll" role="region" aria-label={`${listing.title}, scrollable`} tabIndex={0}>
            <table className="data">
              <caption>{caption}</caption>
              <thead>
                <tr>
                  <th scope="col">Group</th>
                  {listing.columns.map((column) => (
                    <th scope="col" key={column.header}>
                      {column.header}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {items.map((item, index) => (
                  <tr key={index}>
                    <th scope="row">
                      <code>{item.group}</code>
                    </th>
                    {listing.columns.map((column) => (
                      <td key={column.header}>{column.cell(item)}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      ) : null}
      {list.isFetchNextPageError ? <ErrorState error={list.error} what={`Loading more ${listing.noun}`} onRetry={() => void list.fetchNextPage()} /> : null}
      {list.hasNextPage ? (
        <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>
          {list.isFetchingNextPage ? 'Loading…' : `Load more ${listing.noun}`}
        </Button>
      ) : null}
    </section>
  );
}

const common = <T extends { scope: string; pool: string; keyLabel: string | null }>(): Column<T>[] => [
  { header: 'Applies to', cell: (key) => scopeWords(key.scope) },
  { header: 'Pool', cell: (key) => poolWords(key.pool) },
  { header: 'Key label', cell: (key) => (key.keyLabel === null ? keyLabelWords(null) : <code>{keyLabelWords(key.keyLabel)}</code>) },
];

/**
 * The live flow-control keys of a namespace (feature 022, FR-022-022 and FR-022-048): which queue is stuck, and for
 * how long. Both listings need `ops:read`, which the page checks before it renders this.
 */
export function FlowControlKeysPanel({ api, namespace, namespaceId, sessionKey }: { api: FlowControlKeysApi; namespace: string; namespaceId: string; sessionKey: string }) {
  const concurrency: Listing<ConcurrencyKey> = {
    title: 'Concurrency keys',
    noun: 'concurrency keys',
    empty: 'No concurrency key has a holder or a waiter in this namespace.',
    read: (query, signal) => api.concurrencyKeys(query, signal),
    hidden: (key) => key.keyLabel === null,
    columns: [
      ...common<ConcurrencyKey>(),
      { header: 'Holders', cell: (key) => key.holders },
      { header: 'Waiters', cell: (key) => key.waiters },
      { header: 'Oldest wait since', cell: (key) => <Instant at={key.oldestWaitSince} hidden={key.keyLabel === null} empty="Nobody waits" /> },
      { header: 'Oldest holder since', cell: (key) => <Instant at={key.oldestHolderSince} hidden={key.keyLabel === null} empty="Nobody holds" /> },
    ],
  };
  const rate: Listing<RateLimitKey> = {
    title: 'Rate-limit keys',
    noun: 'rate-limit keys',
    empty: 'No rate-limit key has a counted grant or a waiter in this namespace.',
    read: (query, signal) => api.rateLimitKeys(query, signal),
    hidden: (key) => key.keyLabel === null,
    columns: [
      ...common<RateLimitKey>(),
      { header: 'Grants in window', cell: (key) => key.grantsInWindow },
      { header: 'Unleased grants', cell: (key) => key.unleasedGrants },
      { header: 'Waiters', cell: (key) => key.waiters },
      { header: 'Oldest wait since', cell: (key) => <Instant at={key.oldestWaitSince} hidden={key.keyLabel === null} empty="Nobody waits" /> },
      { header: 'Next slot frees', cell: (key) => <Instant at={key.nextSlotAt} hidden={key.keyLabel === null} empty={noNextSlotWords(key)} /> },
    ],
  };
  const base = ['operations', sessionKey, namespaceId, 'flow-control-keys'] as const;
  return (
    <section className="operations-panel operations-keys-panel" aria-labelledby="operations-keys-title">
      <div className="operations-panel-heading">
        <div>
          <h2 id="operations-keys-title">Flow-control keys</h2>
          <p>The keys whose runs or steps hold or wait for a concurrency permit or a rate-limit slot, oldest wait first. A key is shown by its label, never by its value.</p>
        </div>
      </div>
      <KeyListing listing={concurrency} namespace={namespace} queryKey={[...base, 'concurrency']} />
      <KeyListing listing={rate} namespace={namespace} queryKey={[...base, 'rate']} />
    </section>
  );
}
