import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { useId, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { path } from '../../api/client';
import { isApiError } from '../../api/errors';
import { useApp, useCan, useNamespace } from '../../app/context';
import { emptyDocument } from '../../model/ir';
import { Actions, Banner, Button, ColumnHeader, Empty, ErrorState, Hint, Link, Loading, Page, PageHeader, Row, Stack, Table, TableBody, TableCell, TableHead, TableRow, Text, TextInput, VisuallyHidden, useAnnounce } from '../../ui';
import { clip } from './canvasModel';
import { Moment } from '../runs/Now';
import { StartRun } from '../runs/StartRun';
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
      <Stack gap={3}>
        <Text>A workflow starts as an empty draft with a manual trigger. You add its steps in the editor.</Text>
        <TextInput
          label="Name"
          hint="Lowercase letters, digits and dashes; starts with a letter; 3 to 64 characters. For example order-fulfilment."
          error={fieldError}
          value={name}
          autoComplete="off"
          spellCheck={false}
          required
          onChange={setName}
        />
        {error ? <ErrorState error={error} what="Creating the workflow" /> : null}
        <Actions>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={!WORKFLOW_NAME.test(name) || busy || taken === name}>
            {busy ? 'Creating…' : 'Create workflow'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}

/** The workflows of the namespace the user works in. */
export function WorkflowsPage() {
  const { ep } = useApp();
  const namespace = useNamespace();
  const canRead = useCan('definition:read');
  const canWrite = useCan('definition:write');
  const canStart = useCan('run:start');
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
  // The search is the address's too (`?q=`), so a link to "the payment workflows" can be sent.
  const [params, setParams] = useSearchParams();
  const search = (params.get('q') ?? '').slice(0, 64);
  const needle = search.trim().toLowerCase();
  const shown = needle === '' ? workflows : workflows.filter((w) => w.name.toLowerCase().includes(needle));

  return (
    <Page>
      <PageHeader
        title="Workflows"
        actions={
          <Button variant="primary" onClick={() => (canWrite ? setCreating(true) : undefined)} aria-disabled={canWrite ? undefined : true} aria-describedby={canWrite ? undefined : reasonId}>
            New workflow
          </Button>
        }
      />
      {!canWrite ? <Hint id={reasonId}>Creating a workflow needs the definition:write permission, which your role does not have.</Hint> : null}

      {!canRead ? <Banner tone="info">You are not allowed to see workflows: it needs the definition:read permission.</Banner> : null}
      {canRead && namespace === undefined && namespaces.isSuccess && namespaces.data.items.length === 0 ? <Banner tone="info">There is no namespace to work in yet.</Banner> : null}
      {canRead && namespaces.isError ? <ErrorState error={namespaces.error} what="Loading the namespaces" onRetry={() => void namespaces.refetch()} /> : null}
      {canRead && list.isError ? <ErrorState error={list.error} what="Loading the workflows" onRetry={() => void list.refetch()} /> : null}
      {canRead && namespace !== undefined && list.isPending && !list.isError ? <Loading>Loading the workflows…</Loading> : null}
      {list.isSuccess && workflows.length === 0 ? <Empty title={`No workflows in ${namespace ?? ''} yet.${canWrite ? ' Create the first one with New workflow.' : ''}`} /> : null}

      {workflows.length > 0 ? (
        <Row align="end" gap={3}>
          <TextInput
            label="Find a workflow"
            hint={list.hasNextPage && needle !== '' ? `Searching the ${workflows.length} workflows loaded so far; load more to search further.` : undefined}
            type="search"
            width="min(100%, 24rem)"
            value={search}
            maxLength={64}
            autoComplete="off"
            spellCheck={false}
            onChange={(value) =>
              setParams((current) => {
                const next = new URLSearchParams(current);
                if (value === '') next.delete('q');
                else next.set('q', value);
                return next;
              }, { replace: true })
            }
          />
          <Hint role="status">{needle === '' ? `${workflows.length} workflows` : `${shown.length} of ${workflows.length} workflows`}</Hint>
        </Row>
      ) : null}
      {workflows.length > 0 && shown.length === 0 ? <Empty title={`No loaded workflow has "${clip(search.trim(), 40)}" in its name.`} /> : null}
      {shown.length > 0 ? (
        <Table label={`Workflows of the namespace ${namespace ?? ''}`} className="wf-list">
          <TableHead>
            <TableRow>
              <ColumnHeader>Name</ColumnHeader>
              <ColumnHeader>Active version</ColumnHeader>
              <ColumnHeader>Draft revision</ColumnHeader>
              <ColumnHeader>Last updated</ColumnHeader>
              {canStart ? (
                <ColumnHeader>
                  <VisuallyHidden>Actions</VisuallyHidden>
                </ColumnHeader>
              ) : null}
            </TableRow>
          </TableHead>
          <TableBody>
            {shown.map((w) => (
              <TableRow key={w.id}>
                <ColumnHeader scope="row">
                  <Link to={path`/workflows/${w.id}`}>{clip(w.name, 64)}</Link>
                </ColumnHeader>
                <TableCell>{w.activeVersion ? `Version ${w.activeVersion.versionNumber}` : 'None (not published)'}</TableCell>
                <TableCell>{w.draftRevision}</TableCell>
                <TableCell>
                  <Moment at={w.updatedAt} empty="—" />
                </TableCell>
                {canStart ? <TableCell>{w.activeVersion ? <StartRun choices={[{ id: w.id, name: w.name }]} initialId={w.id} /> : null}</TableCell> : null}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
      {list.hasNextPage ? (
        <div>
          <Button onClick={() => void list.fetchNextPage()} disabled={list.isFetchingNextPage}>
            {list.isFetchingNextPage ? 'Loading…' : 'Load more'}
          </Button>
        </div>
      ) : null}

      <Modal open={creating} title="New workflow" onClose={() => setCreating(false)}>
        <CreateForm onClose={() => setCreating(false)} />
      </Modal>
    </Page>
  );
}
