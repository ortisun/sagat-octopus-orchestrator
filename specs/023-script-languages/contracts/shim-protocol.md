# Per-language shims over the existing supervisor channel (feature 023 draft)

The channel is unchanged from feature 005 (`crates/infra-sandbox/src/protocol.rs`, ADR-0013): one JSON
object per line on the container's standard output (script side to supervisor) and standard input
(supervisor to script side); the script's own output is the log on standard error. Messages:

| Direction | `op` | Fields | Bound (runner-side supervisor) |
|---|---|---|---|
| shim → supervisor | `ready` | — | first line |
| shim → supervisor | `fetch` | `id`, `method`, `url`, `headers`, `body` | counts toward 100 host calls; egress policy re-read before each call; `Host`, framing, connection and proxy headers dropped |
| shim → supervisor | `secret` | `id`, `name` | counts toward 100 host calls; only names the step declared; never in a test run |
| shim → supervisor | `checkpoint` | `id`, `details` | reserved, capability-gated; see "Reserved host calls" |
| shim → supervisor | `stream.append` | `id`, `events` | reserved, capability-gated; see "Reserved host calls" |
| supervisor → shim | reply | `id`, `ok`, then `status`/`headers`/`body`, `value` or `error` | bounded by the run's deadline |
| shim → supervisor | `result` | `value` | ≤ 240 KiB parsed; validated against `outputSchema` by the supervisor |
| shim → supervisor | `error` | `message`, `stack` | redacted, bounded; never repeated in the step's safe message |
| builder → supervisor | `built` / `rejected` | `entry`, `files` / `message` | build line bound (native and v4 use the build line) |

Every line is at most 240 KiB plus framing; past 400 lines the run is stopped; refusals are recorded as
`fetch <scheme>://<host>` or `secret <name>`, at most 200 characters, 32 per run. A line the supervisor
does not understand is ignored. **No shim is a security boundary**: each runs inside the container with
the untrusted code, and a script that bypasses its shim and writes the protocol itself gains nothing the
supervisor would not grant the shim.

## Reserved host calls: `checkpoint` and `stream.append` (coordinator decision C17; D1231–D1233)

This feature owns the shim protocol for every script language: the five of feature 005 (through their
existing shims), the four of this feature and the reserved `python-data`. It defines only the message
shapes, the gate, the bounds and the redaction net of two reserved host calls. **Their meaning belongs to
their owners**: heartbeat checkpoint details to feature 019 (FR-019-015..018: ordering, coalescing,
storage, delivery to the next attempt, classification by `heartbeatSchema`, the `REFUSED_*` statuses);
Workflow Streams to feature 026 (FR-026-013..020, ADR-0068: channels, `emits`, schemas, budgets, offsets,
deduplication, the feed). Until 019 or 026 ships, the supervisor answers the corresponding call with
`NOT_GRANTED` and the scripts behave as on a lease without the capability.

**Gate.** The supervisor refuses a reserved call (`ok: false`, `reason: "NOT_GRANTED"`) unless the step's
current lease grants the capability: for `checkpoint`, the runner's worker session negotiated 019's
checkpoint minor and the lease is a workflow step lease; for `stream.append`, the session negotiated
026's stream minor and the lease carries at least one `emits` channel for the node. A test run has no
workflow lease, so both are always `NOT_GRANTED` there. The server checks again (channel declaration,
schemas, fences); the supervisor's gate only avoids useless calls.

**Shapes** (one JSON line each, existing framing):

