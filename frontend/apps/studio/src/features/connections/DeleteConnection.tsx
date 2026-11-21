import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../app/context';
import { Actions, Button, ErrorState, Text, useAnnounce } from '../../ui';
import { connectionsApi } from './api';
import type { Connection } from './api';

export function DeleteConnection({ namespace, connection, onClose }: { namespace: string; connection: Connection; onClose: () => void }) {
  const { api, queries, session } = useApp(); const announce = useAnnounce();
  const [requestId] = useState(() => crypto.randomUUID());
  const [busy, setBusy] = useState(false); const [error, setError] = useState<unknown>();
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  async function remove() {
    if (busy) return; setBusy(true); setError(undefined);
    const token = session.token(); const live = () => mounted.current && session.token() === token;
    try {
      await connectionsApi(api, namespace).delete(connection, requestId);
      if (!live()) return;
      await queries.invalidateQueries({ queryKey: ['connections', namespace] });
      if (live()) { announce(`Connection ${connection.name} deleted.`); onClose(); }
    } catch (problem) { if (live()) setError(problem); }
    finally { if (live()) setBusy(false); }
  }
  return <>
    <Text>Delete {connection.name} at revision {connection.revision}? This prevents subsequent use. Existing database effects are not undone. Its history is retained.</Text>
    {error ? <ErrorState error={error} what="Deleting the connection" /> : null}
    <Actions><Button disabled={busy} onClick={onClose}>Cancel</Button><Button variant="danger" disabled={busy} onClick={() => { void remove(); }}>{busy ? 'Deleting…' : 'Delete connection'}</Button></Actions>
  </>;
}
