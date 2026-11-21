import { useEffect, useId, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import type { SessionSnapshot } from '../../auth/session';
import { stringify } from 'lossless-json';
import { Button, ColumnHeader, ErrorState, Heading, Hint, Loading, Row, Stack, Table, TableBody, TableCell, TableHead, TableRow, Text } from '../../ui';
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
  return (
    <Stack as="section" gap={3} aria-label="Database result">
      <Heading level={3}>Database result</Heading>
      <Hint>Reading rows is permission checked and audited. Close this view to clear its contents.</Hint>
      {namespaces.error ? <ErrorState error={namespaces.error} what="Loading the namespace" /> : null}
      <Row>
        <Button disabled={!namespace || state.phase === 'loading'} aria-controls={id} onClick={() => void load()}>Read database result</Button>
        {state.phase !== 'idle' ? <Button onClick={close}>Close database result</Button> : null}
      </Row>
      <div id={id} aria-live="polite">{state.phase === 'loading' ? <Loading>Reading the complete retained result…</Loading> : state.phase === 'failed' ? <ErrorState error={state.error} what="Reading the database result" /> : state.phase === 'loaded' ? <ResultTable key={`${scope}:${state.result.affectedRows}`} result={state.result} /> : null}</div>
    </Stack>
  );
}

const ROWS = 25, COLUMNS = 12, CELL_CHARS = 2000;
function Cell({ cell }: { cell: DatabaseResult['rows'][number][number] }) {
  const text = cell.type === 'null' ? 'NULL' : typeof cell.value === 'string' ? cell.value : (stringify(cell.value) ?? 'null');
  return <div className="database-result-cell"><span>{text.slice(0, CELL_CHARS)}</span>{text.length > CELL_CHARS ? <small>… (cell preview: first {CELL_CHARS} characters)</small> : null}{cell.type === 'binary' ? <small> (base64)</small> : null}</div>;
}
function ResultTable({ result }: { result: DatabaseResult }) {
  const [row, setRow] = useState(0); const [column, setColumn] = useState(0);
  return (
    <Stack gap={3}>
      <Text>{result.rows.length} rows · {result.columns.length} columns · {result.affectedRows} affected rows</Text>
      {result.rows.length > 0 ? <>
        <Row><Button disabled={row === 0} onClick={() => setRow(row - ROWS)}>Previous rows</Button><span>Rows {row + 1}–{Math.min(row + ROWS, result.rows.length)}</span><Button disabled={row + ROWS >= result.rows.length} onClick={() => setRow(row + ROWS)}>Next rows</Button></Row>
        {result.columns.length > COLUMNS ? <Row><Button disabled={column === 0} onClick={() => setColumn(column - COLUMNS)}>Previous columns</Button><span>Columns {column + 1}–{Math.min(column + COLUMNS, result.columns.length)}</span><Button disabled={column + COLUMNS >= result.columns.length} onClick={() => setColumn(column + COLUMNS)}>Next columns</Button></Row> : null}
        {/* eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard access to the scrollable result table */}
        <div className="database-result-scroll" role="region" aria-label="Database result rows" tabIndex={0}>
          <Table label="Retained database result; typed cells in column order">
            <TableHead><TableRow>{result.columns.slice(column, column + COLUMNS).map((c, i) => <ColumnHeader key={column + i}>{c.name}<br /><small>{c.type}</small></ColumnHeader>)}</TableRow></TableHead>
            <TableBody>{result.rows.slice(row, row + ROWS).map((r, i) => <TableRow key={row + i}>{r.slice(column, column + COLUMNS).map((cell, j) => <TableCell key={column + j}><Cell cell={cell} /></TableCell>)}</TableRow>)}</TableBody>
          </Table>
        </div>
      </> : <Text>No rows returned.</Text>}
    </Stack>
  );
}
