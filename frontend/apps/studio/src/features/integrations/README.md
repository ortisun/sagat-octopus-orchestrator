# integrations

## Overview

Namespace broker connector administration and egress-policy editing.

## Interface

- `IntegrationForm`, `IntegrationsPage`, and `NetworkAccess` are UI entries; integrationsApi and Connector/ConnectorInput aliases expose connector and policy requests. [frontend/apps/studio/src/features/integrations/IntegrationForm.tsx:IntegrationForm] [frontend/apps/studio/src/features/integrations/IntegrationsPage.tsx:IntegrationsPage] [frontend/apps/studio/src/features/integrations/NetworkAccess.tsx:NetworkAccess] [frontend/apps/studio/src/features/integrations/api.ts:integrationsApi] [frontend/apps/studio/src/features/integrations/api.ts:Connector] [frontend/apps/studio/src/features/integrations/api.ts:ConnectorInput]

## Invariants

- Connector slugs have 3–63 lowercase letters/digits/hyphens without consecutive hyphens; targets, required fields, credentials, and port ranges are validated before submission. [frontend/apps/studio/src/features/integrations/IntegrationForm.tsx:IntegrationForm]
- New credentials require secret:manage and TLS/SSL broker addresses or HTTPS SQS; form values are replaced with a secret reference after storage. [frontend/apps/studio/src/features/integrations/IntegrationForm.tsx:IntegrationForm]
- Viewing needs event:read, editing needs event:manage, and network changes need egress:manage. Egress ports are integers from 1 through 65,535. [frontend/apps/studio/src/features/integrations/IntegrationsPage.tsx:IntegrationsPage] [frontend/apps/studio/src/features/integrations/IntegrationsPage.tsx:IntegrationList] [frontend/apps/studio/src/features/integrations/NetworkAccess.tsx:NetworkAccess]

## Side Effects

- Reads/writes egress policy and connectors, and can store credentials as namespace secrets before saving a connector. Saves invalidate queries, announce success, and close the form. [frontend/apps/studio/src/features/integrations/api.ts:integrationsApi] [frontend/apps/studio/src/features/integrations/IntegrationForm.tsx:IntegrationForm]
- Connector lists poll every 15 seconds; network policy writes refetch the policy. [frontend/apps/studio/src/features/integrations/IntegrationsPage.tsx:IntegrationList] [frontend/apps/studio/src/features/integrations/NetworkAccess.tsx:NetworkAccess]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
