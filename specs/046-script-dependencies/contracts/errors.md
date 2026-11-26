# Errors (design 046)

All answers use the platform's error envelope (`code`, `retryable`, `traceId`, `details`), never a registry response, a
resolver's words in a response, or a secret.

## Where each refusal appears

- **Author lock:** the publication request itself answers `422` with the first reason below (or `503
  PACKAGE_GATEWAY_UNAVAILABLE`, retryable, when the metadata call fails, or `503 SCRIPT_POLICY_BUSY`, retryable, when
  the policy inputs changed under five evaluations in a row).
- **Platform resolution:** publication answers `202` with the version `BUILDING`; a refusal later makes the version
  `FAILED` with the reason in its build record, a `script.supply_chain_refused` audit record and an event. Reasons 3 to
  13 appear on this path when the resolved lock breaks them; the gateway's cut and wheel filter mean that a manifest
  that asks only for quarantined versions or source distributions fails earlier with `RESOLUTION_FAILED`.

## Refusal reasons, in their order

The first that applies is the code; `details.findings` lists every finding (kind, package, version, advisory,
severity), naming packages and versions, never file content.

| Order | Code | When |
|---|---|---|
| 1 | `DEPENDENCY_MANIFEST_INVALID` | the manifest breaks `lock-formats.md` |
| 2 | `LOCKFILE_INVALID` | the lock breaks `lock-formats.md` (`details.rule`) |
| 3 | `PACKAGE_PLATFORM_UNSUPPORTED` | a required target is not served |
| 4 | `PACKAGE_SOURCE_ONLY` | a Python package has no usable wheel |
| 5 | `PACKAGE_MALICIOUS` | a malicious-package advisory matches |
| 6 | `PACKAGE_NOT_ALLOWED` | a reserved name from a public source, a deny rule, an allow list, a disabled ecosystem (`details.rule`) |
| 7 | `PACKAGE_QUARANTINED` | younger than the namespace's quarantine at evaluation (an author lock) |
| 8 | `PACKAGE_VULNERABLE` | an advisory at or above the threshold, or unrated with `blockUnrated`, without a live waiver |
| 9 | `PACKAGE_LICENCE_DENIED` | a denied licence |
| 10 | `PACKAGE_YANKED` | a yanked or deprecated version without a live waiver |
| 11 | `PACKAGE_INSTALL_SCRIPT_REFUSED` | an install script with `installScripts: refuse` |
| 12 | `PACKAGE_METADATA_MISMATCH` | the install scripts the unpacker saw differ from the gateway's metadata |
| 13 | `DEPENDENCY_LAYER_TOO_LARGE` | a layer over `maxLayerBytes` or the file bound |
| 14 | `PACKAGE_FETCH_BUDGET_EXCEEDED` | the tenant's daily fetch budget would be passed |

`SCRIPT_DEPENDENCIES_UNAVAILABLE` (`422`) answers a manifest or a package-bearing lock while `script-dependencies-v1` is
not activated.

## Job failures (kind `resolve`, `install` or `backfill`)

| Code | Retryable | When |
|---|---|---|
| `RESOLUTION_FAILED` | no | the resolver failed, or a channel budget was passed |
| `PACKAGE_GATEWAY_UNAVAILABLE` | yes | a forwarded request was refused, timed out, or answered `502`, `503` or `504` |
| `PACKAGE_FILE_UNAVAILABLE` | yes | an install fetch failed in transport or got a file whose hash differs from the lock |
| `INSTALL_FAILED` | no | the unpacker refused an archive (`details.rule`, including a name or version that differs from the lock entry) or the verify step's digest did not match |
| `LAYER_DIGEST_MISMATCH` | no | a backfill's layer digest differs from the record already signed for that (artifact, target); an alert fires |

A retryable failure re-queues the job with `not_before` (15 s times the attempt, feature 005's rule); after the last
attempt the version is `FAILED` (a backfill job only ends `FAILED`; its `READY` version is untouched).

## Run failures (a script step)

| Code | Reason | Retryable | When |
|---|---|---|---|
| `SCRIPT_DENIED` | `PACKAGE_BLOCKED` | no | a block matches the artifact and the namespace, at lease, at materialization or before start |
| `SCRIPT_UNTRUSTED_ARTIFACT` | `LAYER_DIGEST_MISMATCH`, `LOCK_DIGEST_MISMATCH`, `LAYER_RECORD_INVALID` | no | the layer, the served lock or the record does not match |
| `SCRIPT_SANDBOX_UNAVAILABLE` | `LAYER_UNAVAILABLE` | no | three distinct runner principals failed to materialize the layer, or no registered target has a layer record for 10 minutes (the control plane fails the ready steps) |
| `SCRIPT_SANDBOX_UNAVAILABLE` | an old runner's unknown bundle version | yes | a rolled-back runner |

## Other

`404` answers alike for another tenant's artifact, file, reservation or upload and for a missing one, and the attempt is
audited. `409 PACKAGE_VERSION_EXISTS` answers an upload of a different file for an uploaded version. `409
PACKAGE_RELEASE_BY_UPLOADER` answers an early release asked by the upload's own uploader. `409
POLICY_REVISION_CONFLICT` answers a policy write with a stale expected revision. `409 ADVISORY_IMPORT_HELD` answers an
import that waits for an operator's confirmation.