| Message | Fields | Reply (`id`, `ok`, then) |
|---|---|---|
| `{"op":"checkpoint","id":n,"details":{…}}` | `details`: a JSON object | `status`: `QUEUED` (kept as the attempt's latest pending details, sent with the next heartbeat or the final failure report, as 019 requires), or `ok: false` with `reason` `NOT_GRANTED`, `REFUSED_SIZE`, `REFUSED_SECRET`, `BUDGET` |
| `{"op":"stream.append","id":n,"events":[{"channel":"…","data":{…}},…]}` | 1 to 64 events; `channel` matches `^[a-z][a-z0-9_.-]{0,62}$` and does not start with `sagat.`; `data` a JSON object | `offsets`: one per event (from `AppendStreamEvents`), or `ok: false` with `reason` `NOT_GRANTED`, `STREAM_EVENT_INVALID`, `SECRET_IN_OUTPUT`, `STREAM_LIMIT`, `STREAM_RATE` (with `retryAfterMs`), `STALE_ATTEMPT`, `BUDGET` |

The previous checkpoint that 019 delivers with the lease travels to the shim in the run's first message as
`"checkpoint": {"attempt": k, "details": {…}}` or `null`, and is exposed read-only by every shim.

**Rules the supervisor enforces** (runner side, not the shim):

- *Bounds.* `details` and each event's `data` are canonical JSON objects; `details` is at most the
  effective 019 `heartbeat_detail_bytes` (16 KiB default, 64 KiB ceiling); an event is at most 026's event
  size (16 KiB default, 64 KiB ceiling) and a request at most 64 events and 64 KiB, never above the
  240 KiB channel line. Per run at most 600 `checkpoint` calls and 1,000 `stream.append` calls; past a
  budget every further call is refused `BUDGET`. These budgets are separate from the 100 fetch/secret
  calls, and the channel line ceiling becomes 400 + 600 + 1,000 = 2,000 lines per run, still finite.
- *Ordering and idempotency.* The supervisor assigns 026's `publisher_seq` itself, monotonic per attempt,
  so a script cannot reuse one; it retries `UNAVAILABLE` with the same batch and sequences until the lease
  deadline (026's SDK rule). A checkpoint keeps only the latest valid pending `details`; the supervisor
  assigns 019's `details_sequence`. The engine's later status (`STORED`, `COALESCED`, `STALE`,
  `ENCODING_REQUIRED`, per 031) is counted in the runner metric `script_checkpoints_total{language,outcome}`, not
  sent back to the script and not stored by this feature.
- *Redaction net.* Before forwarding, the supervisor searches `details` and every event's `data` for each
  secret value handed to the run (the same exact-value net as the log and the result, 005 D111) and
  refuses a match locally (`REFUSED_SECRET`, `SECRET_IN_OUTPUT`); the server scans again. Neither payload
  is ever written to the script log, the execution record, a span, a metric label or an audit entry.
  A refusal is recorded like other refusals, as `checkpoint` or `stream.append <channel>` (at most 200
  characters, never a value) and audited once per actor, resource and reason.
- *Not a boundary.* As for fetch and secret, a script that bypasses its shim and writes the messages
  itself gains nothing the supervisor would not grant.

**Author API per language** (additive; the call names are reserved in every shim):

| Language | Record a checkpoint | Previous checkpoint | Append stream events |
|---|---|---|---|
| TypeScript | `await octopus.checkpoint(details)` | `octopus.previousCheckpoint` | `await octopus.stream(channel).append(data)` |
| Python, `python-data` | `octopus.checkpoint(details)` | `octopus.previous_checkpoint` | `octopus.stream(channel).append(data)` |
| C# | `await octopus.CheckpointAsync(details, ct)` | `octopus.PreviousCheckpoint` | `await octopus.Stream(channel).AppendAsync(data, ct)` |
| Go | `client.Checkpoint(details)` | `client.PreviousCheckpoint()` | `client.Stream(channel).Append(data)` |
| Java | `octopus.checkpoint(details)` | `octopus.previousCheckpoint()` | `octopus.stream(channel).append(data)` |
| Bash | `octopus checkpoint [JSON \| -]` (exit 3 refused, 6 budget) | `octopus previous-checkpoint` | `octopus stream append CHANNEL [JSON \| -]` |
| PowerShell | `Set-OctopusCheckpoint -Details <hashtable>` | `Get-OctopusPreviousCheckpoint` | `Add-OctopusStreamEvent -Channel <string> -Data <hashtable>` |
| PHP | `$octopus->checkpoint(array $details)` | `$octopus->previousCheckpoint()` | `$octopus->stream($channel)->append(array $data)` |
| Rust | `octopus.checkpoint(&details)?` | `octopus.previous_checkpoint()` | `octopus.stream(channel).append(&data)?` |

A refusal is returned to the script as the language's ordinary refusal (`PermissionError` in Python, a
rejected promise in TypeScript, a non-zero exit in Bash, an `Err` in Rust, an exception elsewhere) and
never fails the step by itself. TypeScript and Python shims are sent to the container at run time, so
existing artifacts gain the calls when the runner is upgraded; C#, Go and Java link their trusted API at
build time, so only versions built after the change have them (G1223). Changing a shim changes its hash
and so the toolchain identity: the next build of an unchanged draft builds again (005 D99).

## Bash

Files: `/tmp/main.sh` (the source), `/tmp/input.json` (the input or step document `{run, node, input,
steps}`), socket `/tmp/.sagat/host.sock` (mode 0600), served by the trusted `sagat-shell-host`.
Environment of `bash` (and nothing else): `PATH=/opt/sagat/bin:/usr/bin:/bin`, `HOME=/tmp`,
`LANG=C.UTF-8`, `TZ=UTC`, `OCTOPUS_INPUT=/tmp/input.json`, `OCTOPUS_SOCKET=/tmp/.sagat/host.sock`,
`OCTOPUS_IDEMPOTENCY_KEY=<key>` (empty in test runs without a key). The `octopus` helper:

| Command | Effect | Exit status |
|---|---|---|
| `octopus input` | prints the input JSON | 0 |
| `octopus secret NAME` | prints the secret value without a newline | 0; 3 when refused; 4 when unavailable |
| `octopus fetch [-X METHOD] [-H 'Name: value']… [--data TEXT | --data-file PATH] URL` | prints `{"status":…,"headers":{…},"body":"…"}` | 0 for any HTTP status; 3 refused; 4 failed |
| `octopus idempotency-key` | prints the key | 0 |
| `octopus result [JSON | -]` | records the result (argument, or stdin with `-`), validated as JSON ≤ 240 KiB | 0; 2 malformed or too large; 5 a result was already recorded |
| `octopus log MESSAGE…` | writes to stderr | 0 |
| `octopus checkpoint [JSON \| -]`, `octopus previous-checkpoint`, `octopus stream append CHANNEL [JSON \| -]` | reserved host calls (section above) | 0; 3 refused; 6 budget |

The host sends `result` only after `bash` exits 0 and a result was recorded; exit 0 without one is
`SCRIPT_NO_RESULT`; a non-zero exit is `SCRIPT_ERROR` with the exit status in the execution record. At
most 8 helper connections are open at once; each connection carries one request.

Starter:

```bash
# The input is a JSON file; read it with jq. Quote every expansion.
n="$(octopus input | jq -r '.n // 0')"
octopus log "n = ${n}"
octopus result "$(jq -cn --argjson n "${n}" '{double: ($n * 2)}')"
```

## PowerShell

The trusted `runtime.ps1` dot-sources `/tmp/main.ps1`, then calls `Run -InputObject $input -Octopus
$octopus` with the input converted by `ConvertFrom-Json -AsHashtable -Depth 64`. The `Sagat.Octopus`
module, imported from `/opt/sagat/modules`, exports:

| Command | Maps to |
|---|---|
| `Invoke-OctopusFetch -Uri <string> [-Method <string>] [-Headers <hashtable>] [-Body <string>]` | `fetch`; returns `[pscustomobject]@{Status; Headers; Body}`; throws on refusal |
| `Get-OctopusSecret -Name <string>` | `secret`; returns a string; throws on refusal |
| `Write-OctopusLog <object[]>` | stderr |
| `Get-OctopusIdempotencyKey` | the key |

`$Octopus` is an object with the same four methods. `Run`'s output is collected: none → `null`, one →
that object, several → an array; it is serialized with `ConvertTo-Json -Depth 64 -Compress` and a depth
check that fails the run as `SCRIPT_ERROR` instead of truncating. Information, warning, verbose, debug
and error streams go to stderr. Native `[Console]::Out` is the channel; the runtime redirects the
success stream of everything except the final result.

Starter:

```powershell
function Run {
    param($InputObject, $Octopus)
    $n = if ($InputObject.ContainsKey('n')) { [int]$InputObject['n'] } else { 0 }
    Write-OctopusLog "n = $n"
    return [ordered]@{ double = $n * 2 }
}
```

## PHP

The trusted `runtime.php` writes the v4 tree under `/tmp/app`, starts an output buffer whose callback
writes to `STDERR`, requires `/tmp/app/vendor/autoload.php` when present, requires `/tmp/app/main.php`,
and calls `run($input, $octopus)` where `$input` is the decoded JSON array (`JSON_THROW_ON_ERROR`,
depth 64). Class `Octopus` (final, in the runtime, not the bundle):

| Method | Maps to |
|---|---|
| `fetch(string $url, string $method = 'GET', array $headers = [], ?string $body = null): array` | `fetch`; returns `['status' => int, 'headers' => array, 'body' => string]`; throws `OctopusRefused` |
| `secret(string $name): string` | `secret`; throws `OctopusRefused` |
| `log(mixed ...$values): void` | stderr |
| `idempotencyKey(): ?string` | the key |

The return value is the result, encoded with `JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES |
JSON_PRESERVE_ZERO_FRACTION`; an exception is `error`.

Starter:

```php
<?php
function run(array $input, Octopus $octopus): mixed
{
    $n = (int) ($input['n'] ?? 0);
    $octopus->log("n = $n");
    return ['double' => $n * 2];
}
```

## Rust

The trusted crate `sagat_inline` (in `runners/rust/sagat-inline/`, compiled into the builder image)
reads the input from sealed fd 3, speaks the channel on stdout/stdin, and calls the author's function:

```rust
pub fn run(input: serde_json::Value, octopus: &mut sagat_inline::Octopus)
    -> Result<serde_json::Value, sagat_inline::Error>
```

| Method | Maps to |
|---|---|
| `octopus.fetch(&Request) -> Result<Response, Error>` | `fetch` |
| `octopus.secret(&str) -> Result<String, Error>` | `secret` |
| `octopus.log(impl Display)` | stderr |
| `octopus.idempotency_key() -> Option<&str>` | the key |

A returned `Err` is `error`; a panic is caught by the fixed `main` and reported as `error` with a fixed
message (the panic text goes to the log); a process exit without a result is `SCRIPT_NO_RESULT`. The
author module compiles under the crate-level `#![forbid(unsafe_code)]` of the fixed `main.rs`.

Starter:

```rust
use serde_json::{json, Value};
use sagat_inline::{Error, Octopus};

pub fn run(input: Value, octopus: &mut Octopus) -> Result<Value, Error> {
    let n = input.get("n").and_then(Value::as_i64).unwrap_or(0);
    octopus.log(format!("n = {n}"));
    Ok(json!({ "double": n * 2 }))
}
```

## Reserved `python-data`

The author API is 005's Python one (`def run(input, octopus)`, `octopus.fetch`, `octopus.secret`,
`octopus.idempotency_key`), over the same channel. The trusted runtime adds only the locked catalog
packages of the version to the import path. Everything specific to data execution (staged inputs,
outputs, mounts) is feature 024's contract, which must keep the FR-023-010 profile or record its own
ADR exception.

## Probes

Each language ships a fixed probe source in its runner directory. The startup probe builds it through
the production builder, runs it with the idempotency key `sagat-<language>-capability-probe` and
requires the result `{"ok": true, "idempotencyKey": "sagat-<language>-capability-probe"}`, as the Go
probe does today (`DockerSandbox::probe_go`).
