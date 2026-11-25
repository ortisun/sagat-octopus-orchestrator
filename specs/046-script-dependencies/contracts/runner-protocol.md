# Runner routes and channel messages (design 046)

## Runner routes (control plane, `/script-runner/`, runner identity)

Every claim carries `capabilities` (`script-dependencies-v1` for this design) and `target` (`linux-amd64` or
`linux-arm64`, from `SAGAT_RUNNER_TARGET`). A runner that does not declare the capability is never given a dependency
job, a materialization, a test run with a manifest or a step of a bundle version 5. Bodies stay under the router's 4 MiB
limit; nothing large passes through the control plane except private package files (at most 64 MiB, on their own route).

| Route | Body | Answer |
|---|---|---|
| `POST /script-runner/dependency-jobs/claim` | `{capabilities, target, languages}` | `204`, or `{jobId, attempt, leaseToken, phase: "resolve" \| "install", version, language, runtime, targets, manifest, cut, maxLayerBytes, reservedNames, lock (install), files (install: hash, size, install path, source "public" \| "private"), grant}` |
| `POST /script-runner/dependency-jobs/{jobId}/heartbeat` | `{leaseToken}` | `204`, or `409` when the lease is lost |
| `POST /script-runner/dependency-jobs/{jobId}/complete` | resolve: `{leaseToken, outcome, lock}`; install: `{leaseToken, outcome, target, layer: {digest, size, files}, installScripts: [{name, version, scripts}], compileFailures}`; failure: `{leaseToken, outcome: "failed", kind, code, retryable, log}` | `204`; a finished job answers `204` and changes nothing |
| `GET /script-runner/pending-activities` (feature 005's, extended) | query `target` | each ready activity with its artifact digest and whether a layer for the target is needed |
| `POST /script-runner/materializations` | `{artifactDigest, target, namespace}` | `{lock, files, layerRecord, maxLayerBytes, grant}`; `403 SCRIPT_DENIED` reason `PACKAGE_BLOCKED` if blocked; `404` for another tenant's artifact or one without a record for the target (the attempt is audited) |
| `GET /script-runner/artifacts/{digest}/runnable` | query `namespace` | `204`, or `403 SCRIPT_DENIED` reason `PACKAGE_BLOCKED` |
| `POST /script-runner/test-runs/claim` (feature 005's, extended) | as today | for a draft with a manifest, also `{manifest, cut, reservedNames, targets: [the runner's], grant}` |
| `POST /script-runner/test-runs/{runId}/dependencies` | `{leaseToken, lock}` | `{verdict, findings, files, maxLayerBytes, grant}` or `422` with the refusal reason |
| `GET /script-runner/private-index/npm/{name}`, `GET /script-runner/private-index/pypi/{project}/` | | the tenant's uploads of that name, in the npm and PEP 691 shapes; `404` otherwise |
| `GET /script-runner/private-files/{sha256}` | | a file of the tenant's uploads; `404` otherwise (audited) |

## Channel messages of a resolve container

On the supervisor channel of feature 005 (`op` messages, at most 240 KiB per line):

- Container to runner: the existing `fetch` op with `method: "GET"` and a URL on the loopback base the shim uses; the
  runner maps the path to a gateway package route or, for a reserved name, a private index route. Any other path is
  refused with status `403`.
- Runner to container: `{"op":"fetchHead","id":N,"status":200,"contentType":"...","length":L}`, then
  `{"op":"fetchChunk","id":N,"data":"<base64 of at most 160 KiB>"}` lines, then `{"op":"fetchEnd","id":N}`.
- Container to runner, at the end: `{"op":"lockChunk","data":"<base64>"}` lines (at most 256 KiB decoded in total), then
  `{"op":"result","lockDigest":"sha256:..."}`, or `{"op":"error","message":"..."}`.

Budgets of a resolve job (in place of a script's 100 host calls and 400 channel lines): 4,000 requests, 30,000 lines,
8 requests in flight, 32 MiB per response, 1 GiB in all, the job's 300 s. Past a budget the runner stops the container and
fails the job as `RESOLUTION_FAILED`. A transport failure of a forwarded request fails the job as
`PACKAGE_GATEWAY_UNAVAILABLE`, retryable, whatever the resolver returns.
