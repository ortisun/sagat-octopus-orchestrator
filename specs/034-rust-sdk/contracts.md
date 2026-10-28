# Native Rust SDK contracts

The public package is sagat-octopus-orchestrator-sdk, with an independent Cargo workspace and lock.
It embeds checked canonical worker.proto/error-catalog.json and contains no backend crate path
references. Release candidate0.3 starts at the repository-tested Rust1.98 baseline.

REST uses ClientOptions (URL, namespace, token provider, timeout/retry, bounded body and TLS),
CallOptions (idempotency/trace/cancellation/deadline of one call), StartOptions (immutable version,
correlation and UTC/duration deadline) and OperationOptions (exact step/scope/outcome/revision).
Run/receipts/pages expose required typed fields plus raw additive JSON. A successful HTTP code alone
cannot accept an invalid/missing body. Errors preserve received HTTP status/valid trace identity,
closed code/retryability, reason and remediation, without credential/transport-body Debug output.

Token providers run on every attempt; only private local provider failures get the before-send
retry exception. Server-provided reasons cannot grant it. A retry freezes body, idempotency/event
identity and duration-derived deadline. Redirects are never followed. Configurations never offer
an insecure certificate/hostname toggle. Native trust, extra roots and mTLS stay explicit.

The planned WorkerHost accepts typed async activity/compensation/reconciliation handlers. Start
returns after registration; stop cancels receiving then drains, with cooperative handler tokens
and immutable per-registration lease/report identity. Only the server decides stale leases,
transitions, workflow retries and compensation. Every RPC refreshes credentials; normal workers
offer protocol1.5 and forward scoped identities without flattening or inventing them. Redis/NATS
internal notification selection does not change the worker's gRPC protocol.

Builders are side-effect-free declarative IR constructors, with shared canonical authoring cases.
Actual runtime/fault/golden parity and packaged clean consumer checks remain release acceptance,
not guarantees supplied by this contract or the initial REST foundation.

Attempt secrets use a lazy successful singleflight cache; malformed names, empty/oversized values,
more than 16 secrets, duplicate names or zero versions are refused before caching. ResolveSecrets
has a separate 512-KiB decode budget. Ended contexts retain a tombstone, not the authorization slot;
revocation cancels pending resolution and clears zeroizing values. Caller-owned secret copies and
third-party encoded transport/TLS buffers have their own lifetime. Request construction follows
fresh token authentication and a new active-capability check.

Workflow identifiers accept UUIDs or namespace-local names, resolved through the public workflow
pages; the name cache belongs to the configured client. ActivityContext.steps exposes retained
business outputs rather than wire envelopes. stop(grace) returns normally after cancelling tokens
when grace expires; an unfinished lease still lapses under server authority. Absent gRPC trace
metadata serializes as an empty string, following the existing shared error contract.
