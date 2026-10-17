# Data model: per-field payload classification

**Status**: designed, not built. No table, type or endpoint below exists yet. Fields and constraints are
the implementation design; release needs the named evidence in `tasks.md`.

Domain rules use typed values; `serde_json` stays in adapter crates (AGENTS.md rule 5). The domain
holds the level, its order, the audience decision and the permission names. The schema walk, the
validation and the projection live in the adapter `crates/infra-schema`, behind ports declared in
`crates/application`.

## 1. Domain types (`crates/domain/src/classification.rs`, new)

| Type | Shape | Rules |
|---|---|---|
| `Level` | enum `Public < Internal < Pii < Secret`, `Ord` | `as_str` / `parse` of exactly `public`, `internal`, `pii`, `secret` (case-sensitive); `stricter(self, other)` is `max`; `Level::UNMARKED` is `Pii` |
| `Audience` | the caller's standing for one request: `{ internal: bool, pii: bool, structure: bool }` | `internal` = holds `payload:read-internal`; `pii` = `internal` and holds a reveal grant in the namespace **and** the request carries `reveal`; `structure` = holds `run:payload:inspect` |
| `Visibility` | enum `Disclose`, `Withhold(reason)`, `Omit` | pure function `visibility(level, audience) -> Visibility`: `Secret` is `Withhold(Secret)` if `structure` else `Omit`; `Pii` is `Disclose` if `audience.pii`, else `Withhold(RevealNotRequested \| LevelNotPermitted)` if `structure`, else `Omit`; `Internal` is `Disclose` if `audience.internal`, else withhold/omit the same way; `Public` is `Disclose` |
| `ClassificationRules` | `u8` newtype, `NONE = 0`, `CURRENT = 1` | `0` reads no annotation |

The reason distinction (`RevealNotRequested` when the caller holds the right but did not ask,
`LevelNotPermitted` otherwise) is decided by the use case from facts the caller already knows about
itself. `Level`, `Audience` and `Visibility` are in the domain because the matrix of `contracts/README.md`
is business policy that must be unit-testable without a database or JSON.

Permissions (`crates/domain/src/identity.rs`): `Permission::PayloadReadInternal` (`payload:read-internal`)
and `Permission::PayloadRevealPii` (`payload:reveal-pii`). `Permission::ALL` grows from 36 to 38 and
keeps meaning "every permission in the catalog"; `Role::Admin` stops being defined as `ALL` and is
defined as `ALL` minus `PayloadRevealPii` (a new `Role::ADMIN_PERMISSIONS` constant, tested against
`ALL` so a future permission is not forgotten). `Developer` and `Operator` gain `PayloadReadInternal`;
`Viewer`, `Worker`, `Runner` gain neither. No role grants `PayloadRevealPii`, so `Role::grants` is false
for it everywhere and the effective check is the grant lookup of section 4.

Audit (`crates/domain/src/audit.rs`): `AuditAction::{PayloadDisclosed, PayloadPiiRevealed,
PayloadRevealDenied, PayloadRevealGranted, PayloadRevealRevoked}` with the strings of
`contracts/README.md` section 3, and `ResourceType::PayloadRevealGrant` (`namespace-id/subject`, the form
`Worker` already uses). `collapses_repeats` is true for `PayloadDisclosed` and
`PayloadRevealDenied` only (window 300 s via `repeat_window`); `PayloadPiiRevealed` is never collapsed.

## 2. Classification of a schema (adapter, `crates/infra-schema/src/classification.rs`, new)

Derived, never stored. Input: a carrier (`serde_json::Value`), the rules version. Output of the
**publication check**: `Vec<ClassificationFinding { code, pointer }>` and the list of
`(pointer, Level)` for the report. Output of the **runtime walk**: for one instance value, an
`EffectiveTree` of `(instance position, Level, governed-by: Declared | Undeclared)`.

The adapter reads the carriers from the pinned canonical document (the stored `document` bytes). The
node-level `outputSchema` is an extra key that the IR schema allows on nodes and that `convert.rs`
ignores, so the domain `Node` and `NodeKind` do not change and no mapper is added.

