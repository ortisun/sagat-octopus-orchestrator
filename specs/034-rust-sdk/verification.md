# Incremental verification — Rust SDK

The SDK is an unreleased 0.3 candidate. No shared six-language conformance or full worker acceptance
is claimed by these local client checks. All gate logs and JSON exit receipts are retained under
`/tmp/sagat-v03-20261026/` on the shared development machine.

- `rust-sdk-contract-controls-red`: six behavioral failures reproduced: missing required Run fields,
  unsupported operation accepted, empty signal receipt accepted, status/trace lost on malformed JSON,
  PAUSED unrecognized, and an encoded signal path rejected. The following green run passed 11 tests.
- `rust-sdk-start-and-operation-contracts`: 14 REST tests passed. Added immutable UTC deadline,
  version/correlation and exact scoped step operation bodies across retries.
- `rust-sdk-real-tls`: four tests passed using owned TLS listeners, including private CA/name
  verification, mutual TLS, invalid PEM redaction, and bounded plaintext warning fields.
- `rust-sdk-client-full-clippy`: `cargo clippy --manifest-path sdks/rust/Cargo.toml --all-targets
  --locked -- -D warnings` exited 0 (5.34 s).
- `rust-sdk-client-full-tests`: `cargo test --manifest-path sdks/rust/Cargo.toml --locked` exited 0;
  14 REST/catalog checks and four TLS tests passed.
- `rust-sdk-client-fmt`: standalone `cargo fmt --check` exited 0 (0.08 s).
- `rust-sdk-protocol-red`: the real drift test failed because the standalone package lacked
  `proto/worker.proto`. Generation and matching-package validation are the next increment.

The CA/mTLS public API was prototyped before the real TLS tests were added; no before-code TLS
behavioral RED is claimed. Initial reqwest TLS-provider panic and oversized error Clippy findings
are retained in the earlier gate logs and were fixed, not hidden or retried without a change.

REST review cycle 1: eight actionable findings addressed by typed receipts/required fields, all
known statuses, canonical operation catalog/scoped body, StartOptions, metadata-preserving errors,
public reason access, plaintext warning and explicit TLS options. Cycle 2 found eight further actionable interface/boundary issues: five reproduced by new behavioral
RED tests, then 19 REST/catalog checks passed. The cancel version, user-agent suffix and held-body
cancellation cases also passed. Cycle 3 found signal wire-array mismatch, platform failure-code
alphabet, typed child lineage and static-token retry classification; these are addressed with
canonical schema alignment and named tests. The broad REST doubt cycle stops at three; no fourth
review is claimed and full client acceptance still requires the shared real-server scenarios.

Further native gates:
- `rust-sdk-protocol-green`: canonical embedded proto test passed; independent pure-Rust protobuf
  generation succeeds without a backend crate or system protoc.
- `rust-sdk-grpc-errors-red` → `rust-sdk-grpc-errors-green`: two actual mapping failures, then two
  passing closed gRPC metadata/error/fallback-classification cases.
- `rust-sdk-worker-api-red`: missing public host/options imports; this is an API-absence RED.
- `rust-sdk-worker-first-tests`: native gRPC registration, automatic older-server fallback, exact
  original scoped lease reporting and arbitrary-precision output passed. A fixture initially omitted
  additive StepCommandResponse fields and was corrected before this result.
- `rust-sdk-worker-nan-red` → `rust-sdk-worker-nan-green`: nested NaN silently became null, then was
  rejected. `rust-sdk-output-budget-red` → `rust-sdk-output-budget-green`: oversized serialization
  initially consumed the whole value; the bounded writer now stops within the byte budget.
- `rust-sdk-worker-lifecycle-behavior-red` → `rust-sdk-worker-lifecycle-green`: native tests reproduced
  hidden shutdown delay and missing fatal receive-error propagation, then all seven worker cases passed.
- `rust-sdk-secret-capability-red` → `rust-sdk-secret-capability-green`: missing capability API, then
  native concurrent singleflight resolution and retained-view revocation passed.
- `rust-sdk-worker-secret-regressions`: 13 unit/native-gRPC, one protocol drift, 24 REST/catalog,
  four TLS and one worker-config test passed. This is local transport acceptance, not shared runtime parity.
- `rust-sdk-local-token-session-red` → `rust-sdk-local-token-session-green`: a local provider failure
  replaced a valid session (two registrations); the fixed receiver preserves it (one registration).

