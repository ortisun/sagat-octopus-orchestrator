# Managed database and OpenBao recovery

Use with [deployment](openbao.md) and [execution semantics](database-runner.md).
The customer database is an external effect system: restoring Sagat never rolls its
committed writes back. Inspect uncertain writes before authorizing another execution.

## Provider outage, sealing or identity expiry

Check OpenBao health/seal status, TLS expiry/trust, Agent authentication and finite token
TTL, then the exact policy/path and Sagat's current role/ACL. Do not copy a root token
into an application or enable local fallback. Restore quorum, unseal with the approved
custodians, or repair workload authentication. Agent renews/re-authenticates; the clients
reread its token file. A failed query can be started as a new run after recovery. An
unknown write remains `MANUAL_REVIEW` until external evidence establishes its outcome.

Pause a connection to refuse new admissions. Revocation also prevents later capability
resolution. Cancellation can stop a live executor, but neither action retracts a
credential already delivered nor reverses a committed SQL effect. Revoke the target
database login/session through its own administration boundary when necessary.

## Rotate a target credential

1. Arrange the new PostgreSQL credential and required grants at the target. Where the
   target permits it, overlap old/new logins long enough to drain active attempts.
2. Rotate the named secret in Studio with its current version and a fresh request ID.
   Keep that ID for an uncertain response. The API is
   `PUT /v1/namespaces/{id}/secrets/{name}` with `requiredProvider: openbao`.
3. On a lost response, use
   `POST /v1/namespaces/{id}/secret-operations/{requestId}/reconcile` before creating
   another request. An intent without provider bytes explicitly requires secure
   resubmission of the original value; there is no plaintext background retry queue.
4. Probe the saved connection through the managed runner and inspect its durable run.
   New attempts resolve the current authorized credential, while existing attempt pins
   preserve the version needed for completion scanning. Finally retire the old target
   login after its sessions drain. Sagat retains current plus four prior logical versions
   and versions pinned by live/manual-review attempts.

KV rotation stores credentials; it does not change PostgreSQL passwords automatically.
If the replacement is invalid, provision a working credential as **another version**.
Never edit immutable receipts or switch provider authority to conceal a failed rotation.

## Migrate an existing local secret

Keep the original local key ring available during migration. An administrator calls
`POST /v1/namespaces/{id}/secrets/{name}/migrate` with the expected version and request ID.
The server fences that exact source version, preserves the name/ACL and cuts authority
over only after verifying the OpenBao receipt. Concurrent rotation/revocation prevents
stale activation. Retry/reconcile the original request after a lost acknowledgement.

Before activation, the original local authority is unchanged; restore the provider and
finish the same operation. After activation, recovery proceeds through OpenBao and a new
credential version. There is no automatic reverse migration or outage-driven provider
switch. A full rollback to an earlier metadata state requires its **matching coordinated
backup**, while the service is isolated, followed by external-effect reconciliation.

## Replace the intent binding key

`SAGAT_OPENBAO_INTENT_KEY_FILE` contains 32 random raw bytes, loaded at control-plane
startup. This key authenticates provisioning receipts; it does not encrypt OpenBao
values. There is no multi-key rolling transition in this release.

1. Disable secret management/migration writes at ingress and drain requests on **all**
   control-plane replicas. Reconcile all pending receipts using the old key. For intents
   with no provider value, securely resubmit the original value or let the bounded cleanup
   retire them after its 24-hour window. Do not modify HMACs/operation state with SQL.
2. Verify `SELECT count(*) FROM secret_provider_operations WHERE state='pending'` is zero
   using an operator read connection. Take a coordinated backup with the old key. Keep
   that key with that recovery point for its full retention period.
3. Stop every control-plane replica; replace the mounted key atomically with one common
   new value; restart them together and verify all replicas use the new Secret revision.
   Never run replicas with different keys while provisioning is enabled.
4. Reconcile pre-rotation request IDs to inspect their activated metadata. Repeating an
   old PUT body/key under the new binding key returns an idempotency conflict; it does not
   create another version. New writes use fresh request IDs and current expected versions.
   Verify a new write/probe, then reopen management traffic.

An accidentally replaced key fails closed for unresolved old receipts. Restore the old
key to every replica and finish those receipts before trying the procedure again. If the
old key is lost/compromised, isolate management, retire outstanding intents through cleanup
and issue new credential operations; do not pretend the old receipts were verified.

## Coordinated backup and restore

An arbitrary PostgreSQL PITR point plus an unrelated Raft snapshot is **not** a consistent
recovery point. A snapshot must preserve OpenBao's destroyed-version metadata: cleanup
destroys value bytes but deliberately keeps CAS fences against delayed writes forever.

1. Block new admissions and management writes, drain active runners and external report
   producers, and settle provider intents as above while reconciliation/cleanup are still
   available to the operator. Verify pending=0, then stop the API/engine/schedulers/consumers.
   Suspend cleanup and wait for running Jobs/leases/provider requests to finish. Verify no
   other identity is writing this provider prefix. Do not merely stop one replica.
2. With writers stopped, take the platform PostgreSQL backup, all referenced blob content,
   an OpenBao Raft snapshot, and the matching intent/local key material and deployment
   configuration. Record versions, timestamps and SHA-256 digests in a protected manifest.
   Keep unseal shares separate from the encrypted backup and follow your retention policy.
   Use `bao operator raft snapshot save` with a short-lived operator identity and TLS.
3. Restore into an isolated recovery environment with outbound execution disabled. Restore
   PostgreSQL/roles, blob content, matching keys and Raft data from **one manifest**. Normal
   `bao operator raft snapshot restore` verifies seal compatibility. A fresh-cluster restore
   with different initialization keys needs the documented force procedure and the original
   snapshot's unseal material; do not use force to bypass an unexplained mismatch.
4. Unseal and verify quorum, policies, workload auth, credential versions and permanently
   destroyed version metadata. Renew/reissue finite application identities; restoring a
   token record is not a promise that its expiry moved. Verify API metadata, retained
   result digests, request replay and a harmless probe with the restored logical reference.
5. Review every execution that might have affected the customer after the recovery point.
   Keep writes disabled until their effects are reconciled. Restore the broker from its
   own documented procedure; database outbox/reconciliation tolerates lost/duplicate hints.
   Resume the engine, readers, then authorized writers and finally cleanup.

The automated Compose drill erases/recreates the platform database and removes actual
credential metadata, restores a quiesced PostgreSQL dump and same-cluster Raft snapshot,
restarts/unseals OpenBao, and proves retained result/replay/new execution and a permanent
retirement fence. It does not claim cross-region PITR, recovery of missing unseal shares,
fresh-cluster force restore or rollback of customer effects. The separate production
fixture proves three-voter TLS Raft and workload authentication/network isolation.

## Evidence and reproduction

```sh
python3 tests/e2e/database-connections/compose.py
cargo test -p sagat-tests --test integration_database_connections --locked
cargo build -p sagat-database-runner --bin database-runner --locked
cargo test -p sagat-tests --test integration_database_runner --locked
```

The separate-process crash test kills the runner after target commit and before engine
completion, expires its real lease, restarts it and verifies `MANUAL_REVIEW`, one attempt
and one external effect. See [verification](../../specs/017-database-connections/verification.md).

Provider semantics: [Raft snapshot API](https://openbao.org/docs/api/system/storage/raft/),
[Raft operator commands](https://openbao.org/docs/commands/operator/raft/) and
[storage backup constraints](https://openbao.org/docs/concepts/storage/).
