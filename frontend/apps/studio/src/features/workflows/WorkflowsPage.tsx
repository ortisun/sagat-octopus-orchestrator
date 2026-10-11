import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { useId, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { path } from '../../api/client';
import { isApiError } from '../../api/errors';
import { useApp, useCan, useNamespace } from '../../app/context';
import { emptyDocument } from '../../model/ir';
import { useAnnounce } from '../../ui/Announcer';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { clip } from './canvasModel';
import { Modal } from './Modal';
import '../../styles/workflows.css';

/** A workflow's name is a slug: the same pattern the platform checks. */
export const WORKFLOW_NAME = /^[a-z][a-z0-9-]{2,63}$/;

function nameProblem(name: string): string | undefined {
  if (name === '') return undefined;
  if (WORKFLOW_NAME.test(name)) return undefined;
  if (!/^[a-z]/.test(name)) return 'A name starts with a lowercase letter.';
  if (name.length < 3) return 'A name has at least 3 characters.';
  if (name.length > 64) return 'A name has at most 64 characters.';
  return 'A name holds only lowercase letters, digits and dashes.';
}

function CreateForm({ onClose }: { onClose: () => void }) {
  const { ep } = useApp();
  const queries = useQueryClient();
  const navigate = useNavigate();
  const announce = useAnnounce();
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [taken, setTaken] = useState<string | undefined>();
  const local = nameProblem(name);
  const fieldError = local ?? (taken !== undefined && taken === name ? `A workflow named "${name}" already exists in this namespace.` : undefined);

  const submit = async () => {
    if (!WORKFLOW_NAME.test(name) || busy) return;
    setBusy(true);
    setError(undefined);
    try {
      const created = await ep.createWorkflow(name, emptyDocument(name));
      void queries.invalidateQueries({ queryKey: ['workflows'] });
      announce(`Created workflow ${name}.`);
      onClose();
      void navigate(path`/workflows/${created.id}`);
    } catch (failure) {
      if (isApiError(failure) && failure.code === 'CONFLICT') setTaken(name);
      else setError(failure);
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <p>A workflow starts as an empty draft with a manual trigger. You add its steps in the editor.</p>
      <Field label="Name" hint="Lowercase letters, digits and dashes; starts with a letter; 3 to 64 characters. For example order-fulfilment." error={fieldError}>
        <input type="text" value={name} autoComplete="off" spellCheck={false} aria-required="true" onChange={(event) => setName(event.target.value)} />
      </Field>
      {error ? <ErrorState error={error} what="Creating the workflow" /> : null}
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={!WORKFLOW_NAME.test(name) || busy || taken === name}>
          {busy ? 'Creating…' : 'Create workflow'}
        </Button>
      </div>
    </form>
  );
}

function when(iso: string | undefined): string {
  if (!iso) return '—';
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
}

/** The workflows of the namespace the user works in. */
export function WorkflowsPage() {
  const { ep } = useApp();
  const namespace = useNamespace();
  const canRead = useCan('definition:read');
  const canWrite = useCan('definition:write');
  const [creating, setCreating] = useState(false);
  const reasonId = useId();

  const namespaces = useQuery({ queryKey: ['namespaces'], queryFn: ({ signal }) => ep.namespaces(signal) });
  const list = useInfiniteQuery({
    queryKey: ['workflows', namespace],
    enabled: canRead && namespace !== undefined,
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => ep.listWorkflows({ limit: 50, ...(pageParam ? { cursor: pageParam } : {}) }, signal),
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const workflows = list.data?.pages.flatMap((p) => p.items) ?? [];

  return (
    <section>
      <div className="wf-page-head">
        <h1>Workflows</h1>
        <Button
          variant="primary"
          onClick={() => (canWrite ? setCreating(true) : undefined)}
          aria-disabled={canWrite ? undefined : true}
          aria-describedby={canWrite ? undefined : reasonId}
        >
          New workflow
        </Button>
      </div>
      {!canWrite ? (
        <p id={reasonId} className="wf-reason">
          Creating a workflow needs the definition:write permission, which your role does not have.
        </p>
      ) : null}

      {!canRead ? <Banner tone="info">You are not allowed to see workflows: it needs the definition:read permission.</Banner> : null}
      {canRead && namespace === undefined && namespaces.isSuccess && namespaces.data.items.length === 0 ? <Banner tone="info">There is no namespace to work in yet.</Banner> : null}
      {canRead && namespaces.isError ? <ErrorState error={namespaces.error} what="Loading the namespaces" onRetry={() => void namespaces.refetch()} /> : null}
      {canRead && list.isError ? <ErrorState error={list.error} what="Loading the workflows" onRetry={() => void list.refetch()} /> : null}
      {canRead && namespace !== undefined && list.isPending && !list.isError ? <p role="status">Loading the workflows…</p> : null}
      {list.isSuccess && workflows.length === 0 ? (
        <p className="empty">
          No workflows in <code>{namespace}</code> yet.{canWrite ? ' Create the first one with New workflow.' : ''}
        </p>
      ) : null}

      {workflows.length > 0 ? (
        <table className="data">
          <caption className="visually-hidden">Workflows of the namespace {namespace}</caption>
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Active version</th>
              <th scope="col">Draft revision</th>
              <th scope="col">Last updated</th>
            </tr>
          </thead>
          <tbody>
            {workflows.map((w) => (
              <tr key={w.id}>
                <th scope="row">
                  <Link to={path`/workflows/${w.id}`}>{clip(w.name, 64)}</Link>
                </th>
                <td>{w.activeVersion ? `Version ${w.activeVersion.versionNumber}` : 'None (not published)'}</td>
                <td>{w.draftRevision}</td>
                <td>{when(w.updatedAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
      {list.hasNextPage ? (
        <p>
          <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>
            {list.isFetchingNextPage ? 'Loading…' : 'Load more'}
          </Button>
        </p>
      ) : null}

      <Modal open={creating} title="New workflow" onClose={() => setCreating(false)}>
        <CreateForm onClose={() => setCreating(false)} />
      </Modal>
    </section>
  );
}