Walk algorithm (single pass over the instance, bounded):

1. `level(root) = annotation(carrier root) ?? Pii`.
2. For an instance object at position p with effective level L and the set of governing schema nodes S
   (initially the root; after `$ref`/`allOf`/`anyOf`/`oneOf` expansion, S is the union of all
   branches): for each property key k, the governing nodes are `properties[k]` of every node in S, else
   the schema (not boolean) `additionalProperties` of every node in S, else none. If none: the child is
   `Undeclared`, level `max(L, Pii)`. Otherwise level `max(L, strictest annotation among the governing
   nodes)`; unannotated governing nodes contribute nothing (inheritance).
3. Arrays: `prefixItems[i]` for i below its length, else `items`; the same rules.
4. Expansion budget: at most 1,000 `$ref` expansions per walk and a visited set of `(pointer, level)`
   to stop cycles; exceeding it ends the walk as `SCHEMA_UNUSABLE`.
5. The result never lowers a level (`max` at every step), which is also what makes the publication rule
   "a lower declaration is an error" a lint rather than a necessity for safety.

Validation gate: `jsonschema::Validator::is_valid(&instance)` of the compiled carrier, in one call,
before the walk. No error iterator is used anywhere on instance data (its messages repeat values).

Caches (process-local, bounded, no cross-tenant sharing issue because the key is the immutable version):

| Cache | Key | Value | Bound |
|---|---|---|---|
| compiled carrier | `(workflow_version_id, carrier)` | `jsonschema::Validator` plus the parsed annotation index | 256 entries and 16 MiB of schema bytes, least recently used |

A restart empties it; nothing depends on it for correctness.

## 3. Stored data

### 3.1 `workflow_versions.classification_rules` (migration, additive)

| Column | Type | Constraint |
|---|---|---|
| `classification_rules` | `smallint NOT NULL DEFAULT 0` | `CHECK (classification_rules IN (0, 1))` |

`workflow_versions` is immutable by trigger (`workflow_versions_immutable`, `BEFORE UPDATE OR DELETE`);
adding a column with a constant default does not rewrite rows or fire it, so every existing row reads
`0` without an UPDATE and `document`, `digest` and the rest are untouched. The publish path sets `1`
on insert. The runtime reads it with the version (same query that loads the document).

### 3.2 `payload_reveal_grants` (migration, additive)

| Column | Type | Constraint |
|---|---|---|
| `tenant_id` | `uuid NOT NULL` | part of the primary key |
| `namespace_id` | `uuid NOT NULL` | part of the primary key |
| `subject` | `text NOT NULL` | part of the primary key; `CHECK (char_length(subject) BETWEEN 1 AND 255 AND subject !~ '[[:cntrl:]]')` as `memberships` |
| `granted_by` | `text NOT NULL` | the actor subject, same check |
| `granted_at` | `timestamptz NOT NULL` | |

`PRIMARY KEY (tenant_id, namespace_id, subject)`; `FOREIGN KEY (tenant_id, namespace_id) REFERENCES
namespaces (tenant_id, id)`; `FOREIGN KEY (tenant_id, subject) REFERENCES memberships (tenant_id,
subject) ON DELETE CASCADE` (so removing a membership, the incident response of owner decision 6, also
removes the grants, with no stale right). No cross-tenant reference is expressible.

Operational notes: the runtime role `sagat_app` gets `SELECT, INSERT, DELETE` only (never `UPDATE`);
the grant count per namespace is enforced at 256 in the granting transaction, counted under a row lock
on the namespace (as D42 of feature 009 counts under a lock); `crates/infra-postgres/storage-classification.json`
gets entries for `subject` and `granted_by` as `protected_system` (bounded by the CHECKs above), because
ADR-0027 requires every variable-length column to be classified. The table is not a content column and
does not take a quota trigger.

No table stores a classification level, a payload copy or a disclosure result. There is no change to
`payloads`, `run_events`, `runs` or the blob catalog. (`runs.result_digest` comes from the G21 change.)

## 4. Request lifecycle and transaction boundary

