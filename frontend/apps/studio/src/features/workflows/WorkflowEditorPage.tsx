import { useQuery } from '@tanstack/react-query';
import { Link, useParams } from 'react-router';
import { isApiError } from '../../api/errors';
import { useApp, useCan, useNamespace } from '../../app/context';
import { Banner } from '../../ui/Banner';
import { ErrorState } from '../../ui/ErrorState';
import { WorkflowEditor } from './WorkflowEditor';
import '../../styles/workflows.css';

/** The route of the builder: finds the workflow, then hands it to the editor, which owns it from there. */
export function WorkflowEditorPage() {
  const { workflowId = '' } = useParams();
  const { ep } = useApp();
  const namespace = useNamespace();
  const canRead = useCan('definition:read');

  const query = useQuery({
    queryKey: ['workflow-draft', namespace, workflowId],
    enabled: canRead && namespace !== undefined && workflowId !== '',
    queryFn: ({ signal }) => ep.getWorkflow(workflowId, signal),
    // The editor takes the draft once and keeps it: a refetch behind its back must not replace what is being edited.
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    refetchOnMount: 'always',
  });

  if (!canRead) {
    return (
      <section>
        <h1>Workflow</h1>
        <Banner tone="info">You are not allowed to see workflows: it needs the definition:read permission.</Banner>
      </section>
    );
  }
  if (query.isError) {
    const gone = isApiError(query.error) && query.error.code === 'NOT_FOUND';
    return (
      <section>
        <h1>Workflow</h1>
        <ErrorState error={query.error} what="Opening the workflow" onRetry={gone ? undefined : () => void query.refetch()} />
        <p>
          <Link to="/workflows">Back to the workflows</Link>
        </p>
      </section>
    );
  }
  if (!query.data) {
    return (
      <section>
        <h1>Workflow</h1>
        <p role="status">Loading the workflow…</p>
      </section>
    );
  }
  return <WorkflowEditor key={query.data.id} workflow={query.data} />;
}
