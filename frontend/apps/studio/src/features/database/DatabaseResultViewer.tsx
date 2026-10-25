import { useEffect, useId, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import type { SessionSnapshot } from '../../auth/session';
import { stringify } from 'lossless-json';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { readDatabaseResult } from './readResult';
import type { DatabaseResult } from './readResult';
import '../../styles/database.css';

type State = { phase: 'idle' } | { phase: 'loading' } | { phase: 'loaded'; result: DatabaseResult } | { phase: 'failed'; error: unknown };
interface Scoped { scope: string; session: SessionSnapshot; state: State }
export function DatabaseResultViewer({ runId, nodeId }: { runId: string; nodeId: string }) {
  const { api, ep } = useApp(); const allowed = useCan('database:result:read'); const read = useCan('run:read');
  const slug = useNamespace(); const session = useSession();
  const namespaces = useQuery({ queryKey: ['namespaces'], enabled: allowed && read, queryFn: ({ signal }) => ep.namespaces(signal) });
  const namespace = namespaces.data?.items.find(n => n.slug === slug)?.id;
  const scope = `${session.info?.tenant}|${session.info?.subject}|${slug}|${namespace}|${runId}|${nodeId}`;
  const [stored, setStored] = useState<Scoped>({ scope, session, state: { phase: 'idle' } });
  const controller = useRef<AbortController | undefined>(undefined); const id = useId();
  const same = stored.scope === scope && stored.session === session;
  if (!same) setStored({ scope, session, state: { phase: 'idle' } });
  const state: State = same ? stored.state : { phase: 'idle' };
  useEffect(() => { controller.current?.abort(); controller.current = undefined; return () => controller.current?.abort(); }, [scope, session]);
  if (!allowed || !read) return null;
  const close = () => { controller.current?.abort(); controller.current = undefined; setStored({ scope, session, state: { phase: 'idle' } }); };
  const load = async () => {
    if (!namespace) return;
    controller.current?.abort(); const next = new AbortController(); controller.current = next;
    setStored({ scope, session, state: { phase: 'loading' } });
    try {
      const result = await readDatabaseResult(api, namespace, runId, nodeId, next.signal);
      if (!next.signal.aborted && controller.current === next) setStored({ scope, session, state: { phase: 'loaded', result } });
    } catch (error) { if (!next.signal.aborted && controller.current === next) setStored({ scope, session, state: { phase: 'failed', error } }); }
  };
  return <section className="database-result" aria-label="Database result"><h3>Database result</h3><p className="hint">Reading rows is permission checked and audited. Close this view to clear its contents.</p>
    {namespaces.error ? <ErrorState error={namespaces.error} what="Loading the namespace" /> : null}
    <div className="database-result-controls"><Button disabled={!namespace || state.phase === 'loading'} aria-controls={id} onClick={() => void load()}>Read database result</Button>{state.phase !== 'idle' ? <Button onClick={close}>Close database result</Button> : null}</div>
    <div id={id} aria-live="polite">{state.phase === 'loading' ? <p role="status">Reading the complete retained result…</p> : state.phase === 'failed' ? <ErrorState error={state.error} what="Reading the database result" /> : state.phase === 'loaded' ? <ResultTable key={`${scope}:${state.result.affectedRows}`} result={state.result} /> : null}</div>
  </section>;
}

const ROWS = 25, COLUMNS = 12, CELL_CHARS = 2000;
function Cell({ cell }: { cell: DatabaseResult['rows'][number][number] }) {
  const text = cell.type === 'null' ? 'NULL' : typeof cell.value === 'string' ? cell.value : (stringify(cell.value) ?? 'null');
  return <div className="database-result-cell"><span>{text.slice(0, CELL_CHARS)}</span>{text.length > CELL_CHARS ? <small>… (cell preview: first {CELL_CHARS} characters)</small> : null}{cell.type === 'binary' ? <small> (base64)</small> : null}</div>;
}
function ResultTable({ result }: { result: DatabaseResult }) {
  const [row, setRow] = useState(0); const [column, setColumn] = useState(0);
  return <div><p>{result.rows.length} rows · {result.columns.length} columns · {result.affectedRows} affected rows</p>
    {result.rows.length > 0 ? <>
      <div className="database-result-controls"><Button disabled={row === 0} onClick={() => setRow(row - ROWS)}>Previous rows</Button><span>Rows {row + 1}–{Math.min(row + ROWS, result.rows.length)}</span><Button disabled={row + ROWS >= result.rows.length} onClick={() => setRow(row + ROWS)}>Next rows</Button></div>
      {result.columns.length > COLUMNS ? <div className="database-result-controls"><Button disabled={column === 0} onClick={() => setColumn(column - COLUMNS)}>Previous columns</Button><span>Columns {column + 1}–{Math.min(column + COLUMNS, result.columns.length)}</span><Button disabled={column + COLUMNS >= result.columns.length} onClick={() => setColumn(column + COLUMNS)}>Next columns</Button></div> : null}
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard access to the scrollable result table */}
      <div className="database-result-scroll" role="region" aria-label="Database result rows" tabIndex={0}><table className="data"><caption className="visually-hidden">Retained database result; typed cells in column order</caption><thead><tr>{result.columns.slice(column, column + COLUMNS).map((c, i) => <th scope="col" key={column + i}>{c.name}<br /><small>{c.type}</small></th>)}</tr></thead><tbody>{result.rows.slice(row, row + ROWS).map((r, i) => <tr key={row + i}>{r.slice(column, column + COLUMNS).map((cell, j) => <td key={column + j}><Cell cell={cell} /></td>)}</tr>)}</tbody></table></div>
    </> : <p>No rows returned.</p>}
  </div>;
}