`PayloadDisclosureService::prepare` (authorization, no I/O on payloads) then `disclose`:

| Step | Where | Notes |
|---|---|---|
| 1. authorize | application | `run:read` required (`403` plain); if `reveal` is present, `payload:read-internal` also, otherwise `403` with `PII_REVEAL_NOT_GRANTED` |
| 2. resolve run | store | `(tenant, run) -> namespace`; missing or foreign is `404`; foreign also records the restricted `CrossTenantAccessDenied` |
| 3. admit | application | namespace API bucket, once (`ApiOperation::PayloadDisclose`) |
| 4. one transaction (`begin_safely`) | `crates/infra-postgres/src/payload_disclosure.rs` | `SELECT ... FROM runs ... FOR SHARE`; select the payload and its provenance exactly as `inspect_and_audit` does (input, result digest, or `(run, sequence)`); load the `document` and `classification_rules` of the version the run pins; if `reveal` was requested, read the caller's row of `payload_reveal_grants` and the role's `payload:read-internal` (absent: roll back, `403 PII_REVEAL_NOT_GRANTED`, best-effort denial audit outside the transaction) |
| 5. project | inside step 4, before commit | the store calls the application's projector with the bytes, the document and the audience; the result says which levels it contains |
| 6. audit | same transaction | `payload.pii_revealed` if the projection contains a `pii` value; `payload.disclosed` if it contains an `internal` value; nothing for a `public`-only answer |
| 7. commit, then return | store | the `Disclosure` leaves the function only after `commit` succeeds |

Steps 5 and 6 interlock: the audit record that precedes the bytes has to describe what the answer
contains, so the projection is computed before commit, from the bytes just read. The store port is
`read_project_audit(prepared, projector, audience) -> Disclosure`. The transaction stays short because
the work is bounded (FR-016-024) and CPU-only, with no network call and no external blob read. A failed
audit insert or commit returns `UNAVAILABLE` and no value.

Locks: `runs ... FOR SHARE` (as ADR-0028) and a plain read of the grant row; no lock on `workflow_versions`
(immutable). No scheduler lease, no outbox row, no timer or subscription is touched: the feature does not
change a state transition (constitution I is satisfied by not participating in any).

Granting: `PUT` takes the namespace row `FOR UPDATE`, counts grants, inserts, writes `payload.reveal_granted`,
commits. `DELETE` deletes and writes `payload.reveal_revoked` in one transaction. Both under `begin_safely`.

## 5. Failure and race behaviour

| Event | Outcome |
|---|---|
| crash or error before commit of a reveal | no audit row, no value; the client may retry |
| crash after commit, before the response | an audit row exists and the value was not delivered; a retry reveals again and audits again (at least one audit per delivered reveal, possibly more) |
| grant revoked between two requests | the next request is refused; one already inside its transaction sees the state at its read |
| grant revoked, membership deleted or role changed during step 4 | the single transaction reads one consistent grant state; a change committed after that read affects only later requests |
| version published or made active while a request runs | no effect: the run pins its version; the document is immutable |
| schema cannot be compiled (old stored document) | `structure-only/SCHEMA_UNUSABLE` for inspectors, `unavailable` otherwise |
| validator cache evicted or process restarted | recompile on next use; same answer |

## 6. Tenant boundary

Every query that reads a run, a payload, a journal event, a version or a grant includes the
authenticated `tenant_id`; the grant table's foreign keys are composite with `tenant_id`. A subject in
a path is resolved only within the caller's tenant. A missing and a foreign resource are the same
`404`; the foreign case alone is audited through `CrossTenantAccessDenied`, withheld from
tenant-visible audit. Metrics never carry tenant or run identifiers; spans may carry `tenant_id` and
`run_id` as today's inspection span does.

## 7. Where each requirement lands

