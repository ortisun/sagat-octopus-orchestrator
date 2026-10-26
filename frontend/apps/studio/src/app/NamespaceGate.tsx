import { Link, Navigate, Outlet } from 'react-router';
import { useCan, useNamespace } from './context';

/**
 * The pages that live in a namespace (workflows, runs, operations, scripts, integrations). Without a
 * chosen namespace there is nothing to show them from, so the person is told how to choose one, instead
 * of a page that asks the platform about no namespace at all.
 */
export function NamespaceGate() {
  const current = useNamespace();
  const canCreate = useCan('namespace:create');
  if (current !== undefined) return <Outlet />;
  return (
    <section aria-labelledby="choose-namespace-title">
      <h1 id="choose-namespace-title">Choose a namespace</h1>
      <p>
        Workflows, runs, operations, scripts and integrations belong to a namespace. Choose one in the menu.{' '}
        <Link to="/namespaces">Open the namespace list</Link> to see them all{canCreate ? ' or to create a new one' : ''}.
      </p>
    </section>
  );
}

/** Where the Studio opens: in the workflows of the namespace the person works in, or in the list to choose one from. */
export function Landing() {
  const current = useNamespace();
  return <Navigate to={current === undefined ? '/namespaces' : '/workflows'} replace />;
}
