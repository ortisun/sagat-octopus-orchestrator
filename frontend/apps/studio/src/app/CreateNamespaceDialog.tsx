import { useState } from 'react';
import type { FormEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp } from './context';
import type { NamespaceResource } from '../api/endpoints';
import type { Page } from '../api/types';
import { isApiError } from '../api/errors';
import { Button } from '../ui/Button';
import { Dialog } from '../ui/Dialog';
import { Field } from '../ui/Field';
import { ErrorState } from '../ui/ErrorState';
import { useAnnounce } from '../ui/Announcer';

export function CreateNamespaceDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  return <Dialog open={open} title="New namespace" onClose={onClose}><CreateForm onClose={onClose} /></Dialog>;
}

function CreateForm({ onClose }: { onClose: () => void }) {
  const { ep, queries, namespace, session } = useApp();
  const announce = useAnnounce();
  const workspaces = useQuery({ queryKey: ['workspaces'], queryFn: ({ signal }) => ep.workspaces(signal) });
  const [workspace, setWorkspace] = useState('');
  const [slug, setSlug] = useState('');
  const [environment, setEnvironment] = useState<NamespaceResource['environment']>('dev');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const selected = workspace || workspaces.data?.[0]?.id || '';
  const valid = /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/.test(slug) && !slug.includes('--');
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || !valid || !selected) return;
    setBusy(true);
    setError(undefined);
    const token = session.token();
    try {
      const created = await ep.createNamespace(selected, { slug, environment });
      if (session.token() !== token) return;
      queries.setQueryData<Page<NamespaceResource>>(['namespaces'], (old) => ({ ...old, items: [...(old?.items ?? []).filter((item) => item.id !== created.id), created] }));
      namespace.set(created.slug);
      announce(`Namespace ${created.slug} created.`);
      onClose();
    } catch (problem) {
      if (session.token() === token) setError(problem);
    } finally {
      setBusy(false);
    }
  };
  return <form onSubmit={(event) => { void submit(event); }} noValidate>
    {workspaces.isPending ? <p role="status">Loading workspaces…</p> : null}
    {workspaces.error ? <ErrorState error={workspaces.error} what="Loading workspaces" onRetry={() => { void workspaces.refetch(); }} /> : null}
    {workspaces.data?.length === 0 ? <p role="status">Create a workspace through the platform API before creating a namespace.</p> : null}
    <Field label="Workspace"><select value={selected} onChange={(event) => setWorkspace(event.target.value)} disabled={busy || !workspaces.data?.length}>{workspaces.data?.map((item) => <option key={item.id} value={item.id}>{item.slug}</option>)}</select></Field>
    <Field label="Namespace name" hint="3–63 lower-case letters, digits or hyphens. Start with a letter; no trailing or consecutive hyphens." error={slug && !valid ? 'Enter a valid namespace name.' : undefined}><input value={slug} onChange={(event) => setSlug(event.target.value)} maxLength={63} autoComplete="off" spellCheck={false} disabled={busy} required /></Field>
    <Field label="Environment"><select value={environment} onChange={(event) => setEnvironment(event.target.value as NamespaceResource['environment'])} disabled={busy}><option value="dev">Development</option><option value="staging">Staging</option><option value="prod">Production</option></select></Field>
    {isApiError(error) && error.status === 409 ? <p role="alert">This namespace name already exists. Choose another name.</p> : error ? <ErrorState error={error} what="Creating the namespace" /> : null}
    <div className="dialog-actions"><Button onClick={onClose} disabled={busy}>Cancel</Button><Button type="submit" variant="primary" disabled={busy || !valid || !selected}>{busy ? 'Creating…' : 'Create namespace'}</Button></div>
  </form>;
}
