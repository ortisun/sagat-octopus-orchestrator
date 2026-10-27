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
