import { useQuery } from '@tanstack/react-query';
import { useApp, useCan, useNamespace } from '../../app/context';
import { databaseIssue, object } from '../../model/database';
import { parseAuthoringJson } from '../../model/safeJson';
import { setField } from '../../model/edit';
import type { IrNode } from '../../model/ir';
import { Banner, Disclosure, ErrorState, Hint, Link, Select, Stack, TextArea, TextInput } from '../../ui';
import { connectionsApi } from '../connections/api';
import type { Change } from './fields';

const scalar = (value: unknown): string => typeof value === 'string' || typeof value === 'number' ? String(value) : '';

/** Stores incomplete values in the draft too: a failed JSON edit cannot run an older binding. */
export function DatabaseFields({ node, disabled, change }: { node: IrNode; disabled: boolean; change: Change }) {
  const { ep, api } = useApp();
  const slug = useNamespace();
  const canWrite = useCan('database:write');
  const canRead = useCan('namespace:read');
  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal), enabled: canRead });
  const namespace = namespaces.data?.items.find(n => n.slug === slug)?.id;
  const connections = useQuery({ queryKey: ['connections', namespace], enabled: canRead && namespace !== undefined, queryFn: ({ signal }) => connectionsApi(api, namespace as string).list(signal) });
  const ref = object(node['connectionRef']);
  const name = typeof ref['name'] === 'string' ? ref['name'] : '';
  const current = connections.data?.items.find(c => c.name === name);
  const set = (field: string, value: unknown) => change(doc => setField(doc, node.id, field, value), `database:${node.id}:${field}`);
  const parameters = node['parameters'] ?? [];
  const limits = object(node['limits']);
  const issue = databaseIssue(node);
  const connectionOptions = [
    ...(name && !current ? [{ value: name, label: `${name} (unavailable)` }] : []),
    ...(connections.data?.items ?? []).map(c => ({ value: c.name, label: `${c.name} · revision ${c.revision}${c.status === 'ACTIVE' ? '' : ' · paused'}`, disabled: c.status !== 'ACTIVE' })),
  ];
  return <Stack gap={3}>
    {namespaces.error || connections.error ? <ErrorState error={namespaces.error ?? connections.error} what="Loading connections" onRetry={() => { void namespaces.refetch(); void connections.refetch(); }} /> : null}
    <Select label="Connection" hint="The run retains the chosen revision. Credentials are resolved when the step executes." value={name} placeholder="Choose a connection" disabled={disabled || !connections.data} options={connectionOptions} onChange={value => set('connectionRef', { name: value })} />
    <Hint><Link to="/connections">Manage connections and credentials</Link></Hint>
    {current?.status === 'PAUSED' ? <Banner tone="warn">This connection is paused. Activate it before starting a run.</Banner> : null}
    <TextInput label="Connection revision" hint="Empty: resolve the active revision when the run starts. Enter a number to pin a retained revision." value={scalar(ref['revision'])} numeric autoComplete="off" disabled={disabled} onChange={value => {
      set('connectionRef', value === '' ? { name } : { name, revision: /^\d+$/.test(value) ? Number(value) : value });
    }} />
    <Select label="Operation" value={scalar(node['operation']) || 'query'} disabled={disabled} options={[{ value: 'query', label: 'Query · read only' }, { value: 'execute', label: 'Execute · insert, update or delete', disabled: !canWrite }]} onChange={value => set('operation', value)} />
    {node['operation'] === 'execute' ? <Banner tone="warn">This operation changes data. An uncertain outcome requires manual review; SQL is not automatically retried.{!canWrite ? ' Your role cannot execute writes.' : ''}</Banner> : null}
    <TextArea label="SQL statement" hint="One statement. Use $1, $2, … for values; bind them below. Never put passwords in SQL or parameters." rows={6} spellCheck={false} mono value={typeof node['statement'] === 'string' ? node['statement'] : ''} disabled={disabled} maxLength={65_536} onChange={value => set('statement', value.slice(0, 65_536))} />
    <TextArea label="Typed parameters (JSON)" hint={'An ordered array, e.g. [{"literal":{"type":"integer","value":"42"}},{"type":"text","expression":"input.name"}]. Integers and decimals use strings. Earlier inline results can be read with steps.first.output.rows[0][0].value.'} error={!Array.isArray(parameters) ? 'Enter a JSON array of typed bindings. Use strings for exact integers and decimals.' : undefined} rows={7} spellCheck={false} mono value={typeof parameters === 'string' ? parameters : JSON.stringify(parameters, null, 2)} disabled={disabled} maxLength={262_144} onChange={typed => {
      const text = typed.slice(0, 262_144);
      let value: unknown = text;
      try { value = parseAuthoringJson(text); } catch { /* Keep incomplete text; platform validation refuses it. */ }
      set('parameters', value === null ? 'null' : value);
    }} />
    <Disclosure summary="Result and timeout limits">
      <Stack gap={3}>
        <Hint>Empty uses connection defaults. Namespace and platform limits also apply.</Hint>
        {([['maxRows', 'Maximum rows', 10_000], ['maxResultBytes', 'Maximum result bytes', 4_194_304], ['timeoutMs', 'Query timeout (ms)', 30_000], ['connectTimeoutMs', 'Connect timeout (ms)', 10_000]] as const).map(([field, label, max]) => <TextInput key={field} label={label} hint={`1–${max}`} numeric autoComplete="off" disabled={disabled} value={scalar(limits[field])} onChange={value => {
          const next = { ...limits };
          if (value === '') delete next[field]; else next[field] = /^\d+$/.test(value) ? Number(value) : value;
          set('limits', Object.keys(next).length ? next : undefined);
        }} />)}
      </Stack>
    </Disclosure>
    {issue ? <Hint role="status">{issue}</Hint> : null}
  </Stack>;
}
