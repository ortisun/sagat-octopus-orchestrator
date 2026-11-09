import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from 'react-router';
import { path } from '../../api/client';
import { isTerminal } from '../../api/follow';
import type { components } from '../../api/schema';
import type { Run } from '../../api/types';
import { useApp, useCan } from '../../app/context';
import { Button } from '../../ui/Button';
import { Banner } from '../../ui/Banner';
import { ErrorState } from '../../ui/ErrorState';
import { useDatabaseCommand } from '../database/useDatabaseCommand';
import { icebergOf } from './api';
import type { Connection } from './api';

type Probe = components['schemas']['DatabaseConnectionProbe'];
type Check = NonNullable<Probe['checks']>[number];
const CHECK_NAMES: Record<Check['check'], string> = { CATALOG_AUTH: 'Catalog authentication', WAREHOUSE: 'Warehouse', STORAGE_ACCESS: 'Storage access' };
/** An outcome by text and by shape, never by colour alone. A check that did not run is not confirmed and is never shown as passed. */
const OUTCOMES: Record<Check['outcome'], string> = { PASSED: '✓ Passed', FAILED: '✗ Failed', SKIPPED: '– Skipped, not confirmed' };
/**
 * What the Studio may say about an Iceberg connection from its latest probe: it is confirmed only by a probe that ended, tested the revision
 * the connection has now, and found every check passed. A probe with no checks (a platform fault, or a result that can no longer be read) never
 * confirms anything, and a probe that has not ended says nothing yet.
 */
function verdict(evidence: Probe, current: number): string | null {
  if (!isTerminal(evidence.run.status)) return null;
  if (evidence.run.status !== 'SUCCEEDED') return 'Not confirmed: the probe could not run.';
  if (!evidence.checks?.length) return 'The result of this probe cannot be read, so the connection is not confirmed.';
  if (evidence.checks.some(check => check.outcome !== 'PASSED')) return 'Not confirmed: at least one check did not pass.';
  return evidence.revision === current ? `Confirmed: every check passed for revision ${evidence.revision}.` : 'Not confirmed: the current revision has not been tested.';
}
export function ConnectionProbe({ namespace, connection }: { namespace: string; connection: Connection }) {
  const { api, session } = useApp(); const queries = useQueryClient();
  const use = useCan('database:use'); const adhoc = useCan('database:adhoc'); const start = useCan('run:start'); const read = useCan('run:read');
  const url = path`/namespaces/${namespace}/connections/${connection.name}/probe`;
  const key = ['connection-probe', namespace, connection.name];
  const latest = useQuery({ queryKey: key, enabled: read, queryFn: ({ signal }) => api.get<Probe | null>(url, { signal }), refetchInterval: q => q.state.data && !isTerminal(q.state.data.run.status) ? 1000 : false });
  const command = useDatabaseCommand<{ revision: number }>(async (intent, signal) => {
    const run = await api.post<Run>(url, { ...intent, signal });
    await queries.invalidateQueries({ queryKey: key });
    return run;
  }, session, `probe:${namespace}:${connection.name}`);
  const evidence = latest.data;
  const lake = icebergOf(connection) !== undefined;
  const summary = lake && evidence ? verdict(evidence, connection.revision) : null;
  return <div className="database-probe">
    {lake
      ? <p>Checks the catalog, the warehouse and the storage of connection <strong>{connection.name}</strong>, revision {command.body?.revision ?? connection.revision}, with its credentials. Nothing is written.</p>
      : <p>Runs a fixed read-only check through the managed executor using connection <strong>{connection.name}</strong>, revision {command.body?.revision ?? connection.revision}.</p>}
    {connection.status !== 'ACTIVE' ? <Banner tone="warn">Activate the connection before starting a probe.</Banner> : null}
    {latest.isPending && read ? <p role="status">Loading probe history…</p> : null}
    {latest.error ? <ErrorState error={latest.error} what="Loading the latest probe" onRetry={() => void latest.refetch()} /> : null}
    {evidence ? <section aria-label="Latest probe"><h3>Latest probe</h3><p>Revision {evidence.revision} · <strong>{evidence.run.status}</strong></p>
      {evidence.revision !== connection.revision ? <p className="hint">This probe tested an older revision. The current revision has not been confirmed by this check.</p> : null}
      {evidence.failureCode ? <p>Failure: <code>{evidence.failureCode}</code></p> : null}
      {lake && evidence.checks?.length ? <ul aria-label="Checks of the probe" className="probe-checks">{evidence.checks.map(check => <li key={check.check}>{CHECK_NAMES[check.check]}: <strong>{OUTCOMES[check.outcome]}</strong>{check.code ? <> <code>{check.code}</code></> : null}</li>)}</ul> : null}
      {summary ? <p role="status">{summary}</p> : null}
      <Link to={path`/runs/${evidence.run.runId}`}>Open probe run</Link>
    </section> : latest.data === null ? <p>No probe has been recorded.</p> : null}
    {command.state.phase === 'failed' ? <><ErrorState error={command.state.error} what="Starting the probe" />
      <Banner tone="warn">{command.state.refused ? 'The request was refused.' : 'The acknowledgement was lost or could not be confirmed. Retrying checks the same request identity.'}</Banner>
      <Button onClick={() => void command.send()}>Retry same probe</Button>
      {command.state.refused ? <Button onClick={command.reset}>Dismiss refusal</Button> : null}
    </> : null}
    {command.state.phase === 'accepted' ? <p role="status">Probe accepted. <Button onClick={command.reset}>New probe</Button> <Link to={path`/runs/${command.state.run.runId}`}>Follow probe run</Link></p> : null}
    {use && adhoc && start ? <Button variant="primary" disabled={command.locked || connection.status !== 'ACTIVE'} onClick={() => void command.send({ revision: connection.revision })}>{command.state.phase === 'sending' ? 'Starting probe…' : 'Start probe'}</Button> : <p className="hint">Starting probes requires database use, ad hoc and run start permissions.</p>}
    {lake
      ? <p className="hint">A connection is confirmed only when every check passed, at the time tested; future availability can change. Saving a connection does not prove it works.</p>
      : <p className="hint">A successful probe confirms this revision at the time tested; future availability can change.</p>}
  </div>;
}
