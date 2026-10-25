import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useApp, useCan } from '../../app/context';
import { Field } from '../../ui/Field';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { useAnnounce } from '../../ui/Announcer';
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
  return <form onSubmit={(event) => { void submit(event); }} noValidate>
    <fieldset disabled={busy}>
      <Field label="Name" hint="A unique name of 3–63 lower-case letters, digits and hyphens."><input value={slug} onChange={(event) => setSlug(event.target.value)} disabled={!!connector} maxLength={63} autoComplete="off" /></Field>
      <Field label="Type"><select value={kind} disabled={!!connector} onChange={(event) => { setKind(event.target.value as Connector['kind']); setAuth('none'); setSecretName(''); clearCredentials(); }}><option value="KAFKA">Kafka</option><option value="NATS">NATS JetStream</option><option value="SQS">Amazon SQS</option></select></Field>
      {kind !== 'SQS' ? <Field label="Servers" hint="One host:port per line. Use tls://host:port for TLS. Do not include credentials in addresses." error={servers && !validServers ? 'Enter broker host:port addresses without credentials, paths or query strings.' : undefined}><textarea value={servers} onChange={(event) => setServers(event.target.value)} rows={2} spellCheck={false} /></Field> : <><Field label="Queue URL"><input value={queue} onChange={(event) => setQueue(event.target.value)} /></Field><Field label="AWS region"><input value={region} onChange={(event) => setRegion(event.target.value)} /></Field></>}
      {kind === 'KAFKA' ? <><Field label="Topic"><input value={topic} onChange={(event) => setTopic(event.target.value)} /></Field><Field label="Initial offset"><select value={startFrom} onChange={(event) => setStartFrom(event.target.value as 'earliest' | 'latest')}><option value="earliest">Earliest</option><option value="latest">Latest</option></select></Field></> : null}
      {kind === 'NATS' ? <><Field label="Stream"><input value={stream} onChange={(event) => setStream(event.target.value)} /></Field><Field label="Durable consumer"><input value={consumer} onChange={(event) => setConsumer(event.target.value)} /></Field><Field label="Subject filter (optional)"><input value={filter} onChange={(event) => setFilter(event.target.value)} /></Field></> : null}
      <Field label="Authentication"><select value={auth} onChange={(event) => { setAuth(event.target.value as Auth); clearCredentials(); }}><option value="none">No credentials</option><option value="existing">Existing secret reference</option>{canSecret && kind === 'NATS' ? <><option value="token">Token</option><option value="user">Username and password</option><option value="creds">NATS credentials file contents</option></> : null}{canSecret && kind === 'KAFKA' ? <option value="sasl">SASL username and password</option> : null}{canSecret && kind === 'SQS' ? <option value="aws">AWS access keys</option> : null}</select></Field>
      {auth === 'existing' ? <Field label="Secret reference" hint="An existing credential secret in this namespace. Stored values are never shown."><input value={secretName} onChange={(event) => setSecretName(event.target.value)} autoComplete="off" /></Field> : null}
      {auth === 'sasl' ? <Field label="SASL mechanism"><select value={mechanism} onChange={(event) => { credentialRequest.current = undefined; setMechanism(event.target.value); }}><option>PLAIN</option><option>SCRAM-SHA-256</option><option>SCRAM-SHA-512</option></select></Field> : null}
      {['user', 'sasl', 'aws'].includes(auth) ? <><Field label={auth === 'aws' ? 'Access key ID' : 'Username'}><input value={username} onChange={(event) => { credentialRequest.current = undefined; setUsername(event.target.value); }} autoComplete="off" /></Field><Field label={auth === 'aws' ? 'Secret access key' : 'Password'}><input type="password" value={password} onChange={(event) => { credentialRequest.current = undefined; setPassword(event.target.value); }} autoComplete="new-password" /></Field></> : null}
      {['token', 'creds', 'aws'].includes(auth) ? <Field label={auth === 'creds' ? 'Credentials file contents' : auth === 'aws' ? 'Session token (optional)' : 'Token'}>{auth === 'creds' ? <textarea value={token} onChange={(event) => { credentialRequest.current = undefined; setToken(event.target.value); }} autoComplete="off" spellCheck={false} rows={4} /> : <input type="password" value={token} onChange={(event) => { credentialRequest.current = undefined; setToken(event.target.value); }} autoComplete="new-password" />}</Field> : null}
      {writingSecret && !secure ? <p role="alert">Use TLS broker addresses before saving credentials.</p> : null}
      {!canSecret ? <p className="hint">Entering new credentials requires secret:manage permission. You can reference an existing secret.</p> : null}
      <Field label="Default event type (optional)"><input value={eventType} onChange={(event) => setEventType(event.target.value)} /></Field>
      <Field label="Status" hint="Activating starts consumption. Existing messages may be processed according to the broker settings."><select value={status} onChange={(event) => setStatus(event.target.value as Connector['status'])}><option value="PAUSED">Paused</option><option value="ACTIVE">Active</option></select></Field>
    </fieldset>
    {notice ? <p role="status">{notice}</p> : null}
    {error ? <ErrorState error={error} what="Saving the integration" /> : null}
    <div className="dialog-actions"><Button disabled={busy} onClick={onClose}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : 'Save integration'}</Button></div>
  </form>;
}
