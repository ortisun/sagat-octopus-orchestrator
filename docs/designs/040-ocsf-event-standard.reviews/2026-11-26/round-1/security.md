# Security review of design 040, round 1 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: e5bda0415ca8eddee6135047ce6d4d4366aae3dc
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 11, Nit 7

1. Major — The tenant sink query does not exclude `access.denied.cross_tenant`
   - Where: design section 4 (lines 301-307), Requirement 040-009 (105-108), appendix row (540), Success criterion 040-004 (143-144). Checked in code.
   - What is wrong: the export SQL filters only on `tenant_id`. The exclusion appears only as an appendix note ("operator sink only"). A cross-tenant denial is written in the caller's tenant and carries the other tenant's identifier (`crates/application/src/authz.rs:59-71`). `GET /audit` excludes it in SQL with `NOT (action = ANY($10))` (`crates/infra-postgres/src/operations.rs:44`). A builder who follows section 4, or filters after the `LIMIT`, gives the tenant two things. First, the denial event itself, which tells tenant A that an identifier exists in tenant B. Second, a cursor positioned on a record A must not see.
   - Correction: put `AND NOT (action = ANY(WITHHELD_FROM_TENANT))` in the tenant-sink SQL in section 4. The cursor should advance only over records that were delivered. Add a fixture to Success criterion 040-004 where tenant A probes a namespace of tenant B, and assert that no event, cursor or count shows it.

2. Major — The cursor gives a tenant a view of activity across the whole cluster
   - Where: Terms (39-40), CUJ 1 ("the sink shows its cursor and lag", 216-217), section 4 start cursor (311-313), section 6 Studio and runbook (353-355).
   - What is wrong: `tx_id` is a transaction counter shared by the whole cluster. Showing `cursor_tx` to a tenant administrator lets them measure the write rate of every tenant. Two ways: create a sink twice and read the two `pg_snapshot_xmin` values, or read the gaps between their own consecutive records. The lag also shows when another tenant's long transaction is holding `xmin` back. On a deployment with few tenants, this tells A how busy B is.
   - Correction: never expose `tx_id`, the start cursor or `xmin` through the API, Studio or OCSF. Show the position as the id and `created_at` of the last delivered record. Compute lag from the tenant's own oldest undelivered record. Make the runbook's "replay from a start time" an operation that takes a time, not a cursor.

3. Major — A reveal grant covers one namespace, but the sink's trail covers the whole tenant
   - Where: Requirement 040-010 (109-113), section 5 step 1 (323-324). Checked in code.
   - What is wrong: `payload_reveal_grants` is keyed on `(tenant_id, namespace_id, subject)` (`crates/infra-postgres/migrations/20261202000000_reveal_grants.up.sql:7-13`). `audit_records` has no namespace column (`crates/infra-postgres/migrations/20261008000000_identity.up.sql:55-66`). "The creator's reveal grant" can therefore only mean the home namespace's grant. That grant would release `pii` reasons from every namespace. The existing rule for data that spans the tenant requires the grant in every namespace (`crates/infra-postgres/src/reveal_grants.rs:69-70`, D041-133). The effective check also needs the current role (`role_can_use_reveal_grant`) and the shared revocation fence (`reveal_grants.rs:40-67`). The design names neither.
   - Correction: allow a `pii` ceiling only when `has_effective_payload_reveal_grant` holds in every namespace of the tenant, as the step-log quota rule does. Recheck it the same way per batch. State the bounded window explicitly: at most one batch already being sent at revocation can still leave. Run the recheck again in the cursor-commit transaction, so the next batch does not go.

