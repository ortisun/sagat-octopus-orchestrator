import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp } from '../../app/context';
import { Field } from '../../ui/Field';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { integrationsApi } from './api';

export function NetworkAccess({ namespace }: { namespace: string }) {
  const { api } = useApp();
  const service = integrationsApi(api, namespace);
  const policy = useQuery({ queryKey: ['network-access', namespace], queryFn: ({ signal }) => service.network(signal) });
  const [draft, setDraft] = useState<{ rules: string; ports: string; allowHttp: boolean }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [saved, setSaved] = useState(false);
  const current = draft ?? { rules: policy.data?.rules.join('\n') ?? '', ports: policy.data?.ports.join(', ') ?? '', allowHttp: policy.data?.allowHttp ?? false };
  const ports = current.ports.split(/[,\s]+/).filter(Boolean).map(Number);
  const valid = ports.every((port) => Number.isInteger(port) && port > 0 && port <= 65535);
  return <details><summary>Network access</summary>
    <p>Allow the broker hosts and ports before saving an integration. This policy applies to all outbound connections in this namespace. Private networks require an explicit CIDR rule.</p>
    {policy.error ? <ErrorState error={policy.error} what="Loading network access" onRetry={() => { void policy.refetch(); }} /> : null}
    {policy.isPending ? <p role="status">Loading network access…</p> : null}
    {policy.data ? <form onSubmit={(event) => {
      event.preventDefault(); if (busy || !valid) return;
      setBusy(true); setError(undefined); setSaved(false);
      void service.saveNetwork({ rules: current.rules.split('\n').map((rule) => rule.trim()).filter(Boolean), ports, allowHttp: current.allowHttp }).then(async () => { await policy.refetch(); setDraft(undefined); setSaved(true); }, (problem: unknown) => setError(problem)).finally(() => setBusy(false));
    }}>
      <fieldset disabled={busy}>
        <Field label="Allowed hosts or networks" hint="One exact host, *.domain or CIDR network per line."><textarea value={current.rules} onChange={(event) => { setSaved(false); setDraft({ ...current, rules: event.target.value }); }} rows={3} spellCheck={false} /></Field>
        <Field label="Allowed ports" hint="Comma-separated ports. Empty uses the platform defaults." error={!valid ? 'Use port numbers between 1 and 65535.' : undefined}><input value={current.ports} onChange={(event) => { setSaved(false); setDraft({ ...current, ports: event.target.value }); }} /></Field>
        <label><input type="checkbox" checked={current.allowHttp} onChange={(event) => { setSaved(false); setDraft({ ...current, allowHttp: event.target.checked }); }} /> Allow unencrypted connections</label>
      </fieldset>
      <Button type="submit" disabled={busy || !valid}>{busy ? 'Saving…' : 'Save network access'}</Button>
      {error ? <ErrorState error={error} what="Saving network access" /> : null}
      {saved ? <p role="status">Network access saved.</p> : null}
    </form> : null}
  </details>;
}
