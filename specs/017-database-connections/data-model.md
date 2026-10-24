# Data model — feature 017

## Secret provider lifecycle

`secrets` remains the namespace/name identity and owns its ACL. Its `authority` is `local` or `openbao`. Active version pointers never refer to a pending provider write.

`secret_versions` has exactly one representation: a non-null local envelope, or a non-null immutable `provider_operation` reference with all envelope columns null. Remote reads derive tenant/namespace/operation from persisted metadata, never a client path. Provider identity and version are distinct from encryption-key identity; remote records have no local key.

`secret_provider_operations` holds UUID identity, tenant/namespace, logical secret identity/name, actor, client request UUID, expected version, 32-byte keyed content binding, state and timestamps. The unique receipt scope is tenant/namespace/actor/request. One pending operation per name fences concurrent management and migration. An operation is prepared and audited in one transaction; OpenBao I/O occurs after commit; activation locks and revalidates current membership, namespace/name identity and expected revision in another transaction. Revocation or a competing edit prevents activation. A replay with different content is a typed conflict. A pending operation without a provider value needs secure resubmission, never background plaintext recovery.

Provider objects with no retained logical-version or attempt-credential reference become eligible 24 hours after creation/activation. Cleanup commits abandonment and an audited 90-second UUID claim under the namespace secret lock before provider I/O. It destroys provider value bytes while retaining permanent CAS tombstone metadata, then conditionally deletes the matching unexpired PostgreSQL receipt and refunds bytes. Failed or interrupted cleanup retains a retryable receipt. Late provisioning cannot recreate the retired provider path. Terminal run pins remain for 24 hours after completion; live/manual-review pins remain indefinitely. Pin release takes one run lock and then its namespace secret lock per transaction. OpenBao tombstone metadata accumulates separately and must be included in provider backups. A hard bound of 1,024 receipts per namespace prevents unlimited metadata growth during a provider/cleanup outage. The existing secret-count quota and current pending-operation policy (default 32, hard maximum 128) apply to new reservations; existing reservations can settle after a policy reduction. Current and four previous logical versions are retained. Remote secret byte length is accounted even though the plaintext is held by OpenBao; it must not become a retained-content quota bypass.

Logical deletion immediately removes resolution/ACL authority and fences pending activation. It does not promise synchronous destruction of provider bytes; retention cleanup performs that bounded, recoverable external action. No remote data can be restored as authoritative merely by replaying a deleted receipt.

## Connection and execution records

Connection identities, immutable revisions, current enabled/tombstoned state and per-run node bindings are separate records. A binding retains a revision while a run can use it; changing the head never retargets a retry. Provider credential version is resolved independently at each permitted attempt. `database_connections` stores tenant/namespace/name identity, current revision and ACTIVE/PAUSED/DELETED authority. `database_connection_revisions` stores configuration JSON, requested status, actor/request UUID and timestamp. A unique tenant/namespace/actor/request receipt binds the exact settings and expected revision. Updates are forbidden by an ALWAYS trigger; a deferred composite FK ensures every head references a committed revision. Identity/name cannot change and tombstones cannot be revived.

`run_database_connections` has one immutable tenant/namespace/run/node → connection/revision binding with tenant-and-namespace composite FKs. Publication validates every connection against the author's current role and secret ACL. Run admission now populates bindings in the run transaction, after acquiring any parent reservation and before commit. Current connection state, OpenBao authority, publisher and initiating actor permissions/ACLs are checked under locks. Triggered trees use their publisher when there is no human initiator; `system:engine` grants no database authority. Receipt replay validates the persisted pins and current authority. A new rerun resolves floating references at its own admission; an explicit revision remains explicit. Settings/names are charged through existing ALWAYS retained-byte triggers. The catalog defaults to 100 current connections (active or paused), governed by the existing namespace/tenant quota policies and operator grants, with a hard maximum of 1,000. It also enforces 1,024 retained identities per namespace and 1,000 ordinary revisions per identity; a final tombstone revision is allowed at that revision ceiling. Referenced connection history is retained.

Lock order for admission is an existing parent/root run reservation (for children), then namespace secret/catalog lock, current membership rows, namespace, connection heads, secret rows and existing ACL rows. Revision selection, run/pin insertion, initial journal/task/outbox records and receipt commit together. ACL removal and role changes wait for an admission already holding their share locks; subsequent admissions see the revocation. A crash before commit leaves no run or pin. A denied child admission completes the parent step with `DATABASE_UNAVAILABLE`; a denied event/cron admission creates no run and records a bounded `database.denied` audit. Dedicated managed leases/capability use recheck the same current authority under their fenced attempt (T017-013).

A catalog write takes the existing namespace secret-management lock, checks the current membership and OpenBao authority, then commits revision/head/audit together. Creates and rotations cannot race the count or head. Historical reads show historical settings with current lifecycle status. Catalog writes/reads are metadata operations. Probe routes admit a fixed durable execution and expose its status/revision; no API route executes customer SQL.

The intent/version schema, immutable content-binding guard, byte-accounting trigger, explicit migration fence and provider-aware resolution have real-service evidence. Bounded cleanup is available as an operator command and scoped CronJob. Coordinated PostgreSQL/Raft restoration preserves values and permanent CAS fences; see [verification](verification.md) and the [recovery procedure](../../docs/runbooks/database-recovery.md).

## Private execution definitions

`workflows.internal` is immutable and implies no active version. Private retained definitions have
an immutable index keyed by `(tenant_id, namespace_id, actor, digest)`, with scoped foreign keys
to their workflow and version. Their publisher is the indexed actor. Definition insertion/reuse
and durable run admission share one transaction and the existing workflow quota lock. Definition
bytes remain subject to the existing retained-content triggers. Ordinary definition and promotion
queries exclude these records; runtime and owner-authorized rerun use private access.

`runs.origin` is nullable for existing/public workflows and otherwise one of `AD_HOC_STEP`,
`AD_HOC_WORKFLOW`, `CONNECTION_PROBE`. The origin is immutable and must agree with a private
definition owned by the initiator. Definition deduplication excludes origin, but receipt identity
includes origin, so submitting an identical operation as ad hoc cannot relabel a probe run.

## Attempt credential evidence

Migration `20261029000000_database_executor` introduces `step_database_credentials`, keyed by
`(tenant_id,run_id,node_id,attempt,generation)`. Its immutable row contains namespace, provider
operation, logical secret name/version and timestamp. Scoped foreign keys tie it to the run,
step and provider operation. There is deliberately no FK to the logical secret/version: logical
deletion revokes future capability resolution immediately, while the provider evidence remains
available to output scanning and retention. It contains no credential bytes. The provider operation
continues to account for those retained external bytes.

`database-runner` is a separate machine membership role. Downgrade refuses live executor identities
or credential pins instead of silently deleting authority/evidence. The migration grants the
application SELECT/INSERT/DELETE on pins and rejects UPDATE with an ALWAYS trigger.
