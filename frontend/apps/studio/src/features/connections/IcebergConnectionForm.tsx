import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { FormEvent } from 'react';
import { useApp, useCan } from '../../app/context';
import { Actions, Banner, Button, Checkbox, CheckboxGroup, Disclosure, ErrorState, Heading, Hint, Select, Stack, TextArea, TextInput, useAnnounce } from '../../ui';
import { secretsApi } from '../secrets/api';
import type { Secret } from '../secrets/api';
import { connectionsApi, editableIcebergOf } from './api';
import type { Connection, ConnectionWrite, IcebergConfig } from './api';

type Auth = 'oauth2' | 'bearer' | 'sigv4';
type Mode = 'append' | 'overwrite' | 'upsert';
type Ceiling = 'public' | 'internal' | 'pii';
type Role = 'catalog' | 'storage';
/** One field of a credential secret. Every value is write-only: it is sent once and never shown again. */
type SecretField = { key: string; label: string; optional?: boolean };
/** Where a credential comes from: a secret that exists, or the values of one to create in OpenBao now. */
type Choice = { source: 'existing' | 'new'; name: string; values: Record<string, string> };

const MODES: readonly Mode[] = ['append', 'overwrite', 'upsert'];
const AWS_KEYS: readonly SecretField[] = [{ key: 'accessKeyId', label: 'Access key ID' }, { key: 'secretAccessKey', label: 'Secret access key' }, { key: 'sessionToken', label: 'Session token', optional: true }];
/** The JSON of each secret (specs/035-iceberg-write-steps/data-model.md, "Credential secrets"). */
const CATALOG_SECRET: Record<Auth, readonly SecretField[]> = { oauth2: [{ key: 'clientId', label: 'Client ID' }, { key: 'clientSecret', label: 'Client secret' }], bearer: [{ key: 'token', label: 'Token' }], sigv4: AWS_KEYS };
const LIMIT_FIELDS = [['maxRecordBytes', 'Maximum record bytes', 1024, 2147483648], ['maxRecords', 'Maximum records', 1, 100000000], ['timeoutMs', 'Write timeout (ms)', 1000, 3600000]] as const;
const LOCATION = /^s3:\/\/[^?#@\s]+$/;
const EXISTING: Choice = { source: 'existing', name: '', values: {} };

/** The problem with a catalog or storage URL, if any. The server decides; this only saves a round trip. */
function uriProblem(value: string, development: boolean): string | undefined {
  if (!value) return undefined;
  let url: URL;
  try { url = new URL(value); } catch { return 'Enter a full URL such as https://catalog.example.com.'; }
  if (url.username || url.password || url.search || url.hash) return 'Remove credentials, query strings and fragments from the URL. Credentials belong in a secret.';
  if (url.protocol !== 'https:' && !(development && url.protocol === 'http:')) return development ? 'Use an http or https URL.' : 'Use an https URL.';
  return value.length > 2048 ? 'Use at most 2048 characters.' : undefined;
}
const filled = (value: string) => value.trim().length > 0;
const choiceReady = (choice: Choice, fields: readonly SecretField[], canCreate: boolean) => choice.source === 'existing' ? !!choice.name : canCreate && fields.every(field => field.optional || filled(choice.values[field.key] ?? ''));

function SecretChoice({ title, fields, choice, onChange, secrets, canCreate, disabled }: { title: string; fields: readonly SecretField[]; choice: Choice; onChange: (next: Choice) => void; secrets: readonly Secret[]; canCreate: boolean; disabled: boolean }) {
  const options = [
    ...(choice.name && !secrets.some(s => s.name === choice.name) ? [{ value: choice.name, label: choice.name }] : []),
    ...secrets.map(s => ({ value: s.name, label: s.provider !== 'openbao' ? `${s.name} · migrate first` : s.name, disabled: s.provider !== 'openbao' })),
  ];
  return (
    <>
      <Select
        label={`${title} credentials`}
        value={choice.source}
        onChange={next => onChange({ source: next, name: choice.name, values: {} })}
        options={[{ value: 'existing' as const, label: 'Existing secret' }, ...(canCreate ? [{ value: 'new' as const, label: 'Create a secret' }] : [])]}
        disabled={disabled}
      />
      {choice.source === 'existing'
        ? <Select label={`${title} secret reference`} hint="Choose an OpenBao secret. Migrate legacy secrets on the Secrets page first." value={choice.name} onChange={name => onChange({ ...choice, name })} options={options} placeholder="Select a secret" disabled={disabled} />
        : (
          <div className="connection-credentials">
            {fields.map(field => (
              // A secret value is write-only: it is typed once, masked, and never shown again.
              <TextInput key={field.key} type="password" label={`${title} ${field.label.toLowerCase()}${field.optional ? ' (optional)' : ''}`} value={choice.values[field.key] ?? ''} onChange={next => onChange({ ...choice, values: { ...choice.values, [field.key]: next } })} autoComplete="off" disabled={disabled} />
            ))}
          </div>
        )}
    </>
  );
}

export function IcebergConnectionForm({ namespace, development, connection, onClose }: { namespace: string; development: boolean; connection?: Connection; onClose: () => void }) {
  const { api, queries, session } = useApp();
  const announce = useAnnounce();
  const canSecret = useCan('secret:manage');
  const canReveal = useCan('payload:reveal-pii');
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const secretRequests = useRef<Partial<Record<Role, string>>>({});
  const connectionRequest = useRef<{ content: string; id: string } | undefined>(undefined);
  const existing: IcebergConfig | undefined = connection ? editableIcebergOf(connection) : undefined;
  const rest = existing?.catalog.rest;
  const s3 = existing?.storage.s3;
  const [name, setName] = useState(connection?.name ?? '');
  const [uri, setUri] = useState(rest?.uri ?? '');
  const [warehouse, setWarehouse] = useState(rest?.warehouse ?? '');
  const [prefix, setPrefix] = useState(rest?.prefix ?? '');
  const [auth, setAuth] = useState<Auth>(rest?.auth.type ?? 'oauth2');
  const [tokenUri, setTokenUri] = useState(rest?.auth.type === 'oauth2' ? rest.auth.tokenUri : '');
  const [scope, setScope] = useState(rest?.auth.type === 'oauth2' ? rest.auth.scope ?? '' : '');
  const [signRegion, setSignRegion] = useState(rest?.auth.type === 'sigv4' ? rest.auth.region : '');
  const [signingName, setSigningName] = useState(rest?.auth.type === 'sigv4' ? rest.auth.signingName : '');
  const [delegation, setDelegation] = useState(rest?.accessDelegation === 'vended-credentials');
  const [catalog, setCatalog] = useState<Choice>({ ...EXISTING, name: existing?.catalogSecret ?? '' });
  const [vended, setVended] = useState(existing?.storage.credentials === 'vended');
  const [storage, setStorage] = useState<Choice>({ ...EXISTING, name: existing?.storageSecret ?? '' });
  const [region, setRegion] = useState(s3?.region ?? '');
  const [endpoint, setEndpoint] = useState(s3?.endpoint ?? '');
  const [pathStyle, setPathStyle] = useState(s3?.pathStyle ?? false);
  const [roleArn, setRoleArn] = useState(s3?.roleArn ?? '');
  const [externalId, setExternalId] = useState(s3?.externalId ?? '');
  const [locations, setLocations] = useState((existing?.warehouseLocations ?? []).join('\n'));
  const [modes, setModes] = useState<readonly Mode[]>(existing?.allowedModes ?? ['append']);
  const [ceiling, setCeiling] = useState<Ceiling>(existing?.egressCeiling ?? 'internal');
  const [limits, setLimits] = useState<Record<string, string>>(Object.fromEntries(Object.entries(existing?.limits ?? {}).map(([key, value]) => [key, String(value)])));
  const [status, setStatus] = useState<'ACTIVE' | 'PAUSED'>(connection?.status === 'ACTIVE' ? 'ACTIVE' : 'PAUSED');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [notice, setNotice] = useState('');
  const secrets = useQuery({ queryKey: ['secrets', namespace], queryFn: ({ signal }) => secretsApi(api, namespace).list(signal) });
  const secretItems = secrets.data?.items ?? [];

  const locationList = locations.split('\n').map(line => line.trim()).filter(Boolean);
  const locationsValid = locationList.length >= 1 && locationList.length <= 16 && new Set(locationList).size === locationList.length && locationList.every(l => LOCATION.test(l) && l.length <= 1024);
  const limitProblem = (key: string, min: number, max: number) => { const text = limits[key]; if (!text) return undefined; const n = Number(text); return Number.isInteger(n) && n >= min && n <= max ? undefined : `Enter a whole number from ${min} to ${max}, or leave it blank.`; };
  const uriError = uriProblem(uri, development);
  const tokenError = auth === 'oauth2' ? uriProblem(tokenUri, development) : undefined;
  const endpointError = uriProblem(endpoint, development);
  const catalogFields = CATALOG_SECRET[auth];
  const authValid = auth === 'oauth2' ? filled(tokenUri) && !tokenError : auth === 'sigv4' ? filled(signRegion) && filled(signingName) && signRegion.length <= 63 && signingName.length <= 63 : true;
  const valid = /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/.test(name) && !name.includes('--')
    && filled(uri) && !uriError && filled(warehouse) && warehouse.length <= 1024 && prefix.length <= 255 && authValid
    && filled(region) && region.length <= 63 && !endpointError && roleArn.length <= 2048 && externalId.length <= 1224
    && locationsValid && modes.length >= 1 && (ceiling !== 'pii' || canReveal) && LIMIT_FIELDS.every(([key, , min, max]) => !limitProblem(key, min, max))
    && choiceReady(catalog, catalogFields, canSecret) && (vended || choiceReady(storage, AWS_KEYS, canSecret));

  function change(role: Role, next: Choice) {
    secretRequests.current[role] = undefined; // changed values are a new secret, so a new receipt
    if (role === 'catalog') setCatalog(next); else setStorage(next);
  }
  /** Writes the values of a new secret to OpenBao and returns its reference; an existing secret is returned as it is. */
  async function reference(role: Role, choice: Choice, fields: readonly SecretField[]): Promise<string> {
    if (choice.source === 'existing') return choice.name;
    const requestId = secretRequests.current[role] ??= crypto.randomUUID();
    const secretName = `lake-${role}-${requestId}`;
    const value = JSON.stringify(Object.fromEntries(fields.flatMap(field => { const text = choice.values[field.key]; return text ? [[field.key, text]] : []; })));
    const saved = await secretsApi(api, namespace).put(secretName, value, 0, requestId);
    if (saved.provider !== 'openbao') throw new Error('Required secret authority unavailable');
    return secretName;
  }
  async function submit(event: FormEvent) {
    event.preventDefault(); if (!valid || busy) return;
    setBusy(true); setError(undefined); setNotice('');
    const sessionToken = session.token();
    const live = () => mounted.current && session.token() === sessionToken;
    let catalogSecret: string; let storageSecret: string | undefined;
    try {
      catalogSecret = await reference('catalog', catalog, catalogFields);
      if (!live()) return;
      if (catalog.source === 'new') { setCatalog({ source: 'existing', name: catalogSecret, values: {} }); secretRequests.current.catalog = undefined; }
      storageSecret = vended ? undefined : await reference('storage', storage, AWS_KEYS);
      if (!live()) return;
      if (!vended && storage.source === 'new') { setStorage({ source: 'existing', name: storageSecret ?? '', values: {} }); secretRequests.current.storage = undefined; }
      if (catalog.source === 'new' || (!vended && storage.source === 'new')) {
        setNotice('Credentials saved. If saving the connection fails, retry: the saved secrets are reused.');
        void queries.invalidateQueries({ queryKey: ['secrets', namespace] });
      }
    } catch {
      if (live()) { setNotice('Credentials could not be saved. Check OpenBao availability and secret management permission, then retry.'); setBusy(false); }
      return;
    }
    const authBody = auth === 'oauth2' ? { type: 'oauth2' as const, tokenUri: tokenUri.trim(), ...(scope.trim() ? { scope: scope.trim() } : {}) } : auth === 'sigv4' ? { type: 'sigv4' as const, region: signRegion.trim(), signingName: signingName.trim() } : { type: 'bearer' as const };
    const limitBody = Object.fromEntries(LIMIT_FIELDS.flatMap(([key]) => limits[key] ? [[key, Number(limits[key])]] : []));
    const config: IcebergConfig = {
      driver: 'iceberg',
      catalog: { type: 'rest', rest: { uri: uri.trim(), warehouse: warehouse.trim(), ...(prefix.trim() ? { prefix: prefix.trim() } : {}), auth: authBody, accessDelegation: delegation ? 'vended-credentials' : 'none', remoteSigning: rest?.remoteSigning ?? false } },
      storage: { type: 's3', s3: { region: region.trim(), ...(endpoint.trim() ? { endpoint: endpoint.trim() } : {}), pathStyle, ...(roleArn.trim() ? { roleArn: roleArn.trim() } : {}), ...(externalId.trim() ? { externalId: externalId.trim() } : {}) }, credentials: vended ? 'vended' : 'static' },
      warehouseLocations: locationList,
      allowedModes: MODES.filter(mode => modes.includes(mode)),
      catalogSecret,
      ...(storageSecret ? { storageSecret } : {}),
      egressCeiling: ceiling,
      ...(Object.keys(limitBody).length ? { limits: limitBody } : {}),
    };
    const choices = { expectedRevision: connection?.revision ?? 0, status, config };
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
        <Hint>Apache Iceberg · REST catalog · S3-compatible storage · revision {connection?.revision ?? 'new'}</Hint>
        <Heading level={3}>Catalog</Heading>
        <TextInput label="Catalog URL" hint="HTTPS outside development. Never put credentials in the URL." error={uriError} value={uri} onChange={setUri} autoComplete="off" maxLength={2048} disabled={busy} />
        <div className="connection-credentials">
          <TextInput label="Warehouse" value={warehouse} onChange={setWarehouse} maxLength={1024} disabled={busy} />
          <TextInput label="Prefix (optional)" value={prefix} onChange={setPrefix} maxLength={255} disabled={busy} />
        </div>
        <Select
          label="Catalog authentication"
          value={auth}
          onChange={next => { setAuth(next); change('catalog', { ...catalog, values: {} }); }}
          options={[{ value: 'oauth2' as const, label: 'OAuth2 client credentials' }, { value: 'bearer' as const, label: 'Bearer token' }, { value: 'sigv4' as const, label: 'AWS SigV4' }]}
          disabled={busy}
        />
        {auth === 'oauth2' ? (
          <div className="connection-credentials">
            <TextInput label="Token URL" error={tokenError} value={tokenUri} onChange={setTokenUri} autoComplete="off" maxLength={2048} disabled={busy} />
            <TextInput label="Scope (optional)" value={scope} onChange={setScope} maxLength={1024} disabled={busy} />
          </div>
        ) : null}
        {auth === 'sigv4' ? (
          <div className="connection-credentials">
            <TextInput label="Signing region" value={signRegion} onChange={setSignRegion} maxLength={63} disabled={busy} />
            <TextInput label="Signing name" value={signingName} onChange={setSigningName} maxLength={63} disabled={busy} />
          </div>
        ) : null}
        <Checkbox label="The catalog vends storage credentials" checked={delegation} onChange={checked => { setDelegation(checked); if (!checked) setVended(false); }} disabled={busy} />
        <SecretChoice title="Catalog" fields={catalogFields} choice={catalog} onChange={next => change('catalog', next)} secrets={secretItems} canCreate={canSecret} disabled={busy} />
        <Heading level={3}>Storage</Heading>
        <div className="connection-credentials">
          <TextInput label="Region" value={region} onChange={setRegion} maxLength={63} disabled={busy} />
          <TextInput label="Endpoint (optional)" hint="For S3-compatible storage." error={endpointError} value={endpoint} onChange={setEndpoint} autoComplete="off" maxLength={2048} disabled={busy} />
        </div>
        <Checkbox label="Path-style addressing" checked={pathStyle} onChange={setPathStyle} disabled={busy} />
        <Select
          label="Storage access"
          value={vended ? 'vended' : 'static'}
          onChange={next => { setVended(next === 'vended'); change('storage', { ...storage, values: {} }); }}
          options={[{ value: 'static' as const, label: 'Static credentials (a secret)' }, ...(delegation ? [{ value: 'vended' as const, label: 'Credentials vended by the catalog' }] : [])]}
          disabled={busy}
        />
        {vended ? <Hint>Vended credentials arrive inside the sandboxed write and are never stored or shown.</Hint> : <SecretChoice title="Storage" fields={AWS_KEYS} choice={storage} onChange={next => change('storage', next)} secrets={secretItems} canCreate={canSecret} disabled={busy} />}
        <Disclosure summary="Assume a role (optional)" open={!!(roleArn || externalId)}>
          <div className="connection-credentials">
            <TextInput label="Role ARN" value={roleArn} onChange={setRoleArn} maxLength={2048} disabled={busy} />
            <TextInput label="External ID" value={externalId} onChange={setExternalId} maxLength={1224} disabled={busy} />
          </div>
        </Disclosure>
        <Heading level={3}>Writes</Heading>
        <TextArea label="Warehouse locations" hint="One s3:// prefix per line, 1 to 16. Every table and data location must start with one of them." error={locations && !locationsValid ? 'Enter 1 to 16 distinct s3:// prefixes, one per line.' : undefined} value={locations} onChange={setLocations} rows={3} spellCheck={false} mono disabled={busy} />
        <CheckboxGroup
          label="Allowed write modes"
          value={modes}
          onChange={next => setModes(MODES.filter(mode => next.includes(mode)))}
          options={MODES.map(mode => ({ value: mode, label: mode }))}
          error={modes.length === 0 ? 'Allow at least one mode.' : undefined}
          disabled={busy}
        />
        <Select
          label="Highest data level a step may write"
          hint={canReveal ? 'Writing personal data needs the reveal grant for this namespace.' : 'Writing personal data needs the payload reveal permission, which you do not hold.'}
          value={ceiling}
          onChange={setCeiling}
          options={[{ value: 'public' as const, label: 'Public' }, { value: 'internal' as const, label: 'Internal' }, { value: 'pii' as const, label: 'Personal data (pii)', disabled: !canReveal }]}
          disabled={busy}
        />
        <Disclosure summary="Write limits (optional)" open={Object.keys(limits).length > 0}>
          <Stack gap={3}>
            {LIMIT_FIELDS.map(([key, label, min, max]) => (
              <TextInput numeric key={key} label={label} error={limitProblem(key, min, max)} value={limits[key] ?? ''} onChange={next => setLimits({ ...limits, [key]: next })} disabled={busy} />
            ))}
          </Stack>
        </Disclosure>
        <Select label="Status" hint="Saving settings does not test connectivity to the catalog or the storage." value={status} onChange={setStatus} options={[{ value: 'PAUSED' as const, label: 'Paused' }, { value: 'ACTIVE' as const, label: 'Active' }]} disabled={busy} />
        {secrets.error ? <ErrorState error={secrets.error} what="Loading secret references" onRetry={() => { void secrets.refetch(); }} /> : null}
        {notice ? <Banner tone="info">{notice}</Banner> : null}
        {error ? <ErrorState error={error} what="Saving the connection" /> : null}
        <Actions><Button onClick={onClose} disabled={busy}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : 'Save connection'}</Button></Actions>
      </Stack>
    </form>
  );
}
