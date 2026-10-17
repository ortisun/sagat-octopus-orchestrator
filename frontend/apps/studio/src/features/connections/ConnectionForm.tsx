import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { FormEvent } from 'react';
import { useApp, useCan } from '../../app/context';
import { Field } from '../../ui/Field';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { useAnnounce } from '../../ui/Announcer';
import { secretsApi } from '../secrets/api';
import { connectionsApi } from './api';
import type { Connection, ConnectionWrite } from './api';

const DEFAULT_LIMITS = { timeoutMs: 10000, maxRows: 1000, maxResultBytes: 1048576, connectTimeoutMs: 5000, poolSize: 4, idleTimeoutMs: 30000, maxStatementBytes: 16384, maxParameters: 64, maxParameterBytes: 65536 };
const LIMIT_FIELDS = [
  ['timeoutMs', 'Query timeout (ms)', 30000], ['maxRows', 'Maximum rows', 10000], ['maxResultBytes', 'Maximum result bytes', 4194304], ['connectTimeoutMs', 'Connect timeout (ms)', 10000], ['poolSize', 'Pool size', 16], ['idleTimeoutMs', 'Idle timeout (ms)', 60000], ['maxStatementBytes', 'Maximum SQL bytes', 65536], ['maxParameters', 'Maximum parameters', 256], ['maxParameterBytes', 'Maximum parameter bytes', 262144],
] as const;