| Requirement | Code |
|---|---|
| FR-016-001..004 | `crates/infra-schema/src/classification.rs`, `crates/infra-ir/src/classification_check.rs`, `crates/infra-ir/src/{schema.rs,convert.rs,lib.rs}`, `workflow.schema.json` |
| FR-016-005 | `crates/infra-postgres/migrations/*_payload_classification.up.sql`, `crates/infra-postgres/src/definitions.rs` (publish insert) |
| FR-016-006..009, 013..015 | `crates/application/src/payload_disclosure.rs`, `crates/infra-postgres/src/payload_disclosure.rs`, `crates/api-http/src/routes/payload_disclosure.rs` |
| FR-016-010..012 | `crates/domain/src/{classification.rs,identity.rs,audit.rs}`, `crates/application/src/{reveal_grants.rs,api_admission.rs}`, `crates/infra-postgres/src/reveal_grants.rs`, `crates/api-http/src/routes/reveal_grants.rs` |
| FR-016-017 | every file above; the canary suites of `tasks.md` Phase 9 |
| FR-016-018 | `frontend/apps/studio/src/features/runs/PayloadDisclosure.tsx` |
| FR-016-019 | `apps/cli/src/{workflow.rs,args.rs,reveal.rs}` |
| FR-016-020 | `sdks/{typescript,go,python,dotnet,java}/` workflow builders |
| FR-016-025 | `crates/infra-ir/src/diff.rs` |
| FR-016-026..029 | `crates/application/src/{events.rs,connectors.rs}`, `crates/infra-postgres/src/{events.rs,connectors.rs}`, `crates/api-http/src/{routes/events.rs,routes/connectors.rs,dto_events.rs}` |

## 8. Event inbox and dead letters (US08)

No new table, permission or reason (D305): the check reuses `payload_reveal_grants`, `event:read`,
`payload:read-internal` and `PII_REVEAL_NOT_GRANTED`. The event's `data` is `pii` as a whole (no carrier),
so there is no walk and no validity gate: the decision is the pure `visibility(Pii, audience)` of section 1.

| Step | Where | Notes |
|---|---|---|
| 1. authorize | `crates/application/src/events.rs` (`event`), `crates/application/src/connectors.rs` (`dead_letter`, `dead_letters`) | `event:read` as today; if `reveal=pii` is present also `payload:read-internal`, otherwise `403` with `PII_REVEAL_NOT_GRANTED`; the list ignores `reveal` |
| 2. resolve | stores | the event by `(tenant, namespace, source, event id)`, the connector and letter by tenant and connector; a foreign or missing one is the opaque `404` |
| 3. read, project, audit, commit | `crates/infra-postgres/src/events.rs` (`event`), `crates/infra-postgres/src/connectors.rs` (`dead_letter`, `dead_letters`) | without reveal, the existing pool read followed by projection with `JsonPayloadProjector` (structure, size, `headerCount`), no audit. With reveal, one `begin_safely` transaction: read the grant row of the event's (connector's) namespace, read the payload, insert `payload.pii_revealed`, commit, then return the values; a missing grant rolls back to `403` and a best-effort `payload.reveal_denied` outside the transaction |
| 4. respond | `crates/api-http/src/routes/events.rs` (`get_event`), `crates/api-http/src/routes/connectors.rs` (`get_dead_letter`, `list_dead_letters`), `crates/api-http/src/dto_events.rs` (`EventBody`, `DeadLetterBody`) | `Cache-Control: private, no-store`; `data`, `body`, `headers` are `Option` and skipped when absent |

The store trait methods gain an audience argument (`EventView::Structure | EventView::Reveal`), so an
adapter cannot return values without being asked to; the type of a structure answer has no field that
could carry a value (as `PayloadStructure` has none). `Debug` of the stored payload and of a dead-letter
body prints sizes only.

Failure and race behaviour: a crash before commit of a reveal leaves no audit row and returns no value; a
crash after commit and before the response leaves one audit row and no delivered value (a retry audits again);
a grant revoked or a membership removed between two requests refuses the next one; the event can expire
between requests, which is the ordinary `404`.

Tenant boundary: every query keeps the authenticated `tenant_id` and the namespace; the grant is read
for the event's namespace (the connector's for a letter), so a grant in namespace A never reveals a value
of namespace B. Metrics carry only `surface` and `outcome` (contracts section 4).
