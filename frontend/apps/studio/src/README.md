# src

## Overview

Browser entry point that loads settings, constructs application context, and mounts the Studio.

## Interface

- Module startup creates Root using createApp with loaded settings and renders under React StrictMode. [frontend/apps/studio/src/main.tsx:loadSettings] [frontend/apps/studio/src/main.tsx:createRoot]
- [api](api/README.md) — Typed same-origin REST access, Studio endpoint facade, safe errors, live run following, and generated OpenAPI contracts.
- [app](app/README.md) — Studio composition, authentication and namespace gates, lazy routing shell, namespace selection, and deployment settings.
- [auth](auth/README.md) — Browser OIDC Authorization Code with PKCE and an in-memory authenticated session store.
- [features](features/README.md) — Studio feature areas composed by the application routes.
- [model](model/README.md) — Pure workflow IR editing, local advice, graph layout/projection, reference rewriting, exact JSON parsing, and status view models.
- [ui](ui/README.md) — Accessible React primitives for announcements, controls, dialogs, statuses, tabs, and responsive subscriptions.

## Invariants

- Startup requires the root element and throws if it is absent; the HTML entry supplies it and loads main.tsx as a module. [frontend/apps/studio/src/main.tsx:container] [frontend/apps/studio/index.html:root]

## Side Effects

- Loads settings asynchronously, creates a React DOM root, and renders into the page. [frontend/apps/studio/src/main.tsx:loadSettings] [frontend/apps/studio/src/main.tsx:createRoot]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
npm --prefix frontend run check:api
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