4. Major — Only the reveal grant is rechecked; the rest of the sink's authority is checked once, at creation
   - Where: Requirement 040-008 (98-104), section 5 (317-324).
   - What is wrong: suppose the creator loses `audit:sink:manage` or `audit:read`, is removed from the tenant, or loses `secret:use` on either secret. An `internal` sink still sends the whole trail to an endpoint they chose, and the engine keeps resolving namespace secrets on their behalf indefinitely. Authorization does not fail closed. Schedules already handle the same case with `schedule.auto_paused` when the authorizing subject loses access (`crates/domain/src/audit.rs:228-230`). `audit:sink:manage` also amounts to `audit:read` without requiring it.
   - Correction: record an authorizing subject. Per batch, recheck their current membership, `audit:sink:manage`, `audit:read` and the right to use both secrets. On failure, pause with a closed state such as `PAUSED_AUTHORITY_LOST`, audited. State in the permission's documentation that it "needs `audit:read` as well".

5. Major — Changing or resuming a `pii` sink does not re-establish who authorized it
   - Where: section 6 PATCH, `:resume` (342-344); Requirement 040-010 (110-112); D1617 ("a destination whose revision declares a ceiling").
   - What is wrong: admin B has `audit:sink:manage` but no reveal grant. B can PATCH admin A's `pii` sink to B's own URL, namespace or secrets. B can also resume a `PAUSED_GRANT_REVOKED` sink. The grant check stays on A's grant, so `pii` flows to B's destination. The rule sets the ceiling per destination revision.
   - Correction: any change to url, namespace, secrets or ceiling creates a new revision. The editor becomes the authorizing subject and must pass the `pii` check from finding 3, or the ceiling drops to `internal`. A resume from `PAUSED_GRANT_REVOKED` requires the grant check to pass for the subject who resumes. Add a test for each case.

6. Major — The operator sink is a database row that the runtime role can write
   - Where: section 5 ("a row with `tenant_id` null created from configuration at start", 317-321); Requirement 040-011.
   - What is wrong: the only stream that crosses tenants is directed by a row in `audit_sinks`. `sagat_app` must be able to INSERT and UPDATE that table for tenant sinks. So "only the deployment configures it" is enforced by application code, not by the data model. Two concrete failures:
     - A tenant PATCH, pause or resume path that updates by `id` without the tenant predicate can redirect or pause the stream of every tenant.
     - Engine replicas with different configuration (for example during a rolling deploy) overwrite each other's URL at start.
   - Correction: keep the operator sink's URL and secrets in memory from configuration only. Persist only its cursor, in a separate one-row table with no destination columns. Add `CHECK (tenant_id IS NOT NULL)` to `audit_sinks`.

7. Major — Policy for the operator sink is not decided, and the Constitution Check marks Principle V "Pass" without discussing it
   - Where: Requirement 040-011 (114-117), Requirement 040-010, Constitution Check V (385).
   - What is wrong, four undecided points:
     - Which ceiling applies? Requirement 040-010 ties `pii` to an administrator's reveal grant, which the operator cannot hold. The builder cannot tell whether reasons of every tenant leave.
     - Which egress control applies? `OutboundRequest` carries a tenant and namespace policy, and the operator has neither.
     - Where do the operator sink's own `audit.sink_delivered`, `audit.sink_stalled` and `secret.resolved` records go? `audit_records.tenant_id` is NOT NULL. Writing them into each tenant's trail tells every tenant about the operator's monitoring.
     - Principle V says "SQL, messages and audit retain tenant scope", yet one batch mixes tenants. The constitution keeps every security exception with the owner, not the delegate (`docs/constitution.md:305-309`).
   - Correction:
     - Decide the operator ceiling. Recommended: `internal`, no reason.
     - Add an egress allowlist at deployment level, still enforced by `crates/infra-egress`.
     - Write operator-sink records to the operator's logs and metrics, not to tenant trails.
     - Make each batch hold one tenant only, reading by tenant within the global cursor or with per-tenant cursors, so messages keep tenant scope. Or record an owner row that accepts the exception.