export function ConnectionForm({ namespace, development, connection, onClose }: { namespace: string; development: boolean; connection?: Connection; onClose: () => void }) {
  const { api, queries, session } = useApp();
  const announce = useAnnounce();
  const canSecret = useCan('secret:manage');
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const secretRequest = useRef<string | undefined>(undefined);
  const connectionRequest = useRef<{ content: string; id: string } | undefined>(undefined);
  const config = connection?.config;
  const [name, setName] = useState(connection?.name ?? '');
  const [host, setHost] = useState(config?.host ?? '');
  const [port, setPort] = useState(String(config?.port ?? 5432));
  const [database, setDatabase] = useState(config?.database ?? '');
  const [tls, setTls] = useState(config?.tls.mode ?? 'verify-full');
  const [ca, setCa] = useState(config?.tls.mode === 'verify-full' ? config.tls.caPem ?? '' : '');
  const [status, setStatus] = useState<'ACTIVE' | 'PAUSED'>(connection?.status === 'ACTIVE' ? 'ACTIVE' : 'PAUSED');
  const [auth, setAuth] = useState('existing');
  const [secretName, setSecretName] = useState(config?.secretName ?? '');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [limits, setLimits] = useState({ ...DEFAULT_LIMITS, ...config?.limits });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [notice, setNotice] = useState('');
  const secrets = useQuery({ queryKey: ['secrets', namespace], queryFn: ({ signal }) => secretsApi(api, namespace).list(signal) });
  const validHost = host.length <= 253 && /^[a-zA-Z0-9.:-]+$/.test(host) && !host.includes('://');
  const validLimits = LIMIT_FIELDS.every(([key, , max]) => Number.isInteger(limits[key]) && limits[key] > 0 && limits[key] <= max);
  const valid = validLimits && /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/.test(name) && !name.includes('--') && validHost && Number.isInteger(Number(port)) && Number(port) > 0 && Number(port) <= 65535 && !!database && database.length <= 63 && (auth === 'new' ? canSecret && !!username && !!password : !!secretName);
  async function submit(event: FormEvent) {
    event.preventDefault(); if (!valid || busy) return;
    setBusy(true); setError(undefined);
    const sessionToken = session.token();
    const live = () => mounted.current && session.token() === sessionToken;
    let reference = secretName;
    if (auth === 'new') {
      const requestId = secretRequest.current ??= crypto.randomUUID();
      reference = `database-${requestId}`;
      try {
        const saved = await secretsApi(api, namespace).put(reference, JSON.stringify({ username, password }), 0, requestId);
        if (saved.provider !== 'openbao') throw new Error('Required secret authority unavailable');
      } catch {
        if (live()) { setNotice('Credentials could not be saved. Check OpenBao availability and secret management permission, then retry.'); setBusy(false); }
        return;
      }
      if (!live()) return;
      setUsername(''); setPassword(''); secretRequest.current = undefined;
      setSecretName(reference); setAuth('existing');
      setNotice(`Credentials saved as ${reference}. Retry saving this connection to reuse them.`);
      void queries.invalidateQueries({ queryKey: ['secrets', namespace] });
    }
    const choices = { expectedRevision: connection?.revision ?? 0, status, config: { driver: 'postgresql' as const, host, port: Number(port), database, tls: tls === 'verify-full' ? { mode: 'verify-full' as const, ...(ca.trim() ? { caPem: ca.trim() } : {}) } : { mode: 'development-plaintext' as const }, secretName: reference, limits } };
    const content = JSON.stringify({ name, ...choices });
    if (connectionRequest.current?.content !== content) connectionRequest.current = { content, id: crypto.randomUUID() };
    const body: ConnectionWrite = { ...choices, requestId: connectionRequest.current.id };
    try {
      await connectionsApi(api, namespace).put(name, body);
      if (!live()) return;
      await queries.invalidateQueries({ queryKey: ['connections', namespace] });
      if (live()) { announce(`Connection ${name} saved.`); onClose(); }
    } catch (problem) { if (live()) setError(problem); }
    finally { if (live()) setBusy(false); }
  }
  return <form className="connection-form" noValidate onSubmit={event => { void submit(event); }}>
    <fieldset disabled={busy}>
      <Field label="Name" hint="A unique name of 3–63 lower-case letters, digits and hyphens."><input value={name} onChange={e => setName(e.target.value)} disabled={!!connection} maxLength={63} /></Field>
      <p className="hint">PostgreSQL · revision {connection?.revision ?? 'new'}</p>
      <Field label="Host" hint="A hostname or IP address. Credentials are entered separately." error={host && !validHost ? 'Enter a hostname or IP without a URL, path or credentials.' : undefined}><input value={host} onChange={e => setHost(e.target.value)} autoComplete="off" maxLength={253} /></Field>
      <div className="connection-target">
      <Field label="Port"><input type="number" value={port} onChange={e => setPort(e.target.value)} min={1} max={65535} /></Field>
      <Field label="Database"><input value={database} onChange={e => setDatabase(e.target.value)} maxLength={63} /></Field>
      </div>
      <Field label="TLS"><select value={tls} onChange={e => setTls(e.target.value as typeof tls)}><option value="verify-full">Verify certificate and hostname</option>{development ? <option value="development-plaintext">Plaintext (development only)</option> : null}</select></Field>
      {tls === 'verify-full' ? <details open={ca ? true : undefined}><summary>Custom CA certificates (optional)</summary><Field label="CA certificates" hint="Public PEM certificates for a private CA. Leave blank to use trusted system certificates."><textarea value={ca} onChange={e => setCa(e.target.value)} rows={3} maxLength={65536} spellCheck={false} /></Field></details> : <p className="hint">The development executor must also explicitly allow plaintext connections.</p>}
      <Field label="Credentials"><select value={auth} onChange={e => { setAuth(e.target.value); setUsername(''); setPassword(''); secretRequest.current = undefined; }}><option value="existing">Existing secret</option>{canSecret ? <option value="new">Create a secret</option> : null}</select></Field>
      {auth === 'existing' ? <Field label="Secret reference" hint="Choose an OpenBao secret containing username and password. Migrate legacy secrets on the Secrets page first."><select value={secretName} onChange={e => setSecretName(e.target.value)}><option value="">Select a secret</option>{secretName && !secrets.data?.items.some(s => s.name === secretName) ? <option value={secretName}>{secretName}</option> : null}{secrets.data?.items.map(s => <option key={s.name} value={s.name} disabled={s.provider !== 'openbao'}>{s.name}{s.provider !== 'openbao' ? ' · migrate first' : ''}</option>)}</select></Field> : <div className="connection-credentials"><Field label="Username"><input value={username} onChange={e => { secretRequest.current = undefined; setUsername(e.target.value); }} autoComplete="off" /></Field><Field label="Password"><input type="password" value={password} onChange={e => { secretRequest.current = undefined; setPassword(e.target.value); }} autoComplete="new-password" /></Field></div>}
      {secrets.error && auth === 'existing' ? <ErrorState error={secrets.error} what="Loading secret references" onRetry={() => { void secrets.refetch(); }} /> : null}
      <details><summary>Execution limits</summary>{LIMIT_FIELDS.map(([key, label, max]) => <Field key={key} label={label} error={!Number.isInteger(limits[key]) || limits[key] < 1 || limits[key] > max ? `Enter a whole number from 1 to ${max}.` : undefined}><input type="number" value={limits[key]} min={1} max={max} onChange={e => setLimits({ ...limits, [key]: Number(e.target.value) })} /></Field>)}</details>
      <Field label="Status" hint="Saving settings does not test database connectivity."><select value={status} onChange={e => setStatus(e.target.value as typeof status)}><option value="PAUSED">Paused</option><option value="ACTIVE">Active</option></select></Field>
    </fieldset>
    {notice ? <p role="status">{notice}</p> : null}
    {error ? <ErrorState error={error} what="Saving the connection" /> : null}
    <div className="dialog-actions"><Button onClick={onClose} disabled={busy}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : 'Save connection'}</Button></div>
  </form>;
}
