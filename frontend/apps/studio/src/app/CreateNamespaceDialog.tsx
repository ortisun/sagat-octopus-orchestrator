import { useState } from 'react';
import type { FormEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useApp } from './context';
import type { NamespaceResource } from '../api/endpoints';
import type { Page } from '../api/types';
import { isApiError } from '../api/errors';
import { Actions, Banner, Button, Dialog, ErrorState, Loading, Select, Stack, TextInput, useAnnounce } from '../ui';

const ENVIRONMENTS = [
  { value: 'dev', label: 'Development' },
  { value: 'staging', label: 'Staging' },
  { value: 'prod', label: 'Production' },
] as const;

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
  return (
    <form onSubmit={(event) => { void submit(event); }} noValidate>
      <Stack gap={3}>
        {workspaces.isPending ? <Loading>Loading workspaces…</Loading> : null}
        {workspaces.error ? <ErrorState error={workspaces.error} what="Loading workspaces" onRetry={() => { void workspaces.refetch(); }} /> : null}
        {workspaces.data?.length === 0 ? <Banner tone="info">Create a workspace through the platform API before creating a namespace.</Banner> : null}
        <Select label="Workspace" value={selected} onChange={setWorkspace} options={(workspaces.data ?? []).map((item) => ({ value: item.id, label: item.slug }))} disabled={busy || !workspaces.data?.length} />
        <TextInput
          label="Namespace name"
          hint="3–63 lower-case letters, digits or hyphens. Start with a letter; no trailing or consecutive hyphens."
          error={slug && !valid ? 'Enter a valid namespace name.' : undefined}
          value={slug}
          onChange={setSlug}
          maxLength={63}
          autoComplete="off"
          spellCheck={false}
          disabled={busy}
          required
        />
        <Select label="Environment" value={environment} onChange={setEnvironment} options={ENVIRONMENTS} disabled={busy} />
        {isApiError(error) && error.status === 409 ? <Banner tone="error">This namespace name already exists. Choose another name.</Banner> : error ? <ErrorState error={error} what="Creating the namespace" /> : null}
        <Actions>
          <Button onClick={onClose} disabled={busy}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={busy || !valid || !selected}>{busy ? 'Creating…' : 'Create namespace'}</Button>
        </Actions>
      </Stack>
    </form>
  );
}
