import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { FormEvent } from 'react';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, Disclosure, ErrorState, Hint, Select, Stack, TextArea, TextInput, useAnnounce } from '../../ui';
import { secretsApi } from '../secrets/api';
import { connectionsApi, postgresOf } from './api';
import type { Connection, ConnectionWrite } from './api';

const DEFAULT_LIMITS = { timeoutMs: 10000, maxRows: 1000, maxResultBytes: 1048576, connectTimeoutMs: 5000, poolSize: 4, idleTimeoutMs: 30000, maxStatementBytes: 16384, maxParameters: 64, maxParameterBytes: 65536 };
const LIMIT_FIELDS = [
  ['timeoutMs', 'Query timeout (ms)', 30000], ['maxRows', 'Maximum rows', 10000], ['maxResultBytes', 'Maximum result bytes', 4194304], ['connectTimeoutMs', 'Connect timeout (ms)', 10000], ['poolSize', 'Pool size', 16], ['idleTimeoutMs', 'Idle timeout (ms)', 60000], ['maxStatementBytes', 'Maximum SQL bytes', 65536], ['maxParameters', 'Maximum parameters', 256], ['maxParameterBytes', 'Maximum parameter bytes', 262144],
] as const;

/** The whole number a text holds, or not a number at all: digits only, as a number input accepted. */
const whole = (text: string): number => (/^\d+$/.test(text.trim()) ? Number(text.trim()) : Number.NaN);

