import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useApp } from '../../app/context';
import { Actions, Banner, Button, Hint, Select, Stack, Text, TextInput, useAnnounce } from '../../ui';
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
  return (
    <form noValidate onSubmit={event => { void submit(event); }}>
      <Stack gap={3}>
        <TextInput label="Secret name" value={name} disabled={busy || !!secret} onChange={next => { receipt.current = undefined; setName(next); }} autoComplete="off" maxLength={64} />
        {migration ? <Text>Move this secret to OpenBao while preserving its name and access list. Future reads require OpenBao. The current value will not be displayed.</Text> : <>
          <Select
            label="Credential format"
            value={format}
            disabled={busy}
            onChange={next => { receipt.current = undefined; setFormat(next); setValue(''); setUsername(''); setPassword(''); }}
            options={[{ value: 'postgresql', label: 'PostgreSQL username and password' }, { value: 'opaque', label: 'Secret value' }]}
          />
          {format === 'postgresql' ? <>
            <TextInput label="Username" value={username} disabled={busy} onChange={next => { receipt.current = undefined; setUsername(next); }} autoComplete="off" />
            <TextInput label="Password" type="password" value={password} disabled={busy} onChange={next => { receipt.current = undefined; setPassword(next); }} autoComplete="new-password" />
          </> : <TextInput label="Secret value" hint="Up to 16 KiB of UTF-8 text. Stored values are never displayed." type="password" value={value} disabled={busy} onChange={next => { receipt.current = undefined; setValue(next); }} autoComplete="new-password" maxLength={16384} />}
          {secret ? <Hint>This replaces stored version {secret.version}. Update the PostgreSQL account separately when changing its password.</Hint> : null}
        </>}
        {error ? <Banner tone="error">{error}</Banner> : null}
        <Actions><Button onClick={onClose} disabled={busy}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid}>{busy ? 'Saving…' : migration ? 'Migrate to OpenBao' : 'Save secret'}</Button></Actions>
      </Stack>
    </form>
  );
}
