import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { isApiError } from '../../api/errors';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Moment } from '../runs/Now';
import { keyLabelWords, noNextSlotWords, poolWords, queuedWords, scopeWords } from './flowControlKeys';
import type { ConcurrencyKey, ConcurrencyKeyGroup, FlowControlKeysApi, KeyQuery, RateLimitKey, RateLimitKeyGroup } from './flowControlKeys';
import './flowControlKeys.css';

const PAGE_SIZE = 50;
const GROUPS_NOTE = 'You may not reveal personal data in this namespace, so the platform lists one row per group, summed over its keys, and no key. A large group can take more than one page: its row comes with the page that reads its last key.';

/** An instant of a key or a group, whole (decision D022-086). */
function Instant({ at, empty }: { at: string | null | undefined; empty: string }) {
  if (at === null || at === undefined) return <>{empty}</>;
  return <Moment at={at} />;
}

interface Column<T> {
  header: string;
  cell: (item: T) => ReactNode;
}

interface Listing<T, G> {
  title: string;
  /** The plural noun, for the buttons and the caption. */
  noun: string;
  empty: string;
  columns: readonly Column<T>[];
  /** The columns of a group row, for a caller the platform lists groups to (`view: GROUPS`). */
  groupColumns: readonly Column<G>[];
  read: (
    query: KeyQuery,
    signal: AbortSignal,
  ) => Promise<{ view?: 'KEYS' | 'GROUPS'; queuedRuns: number; items: T[]; groups?: G[]; nextCursor?: string | null }>;
}

/**
 * One listing: a group filter, a table of a page or more, and "Load more" while the platform has a next page. The
 * platform answers a caller who may reveal personal data with keys, any other with groups (decision D022-086); a
 * platform that does not say which lists keys.
 */
