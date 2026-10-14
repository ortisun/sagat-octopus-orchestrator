import type { RouteObject } from 'react-router';
import { Landing, NamespaceGate } from './NamespaceGate';
import { Shell } from './Shell';
import { NotFoundPage } from './NotFoundPage';
import { RouteErrorPage } from './RouteErrorPage';

/** The pages load on demand, so the editor and the graph libraries are not in the first download. */
export const routes: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    errorElement: <RouteErrorPage />,
    children: [
      { index: true, element: <Landing /> },
      // The tenant's own page: it is there whether or not a namespace is chosen.
      { path: 'namespaces', lazy: async () => ({ Component: (await import('./NamespacesPage')).NamespacesPage }) },
      // Everything else lives in a namespace and waits for one to be chosen.
      {
        element: <NamespaceGate />,
        children: [
          { path: 'integrations', lazy: async () => ({ Component: (await import('../features/integrations/IntegrationsPage')).IntegrationsPage }) },
          { path: 'operations', lazy: async () => ({ Component: (await import('../features/operations/OperationsPage')).OperationsPage }) },
          { path: 'workflows', lazy: async () => ({ Component: (await import('../features/workflows/WorkflowsPage')).WorkflowsPage }) },
          { path: 'workflows/:workflowId', lazy: async () => ({ Component: (await import('../features/workflows/WorkflowEditorPage')).WorkflowEditorPage }) },
          { path: 'runs', lazy: async () => ({ Component: (await import('../features/runs/RunsPage')).RunsPage }) },
          { path: 'runs/:runId', lazy: async () => ({ Component: (await import('../features/runs/RunDetailPage')).RunDetailPage }) },
          { path: 'scripts', lazy: async () => ({ Component: (await import('../features/scripts/ScriptsPage')).ScriptsPage }) },
          { path: 'scripts/:scriptId', lazy: async () => ({ Component: (await import('../features/scripts/ScriptEditorPage')).ScriptEditorPage }) },
        ],
      },
      { path: '*', element: <NotFoundPage /> },
    ],
  },
];
