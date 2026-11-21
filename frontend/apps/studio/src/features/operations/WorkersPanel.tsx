import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Badge, Button, Code, ColumnHeader, Empty, ErrorState, Facts, Hint, Loading, Panel, Stack, Table, RowHeader, TableBody, TableCell, TableHead, TableRow } from '../../ui';
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
    <Panel title="Worker identities" labelledBy="operations-workers-title" hint="Sessions registered for this namespace and identity drains that persist across restarts.">
      <Hint>Registration and session expiry are not health checks. The API lists up to the 200 newest sessions.</Hint>
      {!canManage ? <Hint>Draining or resuming an identity requires worker:manage permission.</Hint> : null}
      {list.isPending ? <Loading>Loading worker identities…</Loading> : null}
      {list.isError ? <ErrorState error={list.error} what="Loading worker identities" onRetry={() => void list.refetch()} /> : null}
      {list.isSuccess && identities.length === 0 ? <Empty title="No worker identities are currently listed." /> : null}
      {identities.length > 0 ? (
        <Stack gap={3}>
          {identities.map((identity) => (
            <section className="operations-item" role="group" aria-label={`Worker identity ${identity.subject}`} key={identity.subject}>
              <Panel
                title={<Code>{identity.subject}</Code>}
                level={3}
                actions={
                  <>
                    <Badge tone={identity.draining ? 'waiting' : 'idle'}>{identity.draining ? 'Drained' : 'Accepting claims'}</Badge>
                    {canManage ? (
                      <Button
                        variant={identity.draining ? 'primary' : 'danger'}
                        onClick={() => setDecision({ action: identity.draining ? 'resume' : 'drain', subject: identity.subject })}
                        aria-label={`${identity.draining ? 'Resume' : 'Drain'} worker identity ${identity.subject}`}
                      >
                        {identity.draining ? 'Resume identity' : 'Drain identity'}
                      </Button>
                    ) : null}
                  </>
                }
              >
                {identity.drain ? (
                  <Facts
                    items={[
                      { term: 'Drain reason', value: identity.drain.reason },
                      { term: 'Drained by', value: <Code>{identity.drain.drainedBy}</Code> },
                      { term: 'Drained at', value: <Moment at={identity.drain.drainedAt} empty="Not recorded" /> },
                    ]}
                  />
                ) : null}
                {identity.sessions.length === 0 ? <Empty title="No sessions are currently listed for this identity." headingLevel={4} /> : (
                  // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- keyboard users must be able to scroll the session table
                  <div className="operations-table-scroll" role="region" aria-label={`Sessions for ${identity.subject}`} tabIndex={0}>
                    <Table label={`Registered sessions for ${identity.subject}`}>
                      <TableHead><TableRow><ColumnHeader>Name</ColumnHeader><ColumnHeader>Capabilities</ColumnHeader><ColumnHeader>In flight</ColumnHeader><ColumnHeader>Session expiry</ColumnHeader></TableRow></TableHead>
                      <TableBody>
                        {identity.sessions.map((session) => (
                          <TableRow key={session.workerId}>
                            <RowHeader>{session.name}</RowHeader>
                            <TableCell>{[...session.activities, ...session.languages, ...session.pools].join(', ') || 'None listed'}</TableCell>
                            <TableCell>{session.inFlight} / {session.maxInflight}</TableCell>
                            <TableCell><Moment at={session.expiresAt} empty="Not recorded" /></TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                )}
              </Panel>
            </section>
          ))}
        </Stack>
      ) : null}
      <WorkerActionDialog
        action={decision?.action}
        subject={decision?.subject}
        api={api}
        namespace={namespace}
        onClose={() => setDecision(undefined)}
        onChanged={() => void list.refetch()}
      />
    </Panel>
  );
}
