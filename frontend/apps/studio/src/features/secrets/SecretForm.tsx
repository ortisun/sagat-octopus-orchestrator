import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useApp } from '../../app/context';
import { Button } from '../../ui/Button';
import { Field } from '../../ui/Field';
import { useAnnounce } from '../../ui/Announcer';
import { secretsApi } from './api';
import type { Secret } from './api';

export function SecretForm({ namespace, secret, onClose }: { namespace: string; secret?: Secret; onClose: () => void }) {
  const { api, queries, session } = useApp(); const announce = useAnnounce();
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const receipt = useRef<string | undefined>(undefined);
  const [name, setName] = useState(secret?.name ?? '');
  const [format, setFormat] = useState('postgresql');
  const [username, setUsername] = useState(''); const [password, setPassword] = useState('');
  const [value, setValue] = useState(''); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const migration = secret?.provider === 'local';
  const content = format === 'postgresql' ? JSON.stringify({ username, password }) : value;
  const valid = /^[a-z][a-z0-9_.-]{0,63}$/.test(name) && (migration || ((format === 'postgresql' ? !!username && !!password : !!value) && new TextEncoder().encode(content).length <= 16384));
  async function submit(event: FormEvent) {
    event.preventDefault(); if (busy || !valid) return;
    setBusy(true); setError('');
    const sessionToken = session.token(); const live = () => mounted.current && session.token() === sessionToken;
    const requestId = receipt.current ??= crypto.randomUUID();
    try {
      const service = secretsApi(api, namespace);
      const saved = migration ? await service.migrate(secret, requestId) : await service.put(name, content, secret?.version ?? 0, requestId);
      if (saved.provider !== 'openbao') throw new Error('Required authority unavailable');
      if (!live()) return;
      setValue(''); setUsername(''); setPassword('');
      await queries.invalidateQueries({ queryKey: ['secrets', namespace] });
      if (live()) { announce(`Secret ${name} ${migration ? 'migrated' : 'saved'}.`); onClose(); }
    } catch {
      if (live()) setError('The secret could not be saved. Check OpenBao availability and permissions, or refresh the list if another version was saved. Retrying without edits uses the same request.');
    } finally { if (live()) setBusy(false); }
  }
  return <form className="connection-form" noValidate onSubmit={event => { void submit(event); }}>
    <fieldset disabled={busy}>
      <Field label="Secret name"><input value={name} disabled={!!secret} onChange={e => { receipt.current = undefined; setName(e.target.value); }} autoComplete="off" maxLength={64} /></Field>
      {migration ? <p>Move this secret to OpenBao while preserving its name and access list. Future reads require OpenBao. The current value will not be displayed.</p> : <>
        <Field label="Credential format"><select value={format} onChange={e => { receipt.current = undefined; setFormat(e.target.value); setValue(''); setUsername(''); setPassword(''); }}><option value="postgresql">PostgreSQL username and password</option><option value="opaque">Secret value</option></select></Field>
        {format === 'postgresql' ? <><Field label="Username"><input value={username} onChange={e => { receipt.current = undefined; setUsername(e.target.value); }} autoComplete="off" /></Field><Field label="Password"><input type="password" value={password} onChange={e => { receipt.current = undefined; setPassword(e.target.value); }} autoComplete="new-password" /></Field></> : <Field label="Secret value" hint="Up to 16 KiB of UTF-8 text. Stored values are never displayed."><input type="password" value={value} onChange={e => { receipt.current = undefined; setValue(e.target.value); }} autoComplete="new-password" maxLength={16384} /></Field>}
        {secret ? <p className="hint">This replaces stored version {secret.version}. Update the PostgreSQL account separately when changing its password.</p> : null}
      </>}
    </fieldset>
    {error ? <p className="error" role="alert">{error}</p> : null}
    <div className="dialog-actions"><Button onClick={onClose} disabled={busy}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : migration ? 'Migrate to OpenBao' : 'Save secret'}</Button></div>
  </form>;
}