8. Major — The signing key and the collector token are ordinary namespace secrets, so the people who can obtain them can forge security events
   - Where: Requirement 040-008 (99-103), section 5 step 2 (325-332).
   - What is wrong: any namespace secret can be delivered to a task that declares it, under `secret:use` (Principle V). It can also be the signing secret of a webhook subscription, and then its receiver holds it. Whoever obtains the token can post to the SIEM collector directly. Most collectors authenticate only by the header and ignore `x-octopus-signature`. Whoever obtains the signing key can sign OCSF batches that a verifying receiver accepts. The integrity of the audit stream is the purpose of the feature. The label does not help against someone who holds the key.
   - Correction: bind both secrets to this purpose. Either the worker delivery path, webhook subscriptions and event endpoints refuse a secret marked `audit-export`, or the platform generates the signing key and shows it once. Test that a workflow declaring the secret is refused.

9. Major — The signature contract contradicts itself and is not fully specified
   - Where: section 5 step 2 (326-329); D040-008 in `specs/040-ocsf-event-standard/research.md:17`; `crates/application/src/signing.rs:41-57,105-131`.
   - What is wrong:
     - The label is the first line of the MAC (`Direction::label`). With `audit-export`, a receiver that verifies webhook deliveries (`octopus-event-out-v1`) cannot verify a batch. D040-008's "a receiver that verifies one verifies the other" is false.
     - `event_type` `ocsf.batch` is already covered by the MAC.
     - The label is unversioned, unlike the existing labels.
     - The design does not say whether the signature covers the gzip bytes or the decompressed body.
     - Webhooks sign with the current and previous secret versions (`crates/scheduler/src/deliveries.rs:33-34`). The design resolves "the" secret, so receivers break during a rotation.
   - Correction: add `Direction::AuditExport` with the label `octopus-audit-export-v1`, and publish known vectors in the contract. Remove the "verifies the other" claim. Sign the bytes on the wire, compressed, so a receiver verifies before inflating. Sign with the current and previous versions, as `deliveries.rs` does.

10. Major — Required OCSF fields exist only in the reason, which the design says to leave out
    - Where: section 2 table, User Management row (274); appendix UM rows (508-510); Requirements 040-005 and 040-006. Checked in code.
    - What is wrong: for `secret.access_granted` and `secret.access_revoked`, the resource id is the secret name, and the grantee exists only in the reason, as `subject {subject}` (`crates/application/src/secrets.rs:815-827`). For `membership.granted`, the role exists only in the reason, as `(role X)` (`crates/infra-postgres/src/identity.rs:162-165`). Filling `user` or Assign Roles therefore means parsing the reason. That copies reason content into an `internal` event, bypassing the omission rule, or else fills `user.uid` with a secret name.
    - Correction: decide per action which structured fields can be derived without parsing the reason. Add those to the appendix's closed `unmapped` keys with a classification. For the rest, state what fills the required object (for example the secret as `resources`, with `user` absent), and validate against the schema.

10. Major — A logical restore into another cluster silently stops every stream
    - Where: section 4 (292-313); Requirement 040-009 ("MUST NOT skip records").
    - What is wrong: the runbook documents `pg_dump` and `pg_restore` (`docs/runbooks/control-plane.md:217-223`). In a fresh cluster, transaction ids restart well below the restored `cursor_tx`. New records sort below every cursor and are never sent. Restored rows with high `tx_id` stay above `xmin` for a long time. Requests succeed with empty batches, so the sink never becomes `STALLED`. The security teams' SIEMs go blind with no signal.
    - Correction: at the start of each pass, if `pg_current_xact_id() < cursor_tx`, mark the sink `CURSOR_INVALID`, audit it, raise an alert, and resume by `created_at` from the last delivered record. Add a step to the restore runbook. Add a test that restores into a fresh cluster.

12. Nit — The runtime role can set `tx_id` itself
    - Where: section 4 and Task 040-9 (446); `crates/infra-postgres/migrations/20261008000200_runtime_role_grants.up.sql:20` (table-wide INSERT).
    - What is wrong: an INSERT that names `tx_id` can hide a record below a cursor, or park it above `xmin` forever.
    - Correction: grant INSERT on `audit_records` by column, leaving out `tx_id`. This costs nothing at write time and adds no trigger (Non-functional requirement 040-001).