Worker identity/lifecycle review cycle 1: four retry/shutdown findings addressed; the protocol
finding is resolved by strict accepted-version parsing/bounding and documented diagnostic override.
Cycle 2: local token failure erroneously re-registered; reproduced and fixed. Cycle 3 found server text spoofing private token-provider provenance; reproduced by the native
registration/receive test and fixed with the private provider-failure predicate. The broad lifecycle
cycle stops at three; further protocol cases use smaller named tests.
Secret lifetime independent review cycle 1 found retained authorization and malformed secret cache
acceptance; fixed with take-on-invalidate authorization, zeroizing owned lease-token copies and
canonical name/value/cardinality validation. Three malformed-response tests were seen failing,
then passed. Native retained-context/view tests prove revocation and no retained authorization.
Cycle 2 found an early raw request clone and excessive per-secret RPC decoding budget; fixed with
post-authentication request construction, a revocation check and a 512-KiB response decoder budget.
The pending-provider constructor test and actual oversized response test pass. Cycle 3 identified
the oversized decoder error classification; a focused RED reproduced INTERNAL instead of PROTOCOL,
then the narrow adapter fix retains canonical mapping for ordinary server OUT_OF_RANGE failures.
This adapter recognizes the pinned Tonic 0.14.6 diagnostic; the native transport regression checks
its behavior. No fourth broad review is claimed. No external review is run, following the owner's
explicit preference. Third-party encoded transport/TLS buffers and caller-owned copies are not
claimed to be securely erased by the SDK.

`rust-sdk-secret-fault-boundaries`: 20 unit/native tests passed, including failed-resolution retry
without a permanent cache and handler return revoking a resolution already in flight. Test races
use channel/notification barriers, not sleeps.

`rust-sdk-attempt-foundation-*`: strict Clippy, all 22 unit/native tests, 24 REST/catalog checks,
one canonical proto drift check, four TLS cases, one worker-configuration case and one README
doctest passed; the separate consumer passed with --locked, as did formatting, architecture and
agent configuration. The later decoder-classification change has its own candidate gate receipts.
`rust-sdk-foundation-package`: Cargo package and its verification build passed with --locked and
--allow-dirty for the review candidate; this is not registry publication or final release acceptance.

Candidate checkpoint: `rust-sdk-candidate-clippy` exited 0 (2.13 s),
`rust-sdk-candidate-tests` exited 0 (9.40 s; 23 unit/native, 24 REST/catalog, one drift,
four TLS, one configuration and one README doctest = 54 passed), the standalone consumer
passed with --locked (0.75 s), and formatting passed (0.10 s). Shared real-server conformance,
remaining SDK operations/builders/capabilities, CI and full release gates remain open.

Pagination/journal/wait increment: `rust-sdk-pages-api-red` failed on the absent public API,
then `rust-sdk-pages-first-green` passed 27 REST/catalog checks. `rust-sdk-page-metadata-red`
reproduced loss of HTTP status on a repeated-cursor error (28 passed/one failed); cursor-progress
validation now runs inside the response parser so it preserves status and trace.
`rust-sdk-pages-clippy` exited 0 (2.06 s) and `rust-sdk-pages-tests` exited 0 (12.34 s):
23 unit/native, 29 REST/catalog, one drift, four TLS, one worker configuration and one README
example passed (59 total). Pagination stays lazy/bounded, encodes opaque cursors, preserves raw
fields and stops on cancellation. Completion wait timeout returns TRANSPORT/WAIT_TIMEOUT and
never cancels or changes the run. Remaining T034-002 operations and common conformance stay open.

Declared state operations/event publication increment: `rust-sdk-state-events-api-red` failed on
missing public APIs; `rust-sdk-state-events-first-green` passed 32 REST/catalog checks. Additional
response-shape and before-send validation cases bring the REST/catalog count to 34.
`rust-sdk-state-events-clippy` exited 0 (2.34 s), and `rust-sdk-state-events-tests` exited 0
(12.65 s; 64 total). Queries preserve exact disclosed values/revisions. Updates retry their body
updateKey with immutable arguments/revision and no idempotency header. Event publication freezes
the generated event ID and UTC instant across retries and accepts the canonical duplicate receipt.
Missing result/receipt fields are PROTOCOL errors retaining HTTP status/trace. This increment does
not close the remaining chain, approval, blob, authoring or shared-conformance work.

Approval increment: `rust-sdk-approvals-api-red` failed on missing APIs, then
`rust-sdk-approvals-first-green` passed 36 REST/catalog checks. Added lazy two-page iteration,
required nullable settlement fields, malformed response metadata and local invalid-key checks.
`rust-sdk-approvals-clippy` exited 0 (2.21 s); `rust-sdk-approvals-tests` exited 0
(12.73 s, 68 total including 38 REST/catalog checks). Approval decisions retain their body key and
reason across retries without an idempotency header. Returned scope dimensions accept additive
fields while preserving the canonical typed pair; serialization still emits only that pair.