export function ConnectionForm({ namespace, development, connection, onClose }: { namespace: string; development: boolean; connection?: Connection; onClose: () => void }) {
  const { api, queries, session } = useApp();
  const announce = useAnnounce();
  const canSecret = useCan('secret:manage');
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const secretRequest = useRef<string | undefined>(undefined);
  const connectionRequest = useRef<{ content: string; id: string } | undefined>(undefined);
  const config = connection ? postgresOf(connection) : undefined;
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
  // A limit is kept as the text that was typed, so a half-typed number is shown as it is and refused, never rounded.
  const [limitText, setLimitText] = useState(() => Object.fromEntries(Object.entries({ ...DEFAULT_LIMITS, ...config?.limits }).map(([key, value]) => [key, String(value)])) as Record<keyof typeof DEFAULT_LIMITS, string>);
  const limits = Object.fromEntries(LIMIT_FIELDS.map(([key]) => [key, whole(limitText[key])])) as typeof DEFAULT_LIMITS;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [notice, setNotice] = useState('');
  const secrets = useQuery({ queryKey: ['secrets', namespace], queryFn: ({ signal }) => secretsApi(api, namespace).list(signal) });
  const validHost = host.length <= 253 && /^[a-zA-Z0-9.:-]+$/.test(host) && !host.includes('://');
  const validLimits = LIMIT_FIELDS.every(([key, , max]) => Number.isInteger(limits[key]) && limits[key] > 0 && limits[key] <= max);
  const portNumber = whole(port);
  const validPort = Number.isInteger(portNumber) && portNumber > 0 && portNumber <= 65535;
  const valid = validLimits && /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/.test(name) && !name.includes('--') && validHost && validPort && !!database && database.length <= 63 && (auth === 'new' ? canSecret && !!username && !!password : !!secretName);
  const secretOptions = [
    ...(secretName && !secrets.data?.items.some(s => s.name === secretName) ? [{ value: secretName, label: secretName }] : []),
    ...(secrets.data?.items ?? []).map(s => ({ value: s.name, label: s.provider !== 'openbao' ? `${s.name} · migrate first` : s.name, disabled: s.provider !== 'openbao' })),
  ];
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
    const choices = { expectedRevision: connection?.revision ?? 0, status, config: { driver: 'postgresql' as const, host, port: portNumber, database, tls: tls === 'verify-full' ? { mode: 'verify-full' as const, ...(ca.trim() ? { caPem: ca.trim() } : {}) } : { mode: 'development-plaintext' as const }, secretName: reference, limits } };
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
  return (
    <form noValidate onSubmit={event => { void submit(event); }}>
      <Stack gap={3}>
        <TextInput label="Name" hint="A unique name of 3–63 lower-case letters, digits and hyphens." value={name} onChange={setName} disabled={busy || !!connection} maxLength={63} />
        <Hint>PostgreSQL · revision {connection?.revision ?? 'new'}</Hint>
        <TextInput label="Host" hint="A hostname or IP address. Credentials are entered separately." error={host && !validHost ? 'Enter a hostname or IP without a URL, path or credentials.' : undefined} value={host} onChange={setHost} autoComplete="off" maxLength={253} disabled={busy} />
        <div className="connection-target">
          <TextInput numeric label="Port" error={validPort ? undefined : 'Enter a whole number from 1 to 65535.'} value={port} onChange={setPort} disabled={busy} />
          <TextInput label="Database" value={database} onChange={setDatabase} maxLength={63} disabled={busy} />
        </div>
        <Select
          label="TLS"
          value={tls}
          onChange={setTls}
          disabled={busy}
          options={[{ value: 'verify-full' as const, label: 'Verify certificate and hostname' }, ...(development ? [{ value: 'development-plaintext' as const, label: 'Plaintext (development only)' }] : [])]}
        />
        {tls === 'verify-full' ? (
          <Disclosure summary="Custom CA certificates (optional)" open={!!ca}>
            <TextArea label="CA certificates" hint="Public PEM certificates for a private CA. Leave blank to use trusted system certificates." value={ca} onChange={setCa} rows={3} maxLength={65536} spellCheck={false} mono disabled={busy} />
          </Disclosure>
        ) : <Hint>The development executor must also explicitly allow plaintext connections.</Hint>}
        <Select
          label="Credentials"
          value={auth}
          onChange={next => { setAuth(next); setUsername(''); setPassword(''); secretRequest.current = undefined; }}
          disabled={busy}
          options={[{ value: 'existing', label: 'Existing secret' }, ...(canSecret ? [{ value: 'new', label: 'Create a secret' }] : [])]}
        />
        {auth === 'existing' ? (
          <Select label="Secret reference" hint="Choose an OpenBao secret containing username and password. Migrate legacy secrets on the Secrets page first." value={secretName} onChange={setSecretName} options={secretOptions} placeholder="Select a secret" disabled={busy} />
        ) : (
          <div className="connection-credentials">
            <TextInput label="Username" value={username} onChange={next => { secretRequest.current = undefined; setUsername(next); }} autoComplete="off" disabled={busy} />
            <TextInput label="Password" type="password" value={password} onChange={next => { secretRequest.current = undefined; setPassword(next); }} autoComplete="new-password" disabled={busy} />
          </div>
        )}
        {secrets.error && auth === 'existing' ? <ErrorState error={secrets.error} what="Loading secret references" onRetry={() => { void secrets.refetch(); }} /> : null}
        <Disclosure summary="Execution limits">
          <Stack gap={3}>
            {LIMIT_FIELDS.map(([key, label, max]) => (
              <TextInput numeric key={key} label={label} error={!Number.isInteger(limits[key]) || limits[key] < 1 || limits[key] > max ? `Enter a whole number from 1 to ${max}.` : undefined} value={limitText[key]} onChange={next => setLimitText({ ...limitText, [key]: next })} disabled={busy} />
            ))}
          </Stack>
        </Disclosure>
        <Select label="Status" hint="Saving settings does not test database connectivity." value={status} onChange={setStatus} disabled={busy} options={[{ value: 'PAUSED' as const, label: 'Paused' }, { value: 'ACTIVE' as const, label: 'Active' }]} />
        {notice ? <Banner tone="info">{notice}</Banner> : null}
        {error ? <ErrorState error={error} what="Saving the connection" /> : null}
        <Actions><Button onClick={onClose} disabled={busy}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : 'Save connection'}</Button></Actions>
      </Stack>
    </form>
  );
}