13. Nit — Secret resolution and its audit are misnamed, and the audit feeds itself
    - Where: section 5 step 2 (326-327).
    - What is wrong: "Feature 009's resolution" is the worker path, bound to a lease, and the engine has no lease. Webhooks read namespace secrets through `SecretReader` (`deliveries.rs:531-543`). `secret.resolved` means "a worker received the value" (`audit.rs:120-121`) and collapses for good (`audit.rs:271,296-311`). Separately, `audit.sink_delivered` is itself streamed, which produces one record per sink per minute indefinitely.
    - Correction: name the real reader. Use a distinct, collapsed action for the engine's use of a secret. Either exclude `audit.sink_delivered` from the sink that wrote it, or accept the heartbeat explicitly.

14. Nit — Error and oracle behaviour of the sink API is not stated
    - Where: section 6 (342-345).
    - What is wrong, three gaps:
      - Nothing says that a `sinkId` of another tenant, or the operator row, answers 404 with the same body and timing, audited as a cross-tenant denial. The design adds no `ResourceType` for a sink.
      - Nothing stops create from distinguishing "secret absent" from "secret not usable".
      - Nothing says what `:test` returns. Egress is guarded: `crates/infra-egress` refuses private addresses in the resolver and refuses userinfo, proxies and redirects, so this is not an SSRF. Still, the response must not echo the remote body.
    - Correction:
      - Answer 404 the same way as a missing sink, and add `ResourceType::AuditSink`.
      - Return one error code for an absent secret and for one the caller cannot use.
      - Have `:test` return a closed outcome only (`ok`, `denied`, `timeout`, `http_<class>`), rate-limit it and audit it.

15. Nit — The URL can itself be a credential
    - Where: Requirement 040-008, section 5 and section 6 (99-101, 317, 352).
    - What is wrong: some collectors put the token in the path (for example Sumo Logic HTTP sources), or in the query. Spans already leave out the path, but the API response, Studio and the audit reasons of `audit.sink_*` are not addressed.
    - Correction: state that the URL never appears in audit reasons or logs. Return only `scheme://host` plus a fingerprint of the path. Either refuse a query string or treat the path as secret-class.

16. Nit — No per-event size bound, and records are never dead-lettered
    - Where: section 5 (325), CUJ 2 (225-226).
    - What is wrong: `resource_id` is unbounded `text`, and only the reason is cut to 2048 characters by `scrub` (`crates/telemetry/src/redact.rs:16`). One event over 1 MiB could never be sent. Because nothing is dead-lettered, it would stall that sink for good, and if it reached the operator sink it would stall the stream of every tenant. That a caller can actually produce such a record is a suspicion I did not prove.
    - Correction: cap every string attribute, truncate with a marker, and test an oversized record.

17. Nit — Smaller disclosure, race and noisy-neighbour items
    - `metadata.product.version` gives tenants and their SIEMs the binary's patch level, which the native read does not (Requirement 040-005).
    - `trace.uid` may be chosen by the caller (`control-plane.md:212-213`, G15), so it can make unrelated events, including other tenants' in the operator's SIEM, appear correlated. Mark it as caller-influenced in the reference.
    - The limit of 4 sinks per tenant is check-then-act. Count under a lock on the tenant row.
    - One tenant's long transaction delays every tenant's sinks and the operator sink. Say so in Risks.

No finding, after looking: metric labels (Non-functional requirement 040-004); `GET /audit/ocsf` reusing `READ_AUDIT_SQL` with its tenant predicate and exclusion; egress transport through `crates/infra-egress/src/lib.rs`; the ordering guarantee of the cursor below `xmin` within one cluster; no SECURITY DEFINER functions are introduced.