function KeyListing<T extends { group: string }, G extends { group: string }>({ listing, namespace, queryKey }: { listing: Listing<T, G>; namespace: string; queryKey: readonly unknown[] }) {
  const [draft, setDraft] = useState('');
  const [group, setGroup] = useState('');
  const headingId = useId();
  const list = useInfiniteQuery({
    queryKey: [...queryKey, group],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => listing.read({ limit: PAGE_SIZE, ...(group === '' ? {} : { group }), ...(pageParam ? { cursor: pageParam } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const groupView = list.data?.pages[0]?.view === 'GROUPS';
  const items = groupView ? [] : (list.data?.pages.flatMap((page) => page.items) ?? []);
  const groups = groupView ? (list.data?.pages.flatMap((page) => page.groups ?? []) ?? []) : [];
  const rows: readonly { group: string }[] = groupView ? groups : items;
  const queued = list.data?.pages[0]?.queuedRuns;
  // A platform without the listing answers its route with a 404 (the namespace itself was resolved by the page): the
  // listing is unavailable there, which is said, not raised as a failure (FR-007-010).
  const unavailable = list.isError && isApiError(list.error) && list.error.status === 404;
  const caption = groupView
    ? `${listing.title} of ${namespace}${group === '' ? '' : `, group ${group}`}, one row per group`
    : `${listing.title} of ${namespace}${group === '' ? '' : `, group ${group}`}, oldest wait first`;

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
      {list.isSuccess && rows.length === 0 ? <p className="empty">{listing.empty}</p> : null}
      {rows.length > 0 ? (
        <>
          {groupView ? <p className="hint">{GROUPS_NOTE}</p> : null}
          {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- a scrollable region must take focus (WCAG 2.1.1) */}
          <div className="operations-key-scroll" role="region" aria-label={`${listing.title}, scrollable`} tabIndex={0}>
            <table className="data">
              <caption>{caption}</caption>
              <thead>
                <tr>
                  <th scope="col">Group</th>
                  {(groupView ? listing.groupColumns : listing.columns).map((column) => (
                    <th scope="col" key={column.header}>
                      {column.header}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {groupView
                  ? groups.map((row, index) => (
                      <tr key={index}>
                        <th scope="row">
                          <code>{row.group}</code>
                        </th>
                        {listing.groupColumns.map((column) => (
                          <td key={column.header}>{column.cell(row)}</td>
                        ))}
                      </tr>
                    ))
                  : items.map((item, index) => (
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

const groupCommon = <G extends { scope: string; pool: string }>(): Column<G>[] => [
  { header: 'Applies to', cell: (row) => scopeWords(row.scope) },
  { header: 'Pool', cell: (row) => poolWords(row.pool) },
];

const common = <T extends { scope: string; pool: string; keyLabel: string | null }>(): Column<T>[] => [
  { header: 'Applies to', cell: (key) => scopeWords(key.scope) },
  { header: 'Pool', cell: (key) => poolWords(key.pool) },
  { header: 'Key label', cell: (key) => (typeof key.keyLabel === 'string' ? <code>{keyLabelWords(key.keyLabel)}</code> : keyLabelWords(key.keyLabel)) },
];

/**
 * The live flow-control keys of a namespace (feature 022, FR-022-022 and FR-022-048): which queue is stuck, and for
 * how long. Both listings need `ops:read`, which the page checks before it renders this.
 */
export function FlowControlKeysPanel({ api, namespace, namespaceId, sessionKey }: { api: FlowControlKeysApi; namespace: string; namespaceId: string; sessionKey: string }) {
  const concurrency: Listing<ConcurrencyKey, ConcurrencyKeyGroup> = {
    title: 'Concurrency keys',
    noun: 'concurrency keys',
    empty: 'No concurrency key has a holder or a waiter in this namespace.',
    read: (query, signal) => api.concurrencyKeys(query, signal),
    columns: [
      ...common<ConcurrencyKey>(),
      { header: 'Holders', cell: (key) => key.holders },
      { header: 'Waiters', cell: (key) => key.waiters },
      { header: 'Oldest wait since', cell: (key) => <Instant at={key.oldestWaitSince} empty="Nobody waits" /> },
      { header: 'Oldest holder since', cell: (key) => <Instant at={key.oldestHolderSince} empty="Nobody holds" /> },
    ],
    groupColumns: [
      ...groupCommon<ConcurrencyKeyGroup>(),
      { header: 'Holders', cell: (row) => row.holders },
      { header: 'Waiters', cell: (row) => row.waiters },
      { header: 'Oldest wait since', cell: (row) => <Instant at={row.oldestWaitSince} empty="Nobody waits" /> },
      { header: 'Oldest holder since', cell: (row) => <Instant at={row.oldestHolderSince} empty="Nobody holds" /> },
    ],
  };
  const rate: Listing<RateLimitKey, RateLimitKeyGroup> = {
    title: 'Rate-limit keys',
    noun: 'rate-limit keys',
    empty: 'No rate-limit key has a counted grant or a waiter in this namespace.',
    read: (query, signal) => api.rateLimitKeys(query, signal),
    columns: [
      ...common<RateLimitKey>(),
      { header: 'Grants in window', cell: (key) => key.grantsInWindow },
      { header: 'Unleased grants', cell: (key) => key.unleasedGrants },
      { header: 'Waiters', cell: (key) => key.waiters },
      { header: 'Oldest wait since', cell: (key) => <Instant at={key.oldestWaitSince} empty="Nobody waits" /> },
      { header: 'Next slot frees', cell: (key) => <Instant at={key.nextSlotAt} empty={noNextSlotWords(key)} /> },
    ],
    groupColumns: [
      ...groupCommon<RateLimitKeyGroup>(),
      { header: 'Grants in window', cell: (row) => row.grantsInWindow },
      { header: 'Unleased grants', cell: (row) => row.unleasedGrants },
      { header: 'Waiters', cell: (row) => row.waiters },
      { header: 'Oldest wait since', cell: (row) => <Instant at={row.oldestWaitSince} empty="Nobody waits" /> },
    ],
  };
  const base = ['operations', sessionKey, namespaceId, 'flow-control-keys'] as const;
  return (
    <section className="operations-panel operations-keys-panel" aria-labelledby="operations-keys-title">
      <div className="operations-panel-heading">
        <div>
          <h2 id="operations-keys-title">Flow-control keys</h2>
          <p>The keys whose runs or steps hold or wait for a concurrency permit or a rate-limit slot, oldest wait first. A key is shown by its label, never by its value; to a caller who may not reveal personal data, by its group only.</p>
        </div>
      </div>
      <KeyListing listing={concurrency} namespace={namespace} queryKey={[...base, 'concurrency']} />
      <KeyListing listing={rate} namespace={namespace} queryKey={[...base, 'rate']} />
    </section>
  );
}
