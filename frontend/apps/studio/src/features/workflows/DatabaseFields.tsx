import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router';
import { useApp, useCan, useNamespace } from '../../app/context';
import { databaseIssue, object } from '../../model/database';
import { parseAuthoringJson } from '../../model/safeJson';
import { setField } from '../../model/edit';
import type { IrNode } from '../../model/ir';
import { Banner } from '../../ui/Banner';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
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
  return <div className="database-fields">
    {namespaces.error || connections.error ? <ErrorState error={namespaces.error ?? connections.error} what="Loading connections" onRetry={() => { void namespaces.refetch(); void connections.refetch(); }} /> : null}
    <Field label="Connection" hint="The run retains the chosen revision. Credentials are resolved when the step executes.">
      <select value={name} disabled={disabled || !connections.data} onChange={e => set('connectionRef', { name: e.target.value })}>
        <option value="">Choose a connection</option>
        {name && !current ? <option value={name}>{name} (unavailable)</option> : null}
        {connections.data?.items.map(c => <option key={c.id} value={c.name} disabled={c.status !== 'ACTIVE'}>{c.name} · revision {c.revision}{c.status === 'ACTIVE' ? '' : ' · paused'}</option>)}
      </select>
    </Field>
    <p className="hint"><Link to="/connections">Manage connections and credentials</Link></p>
    {current?.status === 'PAUSED' ? <Banner tone="warn">This connection is paused. Activate it before starting a run.</Banner> : null}
    <Field label="Connection revision" hint="Empty: resolve the active revision when the run starts. Enter a number to pin a retained revision.">
      <input value={scalar(ref['revision'])} inputMode="numeric" autoComplete="off" disabled={disabled} onChange={e => {
        const value = e.target.value;
        set('connectionRef', value === '' ? { name } : { name, revision: /^\d+$/.test(value) ? Number(value) : value });
      }} />
    </Field>
    <Field label="Operation">
      <select value={scalar(node['operation']) || 'query'} disabled={disabled} onChange={e => set('operation', e.target.value)}>
        <option value="query">Query · read only</option><option value="execute" disabled={!canWrite}>Execute · insert, update or delete</option>
      </select>
    </Field>
    {node['operation'] === 'execute' ? <Banner tone="warn">This operation changes data. An uncertain outcome requires manual review; SQL is not automatically retried.{!canWrite ? ' Your role cannot execute writes.' : ''}</Banner> : null}
    <Field label="SQL statement" hint="One statement. Use $1, $2, … for values; bind them below. Never put passwords in SQL or parameters.">
      <textarea rows={6} spellCheck={false} autoComplete="off" value={typeof node['statement'] === 'string' ? node['statement'] : ''} disabled={disabled} maxLength={65_536} onChange={e => set('statement', e.target.value)} />
    </Field>
    <Field label="Typed parameters (JSON)" hint={'An ordered array, e.g. [{"literal":{"type":"integer","value":"42"}},{"type":"text","expression":"input.name"}]. Integers and decimals use strings. Earlier inline results can be read with steps.first.output.rows[0][0].value.'} error={!Array.isArray(parameters) ? 'Enter a JSON array of typed bindings. Use strings for exact integers and decimals.' : undefined}>
      <textarea rows={7} spellCheck={false} autoComplete="off" value={typeof parameters === 'string' ? parameters : JSON.stringify(parameters, null, 2)} disabled={disabled} maxLength={262_144} onChange={e => {
        let value: unknown = e.target.value;
        try { value = parseAuthoringJson(e.target.value); } catch { /* Keep incomplete text; platform validation refuses it. */ }
        set('parameters', value === null ? 'null' : value);
      }} />
    </Field>
    <details><summary>Result and timeout limits</summary><p className="hint">Empty uses connection defaults. Namespace and platform limits also apply.</p>
      {([['maxRows', 'Maximum rows', 10_000], ['maxResultBytes', 'Maximum result bytes', 4_194_304], ['timeoutMs', 'Query timeout (ms)', 30_000], ['connectTimeoutMs', 'Connect timeout (ms)', 10_000]] as const).map(([field, label, max]) => <Field key={field} label={label} hint={`1–${max}`}>
        <input inputMode="numeric" autoComplete="off" disabled={disabled} value={scalar(limits[field])} onChange={e => {
          const next = { ...limits };
          if (e.target.value === '') delete next[field]; else next[field] = /^\d+$/.test(e.target.value) ? Number(e.target.value) : e.target.value;
          set('limits', Object.keys(next).length ? next : undefined);
        }} />
      </Field>)}
    </details>
    {issue ? <p role="status" className="error">{issue}</p> : null}
  </div>;
}
