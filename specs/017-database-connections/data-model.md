# Data model — feature 017

## Secret provider lifecycle

`secrets` remains the namespace/name identity and owns its ACL. Its `authority` is `local` or `openbao`. Active version pointers never refer to a pending provider write.

`secret_versions` has exactly one representation: a non-null local envelope, or a non-null immutable `provider_operation` reference with all envelope columns null. Remote reads derive tenant/namespace/operation from persisted metadata, never a client path. Provider identity and version are distinct from encryption-key identity; remote records have no local key.

`secret_provider_operations` holds UUID identity, tenant/namespace, logical secret identity/name, actor, client request UUID, expected version, 32-byte keyed content binding, state and timestamps. The unique receipt scope is tenant/namespace/actor/request. One pending operation per name fences concurrent management and migration. An operation is prepared and audited in one transaction; OpenBao I/O occurs after commit; activation locks and revalidates current membership, namespace/name identity and expected revision in another transaction. Revocation or a competing edit prevents activation. A replay with different content is a typed conflict. A pending operation without a provider value needs secure resubmission, never background plaintext recovery.

Provider objects not referenced by retained secret versions are garbage after the receipt window. Keep idempotency receipts for 24 hours, then cleanup may remove unreferenced objects and receipts. Pending intents expire after 24 hours with an audited abandonment. The cleanup order is provider deletion first, then metadata deletion; failures leave retryable metadata. A hard bound of 1,024 receipts per namespace prevents unlimited metadata growth during a provider/cleanup outage. The existing secret-count quota and the 32 pending-operation ceiling apply at preparation and activation. Current and four previous logical versions are retained. Remote secret byte length is accounted even though the plaintext is held by OpenBao; it must not become a retained-content quota bypass.

Logical deletion immediately removes resolution/ACL authority and fences pending activation. It does not promise synchronous destruction of provider bytes; retention cleanup performs that bounded, recoverable external action. No remote data can be restored as authoritative merely by replaying a deleted receipt.

## Connection and execution records

Connection identities, immutable revisions, current enabled/tombstoned state and per-run node bindings are separate records. A binding retains a revision while a run can use it; changing the head never retargets a retry. Provider credential version is resolved independently at each permitted attempt. Later implementation slices add exact migration/contract fields here with their tests.

Implementation status: the intent/version schema, immutable content-binding guard, byte-accounting trigger, explicit migration fence and provider-aware resolution have real-service evidence. Scheduled provider-object/receipt cleanup is still pending T017-024; current hard bounds fail closed when full. The feature is not enabled by production configuration or management routes yet.
