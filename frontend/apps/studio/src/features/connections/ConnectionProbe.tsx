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
import type { Connection } from './api';

type Probe = components['schemas']['DatabaseConnectionProbe'];
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
  return <div className="database-probe">
    <p>Runs a fixed read-only check through the managed executor using connection <strong>{connection.name}</strong>, revision {command.body?.revision ?? connection.revision}.</p>
    {connection.status !== 'ACTIVE' ? <Banner tone="warn">Activate the connection before starting a probe.</Banner> : null}
    {latest.isPending && read ? <p role="status">Loading probe history…</p> : null}
    {latest.error ? <ErrorState error={latest.error} what="Loading the latest probe" onRetry={() => void latest.refetch()} /> : null}
    {evidence ? <section aria-label="Latest probe"><h3>Latest probe</h3><p>Revision {evidence.revision} · <strong>{evidence.run.status}</strong></p>
      {evidence.revision !== connection.revision ? <p className="hint">This probe tested an older revision. The current revision has not been confirmed by this check.</p> : null}
      {evidence.failureCode ? <p>Failure: <code>{evidence.failureCode}</code></p> : null}
      <Link to={path`/runs/${evidence.run.runId}`}>Open probe run</Link>
    </section> : latest.data === null ? <p>No probe has been recorded.</p> : null}
    {command.state.phase === 'failed' ? <><ErrorState error={command.state.error} what="Starting the probe" />
      <Banner tone="warn">{command.state.refused ? 'The request was refused.' : 'The acknowledgement was lost or could not be confirmed. Retrying checks the same request identity.'}</Banner>
      <Button onClick={() => void command.send()}>Retry same probe</Button>
      {command.state.refused ? <Button onClick={command.reset}>Dismiss refusal</Button> : null}
    </> : null}
    {command.state.phase === 'accepted' ? <p role="status">Probe accepted. <Button onClick={command.reset}>New probe</Button> <Link to={path`/runs/${command.state.run.runId}`}>Follow probe run</Link></p> : null}
    {use && adhoc && start ? <Button variant="primary" disabled={command.locked || connection.status !== 'ACTIVE'} onClick={() => void command.send({ revision: connection.revision })}>{command.state.phase === 'sending' ? 'Starting probe…' : 'Start probe'}</Button> : <p className="hint">Starting probes requires database use, ad hoc and run start permissions.</p>}
    <p className="hint">A successful probe confirms this revision at the time tested; future availability can change.</p>
  </div>;
}
