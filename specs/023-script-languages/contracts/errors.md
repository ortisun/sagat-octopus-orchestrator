# Errors and failure mapping (feature 023 draft)

The error envelope and catalog are unchanged (`sdks/error-catalog.json`: `BAD_REQUEST`, `UNAUTHENTICATED`,
`FORBIDDEN`, `NOT_FOUND`, `CONFLICT`, `VALIDATION_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`,
`UNAVAILABLE`, …). No new top-level code is added. Every error carries `code`, `retryable` and `traceId`,
and never a source line, input value, lock content, package selection or secret.

## New `details.reason` values (`422 VALIDATION_FAILED`)

| Reason | Where | Meaning |
|---|---|---|
| `LANGUAGE_DISABLED` | `createScript` (a draft's language is fixed at creation; `UpdateScriptRequest` has no language), `publishScriptVersion`, `createScriptTestRun` | The namespace script runtime policy disables the language |
| `PACKAGE_NOT_ALLOWED` | `publishScriptVersion`, `createScriptTestRun`, `getScriptPackageCatalog?select=`, `putScriptRuntimePolicy` | A lock or selection names a package outside the catalog, or one the namespace policy excludes |
| `LOCKFILE_INVALID` | `createScript`, `updateScript`, `publishScriptVersion`, `createScriptTestRun` | The lockfile does not parse as the language's format, has a foreign root, dev packages or an incomplete closure, or is non-empty for Bash/PowerShell |

Existing reasons keep their meaning: `LANGUAGE_NOT_BUILT` (also for the reserved `python-data` until
feature 024 ships it), `ABOVE_CEILING`, `BUNDLE_MISMATCH` (now also
when the SBOM package set differs from the lockfile).

## Workflow publication

`SCRIPT_LANGUAGE_DISABLED` joins `UNKNOWN_SCRIPT` among the publication diagnostics of `publishWorkflow`
(see `workflow-script-language.schema.json`).

## Build failure kinds (runner → control plane, unchanged set)

| Kind | New causes |
|---|---|
| `rejected` | `bash -n`, PowerShell parser, `php -l` or `rustc` error; Rust `unsafe` under `forbid`; missing `Run`/`run` entry |
| `lockfile` | catalog mismatch (including a `python-data` lock against the image catalog), unknown package, checksum or reference mismatch, incomplete closure, catalog digest differing between control plane and image |
| `limit` | build profile exceeded; v4 tree above 1,024 files or 1,400 KiB; Rust executable above 1,400 KiB compressed or 8 MiB expanded |
| `unsupported` | language not in the runner's probed set |
| `unavailable` | engine or image unavailable (retried as in 005) |

## Reserved host-call refusals (script-visible, FR-023-028)

These are reply reasons on the supervisor channel, not HTTP errors and not step failures; a refusal never
fails the step by itself (the script decides).

| Call | `reason` | Meaning |
|---|---|---|
| both | `NOT_GRANTED` | the lease does not grant the capability (session below the owning feature's minor, no `emits` channel, or a test run) |
| both | `BUDGET` | 600 `checkpoint` or 1,000 `stream.append` calls already made in this run |
| `checkpoint` | `REFUSED_SIZE`, `REFUSED_SECRET` | above the effective `heartbeat_detail_bytes`; contains a secret value handed to the run (019's names) |
| `stream.append` | `STREAM_EVENT_INVALID`, `SECRET_IN_OUTPUT`, `STREAM_LIMIT`, `STREAM_RATE` (`retryAfterMs`), `STALE_ATTEMPT` | 026's refusals, passed through; `SECRET_IN_OUTPUT` may also come from the supervisor's own net |

## Runtime failure codes (unchanged set)

| Code | New-language causes |
|---|---|
| `SCRIPT_ERROR` | non-zero Bash exit; PowerShell terminating error or depth overflow; PHP uncaught throwable; Rust `Err` or panic |
| `SCRIPT_NO_RESULT` | Bash exit 0 without `octopus result`; process exit without a `result` line |
| `SCRIPT_TIMEOUT`, `SCRIPT_OUT_OF_MEMORY`, `SCRIPT_OUTPUT_LIMIT`, `SCRIPT_SCHEMA_VIOLATION` | as in 005, enforced by the runner-side supervisor and the kernel |
| `SCRIPT_DENIED` | as in 005, plus: a step refused at lease because the namespace policy disables its artifact's language (non-retryable; fixed safe message; `script.policy_denied` audit with point `lease`) |
| `SCRIPT_SANDBOX_UNAVAILABLE` | Rust target mismatch; a `python-data` artifact whose `runtimeCatalog` differs from the runner's runtime catalog; runtime image unavailable; language not served |
| `SCRIPT_UNTRUSTED_ARTIFACT` | digest, signature or bundle rules (v1/v3/v4) fail at load |

`retryable`: `SCRIPT_ERROR`, `SCRIPT_TIMEOUT` and `SCRIPT_SANDBOX_UNAVAILABLE`, as in 005 D106.
