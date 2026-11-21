import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, ErrorState, Hint, Select, Stack, TextArea, TextInput, useAnnounce } from '../../ui';
import type { SelectOption } from '../../ui';
import { integrationsApi } from './api';
import type { Connector, ConnectorInput } from './api';

type Auth = 'none' | 'existing' | 'token' | 'user' | 'creds' | 'sasl' | 'aws';
export function IntegrationForm({ namespace, connector, onClose }: { namespace: string; connector: Connector | undefined; onClose: () => void }) {
  const { api, queries, session } = useApp();
  const canSecret = useCan('secret:manage');
  const announce = useAnnounce();
  const credentialRequest = useRef<string | undefined>(undefined);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const settings = connector?.settings;
  const [kind, setKind] = useState<Connector['kind']>(connector?.kind ?? 'KAFKA');
  const [slug, setSlug] = useState(connector?.slug ?? '');
  const [servers, setServers] = useState(settings && 'servers' in settings ? settings.servers.join('\n') : '');
  const [topic, setTopic] = useState(settings && 'topic' in settings ? settings.topic : '');
  const [startFrom, setStartFrom] = useState<'earliest' | 'latest'>(settings && 'startFrom' in settings ? settings.startFrom : 'earliest');
  const [stream, setStream] = useState(settings && 'stream' in settings ? settings.stream : '');
  const [consumer, setConsumer] = useState(settings && 'consumer' in settings ? settings.consumer : '');
  const [filter, setFilter] = useState(settings && 'filterSubject' in settings ? settings.filterSubject ?? '' : '');
  const [queue, setQueue] = useState(settings && 'queueUrl' in settings ? settings.queueUrl : '');
  const [region, setRegion] = useState(settings && 'region' in settings ? settings.region : '');
  const [status, setStatus] = useState<Connector['status']>(connector?.status ?? 'PAUSED');
  const [eventType, setEventType] = useState(connector?.defaultEventType ?? '');
  const [auth, setAuth] = useState<Auth>(connector?.secretName ? 'existing' : 'none');
  const [secretName, setSecretName] = useState(connector?.secretName ?? '');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [token, setToken] = useState('');
  const [mechanism, setMechanism] = useState('SCRAM-SHA-256');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [notice, setNotice] = useState('');
  const addresses = servers.split(/[\n,]+/).map((value) => value.trim()).filter(Boolean);
  const writingSecret = !['none', 'existing'].includes(auth);
  const validServers = addresses.length > 0 && addresses.every((value) => /^(?:(?:tls|ssl|nats|kafka):\/\/)?(?:[a-zA-Z0-9.-]+|\[[a-fA-F0-9:]+\]):[0-9]{1,5}$/.test(value) && Number(value.split(':').at(-1)) > 0 && Number(value.split(':').at(-1)) <= 65535);
  const secure = kind === 'SQS' ? queue.startsWith('https://') : addresses.every((value) => /^(tls|ssl):\/\//.test(value));
  const credentialsValid = auth === 'none' || (auth === 'existing' ? /^[a-z][a-z0-9_.-]{0,63}$/.test(secretName) : canSecret && (auth === 'token' || auth === 'creds' ? !!token.trim() : !!username.trim() && !!password));
  const valid = /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/.test(slug) && !slug.includes('--') && (kind === 'SQS' ? /^https:\/\/[^@?#\s]+$/.test(queue) && !!region.trim() : validServers && (kind === 'KAFKA' ? !!topic.trim() : !!stream.trim() && !!consumer.trim())) && credentialsValid && (!writingSecret || secure);
  function clearCredentials() { credentialRequest.current = undefined; setUsername(''); setPassword(''); setToken(''); }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !valid) return;
    setBusy(true); setError(undefined); setNotice('');
    const sessionToken = session.token();
    const live = () => mounted.current && session.token() === sessionToken;
    const service = integrationsApi(api, namespace);
    let reference = auth === 'existing' ? secretName : undefined;
    if (writingSecret) {
      const value = auth === 'token' ? { token } : auth === 'creds' ? { creds: token } : auth === 'user' ? { user: username, password } : auth === 'sasl' ? { mechanism, username, password } : { accessKeyId: username, secretAccessKey: password, ...(token ? { sessionToken: token } : {}) };
      const requestId = credentialRequest.current ??= crypto.randomUUID();
      reference = `integration-${requestId}`;
      try { await service.credentials(reference, JSON.stringify(value), requestId); }
      catch {
        if (live()) { setNotice('Credentials could not be saved. Check secret:manage permission and the namespace secret configuration, then retry.'); setBusy(false); }
        return;
      }
      if (!live()) return;
      clearCredentials(); setSecretName(reference); setAuth('existing');
      setNotice('Credentials saved. If saving the integration fails, retry to attach the saved credential reference.');
    }
    const body: ConnectorInput = { slug, kind, settings: kind === 'NATS' ? { servers: addresses, stream, consumer, ...(filter ? { filterSubject: filter } : {}) } : kind === 'KAFKA' ? { servers: addresses, topic, startFrom } : { queueUrl: queue, region }, ...(reference ? { secretName: reference } : {}), ...(eventType ? { defaultEventType: eventType } : {}), ratePerSecond: connector?.ratePerSecond ?? 200, status };
    try {
      await service.save(body, connector?.id);
      if (!live()) return;
      await queries.invalidateQueries({ queryKey: ['integrations', namespace] });
      if (live()) { announce(`Integration ${slug} saved.`); onClose(); }
    } catch (problem) { if (live()) setError(problem); }
    finally { if (live()) setBusy(false); }
  }
  const authentication: SelectOption<Auth>[] = [
    { value: 'none', label: 'No credentials' },
    { value: 'existing', label: 'Existing secret reference' },
    ...(canSecret && kind === 'NATS' ? [{ value: 'token' as const, label: 'Token' }, { value: 'user' as const, label: 'Username and password' }, { value: 'creds' as const, label: 'NATS credentials file contents' }] : []),
    ...(canSecret && kind === 'KAFKA' ? [{ value: 'sasl' as const, label: 'SASL username and password' }] : []),
    ...(canSecret && kind === 'SQS' ? [{ value: 'aws' as const, label: 'AWS access keys' }] : []),
  ];
  const secret = (next: string, set: (value: string) => void) => { credentialRequest.current = undefined; set(next); };
  return (
    <form onSubmit={(event) => { void submit(event); }} noValidate>
      <Stack gap={3}>
        <TextInput label="Name" hint="A unique name of 3–63 lower-case letters, digits and hyphens." value={slug} onChange={setSlug} disabled={busy || !!connector} maxLength={63} autoComplete="off" />
        <Select
          label="Type"
          value={kind}
          disabled={busy || !!connector}
          onChange={(next) => { setKind(next); setAuth('none'); setSecretName(''); clearCredentials(); }}
          options={[{ value: 'KAFKA' as const, label: 'Kafka' }, { value: 'NATS' as const, label: 'NATS JetStream' }, { value: 'SQS' as const, label: 'Amazon SQS' }]}
        />
        {kind !== 'SQS' ? (
          <TextArea label="Servers" hint="One host:port per line. Use tls://host:port for TLS. Do not include credentials in addresses." error={servers && !validServers ? 'Enter broker host:port addresses without credentials, paths or query strings.' : undefined} value={servers} onChange={setServers} rows={2} spellCheck={false} mono disabled={busy} />
        ) : <>
          <TextInput label="Queue URL" value={queue} onChange={setQueue} disabled={busy} />
          <TextInput label="AWS region" value={region} onChange={setRegion} disabled={busy} />
        </>}
        {kind === 'KAFKA' ? <>
          <TextInput label="Topic" value={topic} onChange={setTopic} disabled={busy} />
          <Select label="Initial offset" value={startFrom} onChange={setStartFrom} disabled={busy} options={[{ value: 'earliest' as const, label: 'Earliest' }, { value: 'latest' as const, label: 'Latest' }]} />
        </> : null}
        {kind === 'NATS' ? <>
          <TextInput label="Stream" value={stream} onChange={setStream} disabled={busy} />
          <TextInput label="Durable consumer" value={consumer} onChange={setConsumer} disabled={busy} />
          <TextInput label="Subject filter (optional)" value={filter} onChange={setFilter} disabled={busy} />
        </> : null}
        <Select label="Authentication" value={auth} disabled={busy} onChange={(next) => { setAuth(next); clearCredentials(); }} options={authentication} />
        {auth === 'existing' ? <TextInput label="Secret reference" hint="An existing credential secret in this namespace. Stored values are never shown." value={secretName} onChange={setSecretName} autoComplete="off" disabled={busy} /> : null}
        {auth === 'sasl' ? <Select label="SASL mechanism" value={mechanism} disabled={busy} onChange={(next) => secret(next, setMechanism)} options={['PLAIN', 'SCRAM-SHA-256', 'SCRAM-SHA-512'].map((value) => ({ value, label: value }))} /> : null}
        {['user', 'sasl', 'aws'].includes(auth) ? <>
          <TextInput label={auth === 'aws' ? 'Access key ID' : 'Username'} value={username} onChange={(next) => secret(next, setUsername)} autoComplete="off" disabled={busy} />
          <TextInput label={auth === 'aws' ? 'Secret access key' : 'Password'} type="password" value={password} onChange={(next) => secret(next, setPassword)} autoComplete="new-password" disabled={busy} />
        </> : null}
        {auth === 'creds' ? <TextArea label="Credentials file contents" value={token} onChange={(next) => secret(next, setToken)} spellCheck={false} rows={4} mono disabled={busy} /> : null}
        {auth === 'token' || auth === 'aws' ? <TextInput label={auth === 'aws' ? 'Session token (optional)' : 'Token'} type="password" value={token} onChange={(next) => secret(next, setToken)} autoComplete="new-password" disabled={busy} /> : null}
        {writingSecret && !secure ? <Banner tone="error">Use TLS broker addresses before saving credentials.</Banner> : null}
        {!canSecret ? <Hint>Entering new credentials requires secret:manage permission. You can reference an existing secret.</Hint> : null}
        <TextInput label="Default event type (optional)" value={eventType} onChange={setEventType} disabled={busy} />
        <Select label="Status" hint="Activating starts consumption. Existing messages may be processed according to the broker settings." value={status} disabled={busy} onChange={setStatus} options={[{ value: 'PAUSED' as const, label: 'Paused' }, { value: 'ACTIVE' as const, label: 'Active' }]} />
        {notice ? <Banner tone="info">{notice}</Banner> : null}
        {error ? <ErrorState error={error} what="Saving the integration" /> : null}
        <Actions><Button disabled={busy} onClick={onClose}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : 'Save integration'}</Button></Actions>
      </Stack>
    </form>
  );
}