Chain increment: `rust-sdk-chain-api-red` failed on missing APIs, then
`rust-sdk-chain-first-green` passed 42 REST/catalog checks. Two added cases verify that a stale
head is never silently retargeted and out-of-range segment indices fail with response metadata.
`rust-sdk-chain-clippy` exited 0 (2.63 s); `rust-sdk-chain-tests` exited 0 (12.93 s,
74 total including 44 REST/catalog checks). Commands freeze expectedRunId and idempotency keys;
Queries/Updates keep optional head preconditions and body-key semantics. Inspection/completion
waits use server finalOutcome, and paged segment lineage stays lazy and typed.

Run-input blob increment: `rust-sdk-upload-api-red` failed on missing public APIs, then
`rust-sdk-upload-first-green` passed 47 REST/catalog checks. `rust-sdk-upload-io-boundaries`
passed three owned-server/source cases: 500,000-byte single-pass input with <=256-KiB reads and
precondition headers; a source held before EOF refuses an early descriptor; a failed source is
not replayed and its error text is not exposed. A bounded read-only HTTP blob review found no
concrete material violation; external review remains skipped by owner preference.
`rust-sdk-upload-clippy` exited 0 (4.90 s); `rust-sdk-upload-tests` exited 0
(15.47 s, 80 total); the consumer passed after normal lock resolution for the new pinned SHA-256
and Tokio IO features (3.51 s). Source locks were updated through normal resolution, not regenerated.
Upload performs no automatic retries; startWithBlob sends inputBlobRef without inline input and
shares start version/correlation/deadline/idempotency handling. Worker blob capabilities and shared
real-server blob/fault conformance remain open.

Worker capability batch: `rust-sdk-worker-blob-api-red` failed on the missing API.
Several subsequent compile gates exposed Rust higher-ranked Send/lifetime failures in anonymous
stream composition; preserved logs include first-green, boxed-stream, owned-envelope/source,
send-boundary and separated-auth attempts. The named owned UploadStream resolved the failure;
`rust-sdk-worker-blob-concrete-stream` passed without unsafe. Scoped output, inline lookalikes and
revocation during token refresh passed in `rust-sdk-worker-blob-scope-tests` (26 unit/native).
The bounded write review found one protocol-gating bug: a retained BlobOutput could escape through
1.2 reporting. `rust-sdk-worker-blob-legacy-red` reproduced it; the report gate now fails locally
as permanent UNSUPPORTED_PROTOCOL, and the same test passed in legacy-green.
`rust-sdk-worker-read-blob-api-red` failed on the absent read API; first-green passed the actual
gRPC roundtrip. Reader checks the first header, chunk/size bounds, complete SHA-256 and clean EOF;
corruption and retained-reader revocation are native cases. Reconciliation exposes typed Verdict,
rejects untyped lookalikes and preserves the original effect target; compensation uses the same
lease lifecycle. `rust-sdk-worker-capability-clippy` exited 0 (3.04 s) and
`rust-sdk-worker-capability-tests` exited 0 (15.99 s, 90 total). Shared real-server capability/fault
conformance, observability hooks and full streaming acceptance remain open.

Telemetry batch: `rust-sdk-telemetry-typed-failure-red` reproduced a successful metric emitted
before typed decoding; events now reflect the completed typed operation. Ambient trace hooks,
explicit parent precedence, bounded labels and a panicking worker observer are tested.
`rust-sdk-telemetry-clippy` exited 0 (3.30 s); `rust-sdk-telemetry-tests` exited 0
(16.34 s). These are native REST/gRPC fixture results, not shared real-server conformance.

Builder batch: `rust-sdk-builder-api-red` and shared/dedicated database API RED receipts
failed on absent public constructors. Typed activity/timer/condition/switch, parallel/saga,
named collections/loops, approvals, continuation/state, scripts/triggers and managed database
nodes now use the common IR. The public adapter reads scenario programs, never expected outputs.
`public_builders_match_existing_shared_programs_and_reviewed_goldens` compares all 60 existing
shared authoring fixtures and their canonical texts/errors without blessing. This is authoring
evidence only, not runtime conformance.
One bounded read-only review found four parity/API issues; named RED cases reproduced early
canonical number rejection, empty database error routes and silently dropped root options.
The schema budget reproduction initially had incorrectly sized fixtures (kept as failed setup
receipts); `rust-sdk-builder-schema-boundary-red` then failed on the intended byte-budget defect.
Corrections passed `rust-sdk-builder-review-green`. No second broad review was requested.
`rust-sdk-builder-final-clippy` exited 0 (4.37 s); `rust-sdk-builder-final-tests` exited 0
(17.99 s, 102 total, including the shared 60-fixture authoring comparison).
