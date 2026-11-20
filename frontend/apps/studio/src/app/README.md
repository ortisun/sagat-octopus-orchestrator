# app

## Overview

Studio composition, authentication and namespace gates, lazy routing shell, namespace selection, and deployment settings.

## Interface

- `Root`, `SignIn`, `OidcCallback`, `NamespaceGate`, `Landing`, `Shell`, and SCOPED compose authentication, routing, and namespace navigation. [frontend/apps/studio/src/app/Root.tsx:Root] [frontend/apps/studio/src/app/SignIn.tsx:SignIn] [frontend/apps/studio/src/app/OidcCallback.tsx:OidcCallback] [frontend/apps/studio/src/app/NamespaceGate.tsx:NamespaceGate] [frontend/apps/studio/src/app/NamespaceGate.tsx:Landing] [frontend/apps/studio/src/app/Shell.tsx:Shell] [frontend/apps/studio/src/app/Shell.tsx:SCOPED]
- `CreateNamespaceDialog`, `NamespacesPage`, `NotFoundPage`, and `RouteErrorPage` provide namespace and route-management surfaces. [frontend/apps/studio/src/app/CreateNamespaceDialog.tsx:CreateNamespaceDialog] [frontend/apps/studio/src/app/NamespacesPage.tsx:NamespacesPage] [frontend/apps/studio/src/app/NotFoundPage.tsx:NotFoundPage] [frontend/apps/studio/src/app/RouteErrorPage.tsx:RouteErrorPage]
- `StudioSettings`, `App`, `AppOptions`, createApp, AppContext, and app/session/permission/namespace hooks form the composition contract. NamespaceStore, routes, and loadSettings supply persisted selection, route definitions, and config loading. [frontend/apps/studio/src/app/context.tsx:StudioSettings] [frontend/apps/studio/src/app/context.tsx:App] [frontend/apps/studio/src/app/context.tsx:AppOptions] [frontend/apps/studio/src/app/context.tsx:createApp] [frontend/apps/studio/src/app/context.tsx:AppContext] [frontend/apps/studio/src/app/context.tsx:useApp] [frontend/apps/studio/src/app/context.tsx:useSession] [frontend/apps/studio/src/app/context.tsx:useCan] [frontend/apps/studio/src/app/context.tsx:useNamespace] [frontend/apps/studio/src/app/namespace.ts:NamespaceStore] [frontend/apps/studio/src/app/routes.tsx:routes] [frontend/apps/studio/src/app/settings.ts:loadSettings]

## Invariants

- Root shows callback/sign-in before authentication and creates the /studio router only after session information exists. Namespace features render only after namespace selection. [frontend/apps/studio/src/app/Root.tsx:Root] [frontend/apps/studio/src/app/Root.tsx:Authenticated] [frontend/apps/studio/src/app/NamespaceGate.tsx:NamespaceGate]
- A remembered namespace absent from the tenant list is cleared without choosing a replacement. NamespaceStore accepts only lowercase alphanumeric/hyphen slugs up to 63 characters and persists only that non-secret choice. [frontend/apps/studio/src/app/Shell.tsx:Shell] [frontend/apps/studio/src/app/namespace.ts:NamespaceStore.valid] [frontend/apps/studio/src/app/namespace.ts:NamespaceStore.set]
- createApp disables blind query retries, refreshes stale state on focus, and clears cached queries on session end. [frontend/apps/studio/src/app/context.tsx:createApp]
- Feature pages load lazily; namespaces are tenant-level, while the remaining feature routes sit behind NamespaceGate. Failed/non-OK settings fetches disable OIDC and trace URL settings. [frontend/apps/studio/src/app/routes.tsx:routes] [frontend/apps/studio/src/app/settings.ts:loadSettings]

## Side Effects

- Creates/lists namespaces through API endpoints, updates query caches and selection, announces changes, and moves focus after navigation. [frontend/apps/studio/src/app/CreateNamespaceDialog.tsx:CreateForm] [frontend/apps/studio/src/app/NamespacesPage.tsx:NamespacesPage] [frontend/apps/studio/src/app/Shell.tsx:Shell]
- Exchanges OIDC codes, signs in, and may navigate to /studio/. [frontend/apps/studio/src/app/OidcCallback.tsx:OidcCallback]
- Reads/writes/removes namespace localStorage and notifies subscribers; fetches /studio/config.json without credentials or cache. [frontend/apps/studio/src/app/namespace.ts:NamespaceStore] [frontend/apps/studio/src/app/settings.ts:loadSettings]
- Loads feature bundles dynamically; route recovery focuses its heading and can reload the page. [frontend/apps/studio/src/app/routes.tsx:routes] [frontend/apps/studio/src/app/RouteErrorPage.tsx:RouteErrorPage]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
