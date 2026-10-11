# Data Model: GitOps, versioning and definition promotion

## Context

Maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and illustrative SQL in `contracts/schema.sql`.

## Entities and observable state

- **Definition/Version/Artifact:** immutable after publication; tenant and namespace required.
- **Run/Step/Attempt:** stable identifiers, version, status, timestamps, deadlines, parent/correlation.
- **JournalEvent:** append-only seq per Run, event_type, payload_ref, causation/correlation and actor.
- **Inbox/Outbox:** uniqueness for dedupe and delivery; next_attempt_at; last_error without PII.
- **Lease/Subscription/Timer:** fencing generation, validity, deadline and transition status.
- **SagaEntry/Compensation:** state, dependency order, idempotency key, reason and outcome.

## Invariants

- Every tenant-scoped record validates the security context.
- No in-place update of a published version.
- Every operational state change records a sequenced event and information for reconstruction.
- Large payloads go to the blob store with digest and non-sensitive references.

## Migrations

Additive migrations first; expand/contract for existing schemas; `CONCURRENTLY` indexes where applicable on large databases; idempotent and measured backfill. See the feature plan and ADRs.

## What feature 011 added (migration `20261016000000_versioning`)

Everything is additive; the down migration removes it in the reverse order and a test runs the migration back and
forth with data in the tables (`tests/integration/gitops-versioning/migration.rs`). Every table is tenant-scoped, and
every foreign key that points at a tenant's row includes the tenant, so the database refuses a row that points into
another tenant (`isolation.rs::the_database_refuses_a_policy_a_request_or_a_dependency_that_points_into_another_tenant`).

### Tables

| Table | One row is | Notes |
|---|---|---|
| `publication_policies` | how many people other than the author must approve a publication into a namespace | `(tenant, namespace)` primary key; `required_approvals` 0..3; no row means the default of the environment (production 1, otherwise 0) |
| `publication_requests` | a request to publish a workflow's draft (kind `PUBLISH`) or the document of a version of another namespace (kind `PROMOTE`) | names `draft_revision` and the `digest` of the document the approvers saw; `required_approvals` is what the policy asked for when it was opened (the policy in force is read again when it is published); `state`; who published, when and which version; for a promotion `promoted_from_version_id`; a partial unique index allows one `PENDING`/`APPROVED` request per workflow |
| `publication_decisions` | what one person decided about a request, and why | primary key `(request, approver)`: a person decides once; `verdict` `APPROVED`/`REJECTED`; the reason is 8..2000 characters |
| `workflow_activations` | one change of the pointer to the active version | `kind` `PUBLISH`/`PROMOTE`/`ROLLBACK`/`ACTIVATE`, the version, the version before it, who, why, the request that authorised it, the version a promotion came from; **append-only** (a trigger refuses update, delete and truncate; the runtime role has `SELECT, INSERT`); the last row of a workflow is its pointer; index `(tenant, workflow, created_at DESC, id DESC)` |
| `workflow_version_scripts` | a script artifact a version pins, by digest | foreign key to `script_artifacts` with `ON DELETE RESTRICT`: an artifact a version depends on cannot be removed |

### Request states

```
            open                        approve (enough approvals)
   (nothing) ─────▶ PENDING ──────────────────────────────▶ APPROVED ──publish──▶ PUBLISHED

   from either open state:   reject (one is enough) ──▶ REJECTED
                             withdraw (the author)  ──▶ WITHDRAWN
                             the draft is edited    ──▶ SUPERSEDED
```

`PENDING` and `APPROVED` are the open states; the other four are final. A request in a final state frees the slot for
the next one. `APPROVED` is reached when the approvals equal what the policy asked for when the request was opened;
publishing counts them again against the policy in force then.

### Invariants

- The last `workflow_activations` row of a workflow is the version `workflows.active_version_id` points at; both are
  written in one transaction with the workflow row locked, so the history is the order in which the pointer moved
  (a row is stored strictly after the latest one of its workflow: `fault history.rs`, `fault storm.rs`).
- A version, once published, is never changed; a rollback adds a row to the history and moves the pointer, and nothing
  else (`activation.rs::every_publication_is_a_row_of_the_history_and_a_rollback_adds_one`).
- A request publishes the draft at the revision it names, only while it is `APPROVED` with the approvals the policy
  asks for now, and at most once.
- The author of a request is never one of its approvers, and a person is at most one decision.
- The event triggers in force for a workflow are those of its active version, with ids that do not change when the
  pointer moves.
- An artifact pinned by a version is not removable.
