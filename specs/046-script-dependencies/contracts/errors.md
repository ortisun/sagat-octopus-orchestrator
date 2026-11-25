# Errors (design 046)

All answers use the platform's error envelope (`code`, `retryable`, `traceId`, `details`), never a registry response,
a resolver's words in a response, or a secret.

## Publication refusals: `422`, `retryable: false`

Checked in this order; the first that applies is the code, and `details.findings` lists every finding of the lock (kind,
package, version, advisory identifier, severity). `details` names the package and version, never its file content.

| Order | Code | When |
|---|---|---|
| 1 | `DEPENDENCY_MANIFEST_INVALID` | the manifest breaks `lock-formats.md` |
| 2 | `LOCKFILE_INVALID` | the lock breaks `lock-formats.md` (`details.rule` names the rule) |
| 3 | `PACKAGE_PLATFORM_UNSUPPORTED` | a required target is not served |
| 4 | `PACKAGE_SOURCE_ONLY` | a Python package has no usable wheel |
| 5 | `PACKAGE_MALICIOUS` | a malicious-package advisory matches |
| 6 | `PACKAGE_NOT_ALLOWED` | a deny rule, an allow list, a disabled ecosystem, or a reserved name taken from a public source (`details.rule`) |
| 7 | `PACKAGE_QUARANTINED` | younger than the namespace's quarantine |
| 8 | `PACKAGE_VULNERABLE` | an advisory at or above the threshold, or unrated with `blockUnrated`, without a live waiver |
| 9 | `PACKAGE_LICENCE_DENIED` | a denied licence |
| 10 | `PACKAGE_YANKED` | a yanked or deprecated version without a live waiver |
| 11 | `PACKAGE_INSTALL_SCRIPT_REFUSED` | an install script with `installScripts: refuse` |
| 12 | `PACKAGE_METADATA_MISMATCH` | the install scripts the unpacker saw differ from the gateway's metadata |
| 13 | `DEPENDENCY_LAYER_TOO_LARGE` | a layer over `maxLayerBytes` or the file bound |

`SCRIPT_DEPENDENCIES_UNAVAILABLE` (`422`) answers a manifest or a package-bearing lock while the capability
`script-dependencies-v1` is not activated.

## Build failures (the version's build record, kind `resolve` or `install`)

| Code | Retryable | When |
|---|---|---|
| `RESOLUTION_FAILED` | no | the resolver failed (a missing package, a conflict) or a channel bound was passed |
| `PACKAGE_GATEWAY_UNAVAILABLE` | yes | the gateway refused the connection, timed out, or answered `502`, `503` or `504` |
| `PACKAGE_FETCH_BUDGET_EXCEEDED` | yes (after the day turns) | the tenant's daily fetch budget is spent |
| `INSTALL_FAILED` | no | the unpacker refused an archive (`details.rule`) |

## Run failures (a script step)

| Code | Reason | Retryable | When |
|---|---|---|---|
| `SCRIPT_DENIED` | `PACKAGE_BLOCKED` | no | a block matches the artifact and the namespace, at lease |
| `SCRIPT_UNTRUSTED_ARTIFACT` | `LAYER_DIGEST_MISMATCH`, `LOCK_DIGEST_MISMATCH` | no | the materialized layer or the served lock does not match the signed bundle |
| `SCRIPT_SANDBOX_UNAVAILABLE` | `LAYER_UNAVAILABLE` | yes | materialization could not fetch files or ran past its deadline |

## Other

`404` answers alike for another tenant's artifact, file, reservation or upload and for a missing one. `409
PACKAGE_VERSION_EXISTS` answers an upload of a different file for an uploaded version. `409 POLICY_REVISION_CONFLICT`
answers a policy write with a stale expected revision.
