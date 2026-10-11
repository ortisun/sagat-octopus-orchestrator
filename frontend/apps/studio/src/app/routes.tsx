import { Navigate } from 'react-router';
import type { RouteObject } from 'react-router';
import { Shell } from './Shell';
import { NotFoundPage } from './NotFoundPage';

/** The pages load on demand, so the editor and the graph libraries are not in the first download. */
export const routes: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    children: [
      { index: true, element: <Navigate to="/workflows" replace /> },
      { path: 'workflows', lazy: async () => ({ Component: (await import('../features/workflows/WorkflowsPage')).WorkflowsPage }) },
      { path: 'workflows/:workflowId', lazy: async () => ({ Component: (await import('../features/workflows/WorkflowEditorPage')).WorkflowEditorPage }) },
      { path: 'runs', lazy: async () => ({ Component: (await import('../features/runs/RunsPage')).RunsPage }) },
      { path: 'runs/:runId', lazy: async () => ({ Component: (await import('../features/runs/RunDetailPage')).RunDetailPage }) },
      { path: 'scripts', lazy: async () => ({ Component: (await import('../features/scripts/ScriptsPage')).ScriptsPage }) },
      { path: 'scripts/:scriptId', lazy: async () => ({ Component: (await import('../features/scripts/ScriptEditorPage')).ScriptEditorPage }) },
      { path: '*', element: <NotFoundPage /> },
    ],
  },
];
