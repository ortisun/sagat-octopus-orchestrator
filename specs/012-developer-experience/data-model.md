# Data Model: CLI, quickstarts, local testing and documentation

## Context

Feature 012 is almost all tooling, so it adds **no table and no migration**. What it keeps is files a person owns
(workflows, mock files, tokens, examples), two small things the server does (a version endpoint, a seed), and one
process that signs tokens for a development machine. Data ownership still follows `docs/architecture/overview.md`
and the reference physical schema in `specs/001-platform-foundation/data-model.md`; T012-006 (additive migrations)
is therefore met by there being none, and the migration suites are unchanged.

## What the feature touches in the platform

| Where | What | Tenant boundary |
|---|---|---|
| `GET /v1/version` | `{service, version, apiVersion, workerProtocol: {minimum, maximum}}`, from the build; nothing stored | authenticated, no permission, no tenant data in the answer |
| `seed-dev` (`SAGAT_SEED_NAMESPACE`) | a workspace and a namespace for the development tenant, made through `IdentityService` as its administrator | the tenant seed-dev made; audited as any creation (`workspace.created`, `namespace.created`) |
| `worker bind` / `worker dev --bind` | `PUT /namespaces/{id}/worker-bindings/{subject}`: replaces the activities an identity may serve in the namespace (feature 009) | the caller's tenant; needs the permission that route already needs |
| `dev token` | nothing stored; asks the development identity provider | none: the provider is not the platform |

## Files the tool reads and writes

| File | Shape | Bounds | Notes |
|---|---|---|---|
| a workflow document (`workflow.json`/`.yaml`) | the IR of `specs/001-platform-foundation/contracts/workflow.schema.json` | 1 MiB; bounded YAML expansion (32 MiB budget) | validated with the platform's codec |
| a mock file (`mocks.yaml`) | `delayMs`, `activities`, `compensations`, `reconcilers` (below) | 200 names, 10 s of delay, unknown keys refused | never contains a secret; templates carry none |
| a token file (`--token-file`, `dev token --out`) | the token and a newline | 16 KiB read; written `0600`, replacing | the token is otherwise only in `SAGAT_TOKEN` |
| a script source (`script test|publish`) | TypeScript or Python source | the platform's source limit | created or replaced as a script named after the file |
| an example's build outputs | `node_modules`, `dist`, `.venv`, `bin`, `obj`, `target` | ignored by git | removed and rebuilt by `prepare.sh`/the benchmark |

### A mock file

```yaml
delayMs: 0                      # how long each call takes, at most 10000
activities:
  <name>:
    output: {…} | echo: true | fail: {code, retryable, message} | lose: true | delayMs: N
    sequence: [<one answer>, …] # one answer per call; the last one repeats
compensations:
  <name>: <the same>            # not listed: succeeds with {}
reconcilers:
  <name>: { effect: applied|not_applied|unknown, output: {…} }   # not listed: unknown
```

An activity of the workflow not in `activities` fails for good as `NO_MOCK`. Calls are counted per name and kind
(`FORWARD`, `COMPENSATION`, `RECONCILIATION`), not per run, so a sequence advances with every call whichever run makes it.

## The development identity provider

State: one signing key (ES256), in memory or in a file (`0600`) so that tokens outlive a restart. It stores no
subject, tenant or token. Output: a JWKS (`kid` derived from the public key), an OIDC discovery document, and tokens
`{iss, aud, sub, <tenant claim>, iat, exp}` with `exp - iat` at most 3600 s.

## Entities from the previous contexts that the feature only reads

- **Definition/Version/Artifact:** immutable after publication; tenant and namespace required. The tool publishes
  through the same routes as the manifest of feature 011 (create or update the draft, publish, with deterministic
  idempotency keys for a document that has not changed).
- **Run/Step/Attempt/JournalEvent:** read by `workflow run --wait`, `inspect`, `runs`; the journal carries no payload
  (G144), only the digest of an output, which is what the end-to-end test of the examples compares.
- **Lease/Subscription/Timer:** `worker dev` takes leases like any worker; a `lose` answer leaves the lease to expire.

## Invariants

- Every tenant-scoped call carries the caller's token; the tool never learns another tenant's data
  (`fault_developer_experience::another_tenant_cannot_see_run_bind_or_serve_what_is_not_its_own`).
- The tool never prints or sends the token anywhere but the address it was given
  (`fault_developer_experience::no_command_prints_the_token_…`).
- The provider signs only when `SAGAT_ENV=dev`, is reachable only as configured, and never for the platform's own actors.
- A generated page is what a regeneration gives (`contract_developer_experience`).
- Large payloads still go to the blob store with a digest; nothing here puts one in a file or a log.

## Migrations

None. The feature is additive to the API (one operation, `getPlatformVersion`) and to the worker bindings use
(unchanged routes).
