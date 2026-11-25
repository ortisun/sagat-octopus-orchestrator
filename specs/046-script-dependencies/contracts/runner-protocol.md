# Runner routes and channel messages (design 046)

## Runner routes (control plane, `/script-runner/`, runner identity)

Every claim carries `capabilities` (a list; `script-dependencies-v1` for this design) and `target` (`linux-amd64` or
`linux-arm64`). A runner that does not declare the capability is never given a dependency job, a materialization, a
test run with a manifest or a step of a bundle version 5.

| Route | Body | Answer |
|---|---|---|
| `POST /script-runner/dependency-jobs/claim` | `{capabilities, target, languages}` | `204`, or a job: `{jobId, leaseToken, phase: "resolve" \| "install", version, language, runtime, targets, manifest, lock (install only), files (install only: hash, size, install path, ecosystem, for this target), gatewayToken}` |
| `POST /script-runner/dependency-jobs/{jobId}/heartbeat` | `{leaseToken}` | `204`, or `409` when the lease is lost |
| `POST /script-runner/dependency-jobs/{jobId}/complete` | resolve: `{leaseToken, outcome, lock}`; install: `{leaseToken, outcome, target, layer: {digest, size, files}, installScripts: [{name, version, scripts}]}`; failure: `{leaseToken, outcome: "failed", kind, code, retryable, log}` | `204`; a finished job answers `204` and changes nothing |
| `POST /script-runner/materializations` | `{artifactDigest, target}` | `{lock, files, gatewayToken}`; `404` for an artifact of another tenant or one without a layer for that target |
| `POST /script-runner/test-runs/{runId}/dependencies` | `{leaseToken, lock}` | `{verdict, findings, files, gatewayToken}` or `422` with the refusal reason |

Bodies are at most 512 KiB (the lock is at most 256 KiB), under the router's existing 4 MiB limit. Nothing large passes
through the control plane: files go from the gateway to the runner.

## Channel messages of a resolve container

On the supervisor channel of feature 005 (JSON lines, at most 240 KiB each):

- Container to runner: `{"type":"http","id":N,"method":"GET","path":"/npm/axios","accept":"application/vnd.npm.install-v1+json"}`.
  `method` is `GET`; `path` must match a package route of the job's ecosystem (`package-gateway.md`); at most 8 requests
  in flight.
- Runner to container: `{"type":"httpHead","id":N,"status":200,"contentType":"...","length":L}`, then
  `{"type":"httpChunk","id":N,"data":"<base64, at most 192 KiB decoded>"}` lines, then `{"type":"httpEnd","id":N}`. A
  refused path answers `httpHead` with status `403` and no body. A transport failure toward the gateway answers status
  `502` and the runner records the job's failure kind as `PACKAGE_GATEWAY_UNAVAILABLE` if the resolver then fails.
- Container to runner, at the end: `{"type":"lockChunk","data":"<base64>"}` lines (at most 256 KiB decoded in total),
  then `{"type":"result","lockDigest":"sha256:..."}`, or `{"type":"error","message":"..."}`.

Bounds: 32 MiB per response, 1 GiB per job, 4,000 requests per job, the job's 300 s wall clock. Past a bound the runner
stops the container and fails the job as `RESOLUTION_FAILED` (not retryable).
