import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp } from '../../app/context';
import { Field } from '../../ui/Field';
import { ConnectionForm } from './ConnectionForm';
import { IcebergConnectionForm } from './IcebergConnectionForm';
import { connectionsApi, editableIcebergOf } from './api';
import type { Connection } from './api';

/**
 * Picks the form for a connection. An existing connection keeps its own kind. A new one is PostgreSQL unless the
 * deployment lists the Iceberg driver (`GET .../database-drivers`), in which case the administrator chooses: the Studio
 * never offers a kind the deployment does not run.
 */
export function ConnectionEditor({ namespace, development, connection, onClose }: { namespace: string; development: boolean; connection?: Connection; onClose: () => void }) {
  const { api } = useApp();
  const drivers = useQuery({ queryKey: ['database-drivers', namespace], enabled: !connection, retry: false, queryFn: ({ signal }) => connectionsApi(api, namespace).drivers(signal) });
  const [kind, setKind] = useState<'postgresql' | 'iceberg'>('postgresql');
  if (connection && editableIcebergOf(connection)) return <IcebergConnectionForm namespace={namespace} development={development} connection={connection} onClose={onClose} />;
  const lake = !connection && (drivers.data?.items.some(driver => driver.id === 'iceberg') ?? false);
  return <>
    {lake ? <Field label="Connection type"><select value={kind} onChange={e => setKind(e.target.value === 'iceberg' ? 'iceberg' : 'postgresql')}><option value="postgresql">PostgreSQL</option><option value="iceberg">Apache Iceberg (REST catalog, S3 storage)</option></select></Field> : null}
    {lake && kind === 'iceberg'
      ? <IcebergConnectionForm namespace={namespace} development={development} onClose={onClose} />
      : <ConnectionForm namespace={namespace} development={development} {...(connection ? { connection } : {})} onClose={onClose} />}
  </>;
}
