import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import type { OperationsApi } from './api';
import { Moment } from '../runs/Now';
import { groupWorkerIdentities } from './model';
import { WorkerActionDialog } from './WorkerActionDialog';

type Decision = { action: 'drain' | 'resume'; subject: string };

export function WorkersPanel({ api, canManage, namespace, namespaceId, sessionKey }: {
  api: OperationsApi;
  canManage: boolean;
  namespace: string;
  namespaceId: string;
  sessionKey: string;
}) {
  const key = ['operations', sessionKey, namespaceId, 'workers'];
  const list = useQuery({ queryKey: key, queryFn: ({ signal }) => api.workers(signal), refetchInterval: 15_000 });
  const [decision, setDecision] = useState<Decision>();
  const identities = list.data ? groupWorkerIdentities(list.data.items, list.data.drains) : [];

  return (
    <section className="operations-panel" aria-labelledby="operations-workers-title">
      <div className="operations-panel-heading">
        <div>
          <h2 id="operations-workers-title">Worker identities</h2>
          <p>Sessions registered for this namespace and identity drains that persist across restarts.</p>
        </div>
      </div>
      <p className="hint">Registration and session expiry are not health checks. The API lists up to the 200 newest sessions.</p>
      {!canManage ? <p className="hint">Draining or resuming an identity requires worker:manage permission.</p> : null}
      {list.isPending ? <p role="status">Loading worker identities…</p> : null}
      {list.isError ? <ErrorState error={list.error} what="Loading worker identities" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && identities.length === 0 ? <p className="empty">No worker identities are currently listed.</p> : null}
      {identities.length > 0 ? (
        <div className="operations-worker-list">
          {identities.map((identity) => (
            <section className="operations-worker" role="group" aria-label={`Worker identity ${identity.subject}`} key={identity.subject}>
              <header>
                <div>
                  <h3><code>{identity.subject}</code></h3>
                  <span className="badge" data-tone={identity.draining ? 'waiting' : 'idle'}>{identity.draining ? 'Drained' : 'Accepting claims'}</span>
                </div>
                {canManage ? (
                  <Button
                    variant={identity.draining ? 'primary' : 'danger'}
                    onClick={() => setDecision({ action: identity.draining ? 'resume' : 'drain', subject: identity.subject })}
                    aria-label={`${identity.draining ? 'Resume' : 'Drain'} worker identity ${identity.subject}`}
                  >
                    {identity.draining ? 'Resume identity' : 'Drain identity'}
                  </Button>
                ) : null}
              </header>
              {identity.drain ? (
                <dl className="operations-facts">
                  <dt>Drain reason</dt><dd>{identity.drain.reason}</dd>
                  <dt>Drained by</dt><dd><code>{identity.drain.drainedBy}</code></dd>
                  <dt>Drained at</dt><dd><Moment at={identity.drain.drainedAt} empty="Not recorded" /></dd>
                </dl>
              ) : null}
              {identity.sessions.length === 0 ? <p className="empty">No sessions are currently listed for this identity.</p> : (
                // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the session table
                <div className="operations-table-scroll" role="region" aria-label={`Sessions for ${identity.subject}`} tabIndex={0}>
                  <table className="data">
                    <caption className="visually-hidden">Registered sessions for {identity.subject}</caption>
                    <thead><tr><th scope="col">Name</th><th scope="col">Capabilities</th><th scope="col">In flight</th><th scope="col">Session expiry</th></tr></thead>
                    <tbody>
                      {identity.sessions.map((session) => (
                        <tr key={session.workerId}>
                          <th scope="row">{session.name}</th>
                          <td>{[...session.activities, ...session.languages, ...session.pools].join(', ') || 'None listed'}</td>
                          <td>{session.inFlight} / {session.maxInflight}</td>
                          <td><Moment at={session.expiresAt} empty="Not recorded" /></td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          ))}
        </div>
      ) : null}
      <WorkerActionDialog
        action={decision?.action}
        subject={decision?.subject}
        api={api}
        namespace={namespace}
        onClose={() => setDecision(undefined)}
        onChanged={() => void list.refetch()}
      />
    </section>
  );
}
