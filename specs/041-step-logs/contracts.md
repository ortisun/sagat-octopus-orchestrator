# Contracts of feature 041 (step logs)

What T041-3 copies into `specs/001-platform-foundation/contracts/worker.proto` (and its six SDK copies) and into
`specs/001-platform-foundation/contracts/openapi.yaml`. It is part of the design
[docs/designs/041-step-logs.md](../../docs/designs/041-step-logs.md) (section 1 and section 5 say why; this file says
what). After T041-3 merges, the contract files are the authority and this file is not edited again except to record a
change that a later task makes to them.

Conventions read in the code: enums start with `PREFIX_UNSPECIFIED = 0` (`worker.proto`, for example
`EGRESS_CEILING_UNSPECIFIED`); `attempt` is `int32` and `generation` `int64`, as in `HeartbeatRequest`
(`worker.proto:128-133`); a REST list answers `{items, nextCursor}` with `Cursor` and `Limit` (`openapi.yaml:4449-4458`);
`Error.code` is the closed set `BAD_REQUEST, UNAUTHENTICATED, FORBIDDEN, NOT_FOUND, CONFLICT, VALIDATION_FAILED, RATE_LIMITED, QUOTA_EXCEEDED, UNAVAILABLE, INTERNAL` (`openapi.yaml:4881`, `crates/api-http/src/error.rs:26`, held equal by
`the_error_codes_of_the_contract_are_exactly_the_ones_the_code_can_emit`); a refinement is a `details.reason` listed in
the `Error.details` description and in the remediation table of `error.rs` (`ErrorCode::remediation`). This feature adds
no `Error.code` value.

## 1. gRPC, protocol minor 1.7

Added to `service WorkerService`:

```proto
  // Added in 1.7 (feature 041). Stores the log records of one leased attempt. Answered only after the
  // records are committed; a record is stored at most once (attempt, generation, sequence).
  //   Verdicts, not errors, say what the worker does next (StepLogVerdict).
  //   Errors: UNAUTHENTICATED; PERMISSION_DENIED (no worker:execute, or a runner or database-runner
  //   token); INVALID_ARGUMENT a malformed
  //   batch (more than 500 records, more than 192 KiB encoded, sequences not increasing, a sequence at or
  //   above 2^62, an unknown level); RESOURCE_EXHAUSTED (x-sagat-error RATE_LIMITED, retry-after-ms) shed
  //   by admission; UNAVAILABLE the database or the serving gate.
  rpc AppendStepLogs(AppendStepLogsRequest) returns (AppendStepLogsResponse);
```

Messages, appended after the existing ones:

```proto
// Added in 1.7. The attempt is named and fenced exactly as HeartbeatRequest names and fences it, plus
// the session that registered the worker.
message AppendStepLogsRequest {
  string worker_id = 1;                       // the session id RegisterWorkerResponse returned
  string session_token = 2;                   // the session token currently held (after a re-registration, the new one)
  string task_id = 3;                         // carried as HeartbeatRequest carries it; the step is found by
                                              // run_id, step_id and instance
  int64 generation = 4;
  string lease_token = 5;
  string run_id = 6;
  string step_id = 7;
  int32 attempt = 8;
  ExecutionInstanceIdentity instance = 9;
  repeated StepLogRecord records = 10;        // 0 to 500, sequences strictly increasing
  bool final_batch = 11;                      // the attempt's last batch, sent before CompleteStep/FailStep
  uint64 highest_sequence = 12;               // with final_batch: the highest sequence the logger assigned (at most
                                              // 2^62-1, else INVALID_ARGUMENT); else 0
}

enum StepLogLevel {
  STEP_LOG_LEVEL_UNSPECIFIED = 0;             // refused as INVALID_ARGUMENT
  STEP_LOG_LEVEL_TRACE = 1;
  STEP_LOG_LEVEL_DEBUG = 2;
  STEP_LOG_LEVEL_INFO = 3;
  STEP_LOG_LEVEL_WARN = 4;
  STEP_LOG_LEVEL_ERROR = 5;
}

message StepLogRecord {
  uint64 sequence = 1;                        // 1 .. 2^62-1, one per logger call that passed the level floor
  int64 worker_time_epoch_ms = 2;
  StepLogLevel level = 3;
  string message = 4;                         // redacted and cut by the SDK; the server cuts again at 8 KiB
  repeated StepLogField fields = 5;           // at most 32, keys unique after redaction
  string exception = 6;                       // formatted exception text and stack, redacted; empty when none
  bool truncated = 7;                         // the SDK cut the message, the exception or the fields
}

message StepLogField {
  string key = 1;
  oneof value {
    string string_value = 2;
    int64 int_value = 3;
    double double_value = 4;                  // finite only; NaN and the infinities travel as string_value
    bool bool_value = 5;
  }
}

enum StepLogVerdict {
  STEP_LOG_VERDICT_UNSPECIFIED = 0;
  STEP_LOG_VERDICT_CONTINUE = 1;              // keep sending
  STEP_LOG_VERDICT_FINAL_ONLY = 2;            // a ceiling was met: send no more records; still send the empty final batch
  STEP_LOG_VERDICT_STOP_ATTEMPT = 3;          // send nothing more for this attempt (stop_reason says why)
  STEP_LOG_VERDICT_STOP_CAPTURE = 4;          // send nothing more on this session (stop_reason says why)
  STEP_LOG_VERDICT_RETRY_LATER = 5;           // nothing stored; send the same batch again after retry_after_ms
}

enum StepLogStopReason {
  STEP_LOG_STOP_REASON_UNSPECIFIED = 0;
  STEP_LOG_STOP_REASON_FINAL_RECORDED = 1;    // STOP_ATTEMPT: the final batch is already stored (a resend of it gets this)
  STEP_LOG_STOP_REASON_LEASE = 2;             // STOP_ATTEMPT: the lease is not this attempt's, has expired, or the run is ending
  STEP_LOG_STOP_REASON_AUTHORITY = 3;         // STOP_ATTEMPT: binding or membership no longer covers the step
  STEP_LOG_STOP_REASON_REMOVED = 4;           // STOP_ATTEMPT: the attempt's log was deleted or purged
  STEP_LOG_STOP_REASON_CAPTURE_OFF = 5;       // STOP_CAPTURE: the deployment switch or the namespace turned capture off
  STEP_LOG_STOP_REASON_SESSION = 6;           // STOP_CAPTURE: the session is unknown, expired, of another subject,
                                              // namespace or token, or below 1.7
}

message AppendStepLogsResponse {
  StepLogVerdict verdict = 1;
  StepLogStopReason stop_reason = 2;          // set with STOP_ATTEMPT and STOP_CAPTURE, else UNSPECIFIED
  uint64 stored_through_sequence = 3;         // the stream's highest worker sequence now known to the server
  int32 retry_after_ms = 4;                   // with RETRY_LATER: 50 to 5,000
}

// Added in 1.7. Sent only when the accepted minor is at least 7.
message StepLogPolicy {
  bool enabled = 1;                           // the deployment switch and the namespace's capture are both on
}
```

`RegisterWorkerResponse` gains `StepLogPolicy step_logs = 4;`. Nothing else in the policy: the buffer, flush and batch
numbers are constants of `sdks/SPEC.md` (design section 6), and every ceiling is enforced by the server whatever the
worker does.

`worker.baseline.json` records the new rpc, the six messages and the field of `RegisterWorkerResponse`
(`SAGAT_UPDATE_BASELINE=1`, `crates/api-grpc/tests/compatibility.rs`), read as a diff before it is committed.
`crates/api-grpc/build.rs` adds `AppendStepLogsRequest`, `StepLogRecord`, `StepLogField` and `StepLogField.value` to
`skip_debug`, with hand-written `Debug` impls that print counts only.

## 2. REST

Four operations, each tagged `x-planned: '041'` by T041-3 and switched to `x-implemented: '041'` by the task that serves
it (T041-11 for the first two, T041-12, T041-30). Path parameter `RunId` as every run route.

### `GET /runs/{runId}/logs` (`listRunLogs`, tag Runs)

Needs `run:read` and `run:logs:read`, and in a classified namespace the PII reveal grant (design FR-041-013).
Parameters: `Cursor`, `Limit`, and

| Name | Schema | Meaning |
|---|---|---|
| `stream` | string, format uuid | one stream (`StepLogStream.id`) of this run: an attempt log, or a script execution's id for its marker |
| `node` | string, 1-256 | the definition node |
| `attempt` | integer, 1-2^31-1 | |
| `minLevel` | enum `trace, debug, info, warn, error` | ordered as listed |
| `from`, `to` | string, format date-time | storage time, `from` inclusive, `to` exclusive |
| `q` | string, 1-128 characters | literal, case-insensitive substring of the message |

Response `200`:

```yaml
StepLogPage:
  type: object
  required: [items, nextCursor, heldBack, complete]
  properties:
    items: {type: array, items: {$ref: '#/components/schemas/StepLogLine'}}
    nextCursor: {type: string, description: 'Always present: the position after the last line returned, to poll from.'}
    heldBack: {type: boolean, description: 'Lines of this run are stored but not yet returned, because an older transaction of the database is still open.'}
    complete: {type: boolean, description: 'The run is finished and every line that can still be stored for it has been returned up to this cursor.'}
StepLogLine:
  type: object
  required: [stream, node, attempt, instancePath, sequence, storedAt, kind, level, truncated]
  properties:
    stream: {type: string, format: uuid}
    node: {type: string}
    attempt: {type: integer}
    instancePath: {type: [array, 'null'], items: {$ref: '#/components/schemas/ApprovalScope'}, description: 'The existing {scopeNodeId, iterationIndex} schema; empty for a root step; null for a script marker, which has no iteration.'}
    sequence: {type: string, description: 'Decimal; above 2^53 for a marker.'}
    workerTime: {type: [string, 'null'], format: date-time}
    storedAt: {type: string, format: date-time}
    kind: {type: string, enum: [line, ceiling_marker, script_marker]}
    level: {type: string, enum: [trace, debug, info, warn, error]}
    message: {type: [string, 'null']}
    exception: {type: [string, 'null']}
    fields: {type: [object, 'null'], additionalProperties: {type: [string, number, boolean]}}
    truncated: {type: boolean}
    marker: {type: [object, 'null'], properties: {reason: {type: string, enum: [attempt_records, attempt_bytes], description: 'ceiling_marker'}, scriptExecutionId: {type: string, format: uuid, description: 'script_marker: open it with getScriptExecution'}}}
```

### `GET /runs/{runId}/logs/streams` (`listRunLogStreams`, tag Runs)

Same permissions. Parameters `Cursor`, `Limit`, `source` (enum `worker`, `script`, default `worker`; each source is paged
on its own). Response `200` `{items: StepLogStream[], nextCursor: string|null}`:

```yaml
StepLogStream:
  type: object
  required: [id, source, node, attempt, instancePath, state]
  properties:
    id: {type: string, format: uuid, description: 'The stream public id; for source script, the script execution id, which is also the id of its marker stream.'}
    source: {type: string, enum: [worker, script]}
    node: {type: string}
    attempt: {type: integer}
    instancePath: {type: [array, 'null'], items: {$ref: '#/components/schemas/ApprovalScope'}, description: 'Null for source script.'}
    state: {type: string, enum: [open, ended, ended_without_final, deleted, expired], description: 'deleted: the delete route; expired: removal by retention has begun, and no line is returned'}
    records: {type: integer, description: 'worker only: lines stored'}
    bytes: {type: integer, description: 'worker only: text bytes stored (UTF-8 octets of message, exception and fields; the tenant quota charges 128 more per line)'}
    droppedByWorker: {type: integer, description: 'worker only: sequences the worker numbered and never stored'}
    droppedByServer: {type: integer}
    ceiling: {type: [string, 'null'], enum: [attempt_records, attempt_bytes, run_records, run_bytes, tenant_quota, null]}
    firstAt: {type: [string, 'null'], format: date-time}
    lastAt: {type: [string, 'null'], format: date-time}
    expiresAt: {type: [string, 'null'], format: date-time, description: 'worker: the last recorded batch plus the retention the stream took when it was created; script: the execution expiry.'}
    removedAt: {type: [string, 'null'], format: date-time}
    logTruncated: {type: boolean, description: 'script only'}
```

Items of `source = worker` are the workers' attempt logs; the marker streams of script attempts are not listed there.
`ended` means the final batch was stored; `ended_without_final` means the attempt's lease is gone and no final batch was
stored (design FR-041-017); `open` otherwise. For `source = script` the item is a row of `script_executions` of the run
(`state` `ended` or `expired`), read under the same rule as FR-041-022.

### `GET /runs/{runId}/logs/stream` (`streamRunLogs`, tag Runs)

`text/event-stream`, the filters of `listRunLogs` except `Cursor` and `Limit`, plus `after` (a cursor; `Last-Event-ID`
wins). Events:

| Event | `id` | `data` |
|---|---|---|
| `log` | the cursor after the last line of the event | `{"items": StepLogLine[]}`, at most 200 |
| `progress` | the cursor after the scanned position | `{"heldBack": bool}`; sent instead of a `: ping` when the position moved without a matching line |
| `end` | the final cursor | `{}`; the run is finished and `complete` holds |
| `reconnect` | the current cursor | `{}`; token expiry or the 15-minute lifetime |

A `: ping` comment every 15 seconds otherwise. The caps are those of `streamRunChanges` and shared with it: `429 RATE_LIMITED` with `STREAMS_GLOBAL`, `STREAMS_PER_SUBJECT` or `STREAMS_PER_TENANT` (`crates/api-http/src/routes/execution.rs:1264-1269`).

### `DELETE /runs/{runId}/logs` (`deleteRunLogs`, tag Runs)

Needs `run:logs:delete` (admin). Optional `stream` (uuid, of this run). Response `204` with no body, so the answer says
nothing about how many streams the run has; deleting what is already deleted also answers `204` and audits nothing.

### `getScriptExecution` and `listScriptExecutions` (FR-041-022)

`ScriptExecution` gains an optional `logWithheld: boolean`. For a caller who may not read the run's step logs, `log` is
`""`, `logTruncated` is `false` and `logWithheld` is `true`; `ScriptExecutionSummary.logTruncated` is `false` for the
same caller. `log` stays required, so the schema change is additive; the change of what a viewer is given is a behaviour
change stated in both descriptions and the release notes.

### `recordScriptExecution` (the script runner's report)

Unchanged. The script marker is keyed on the execution the report creates (design section 7, D041-060), so the runner
sends nothing new.

### `Quota.resource` and `QuotaResourceName`

Gain `tenant_step_log_bytes` (ADR-0027 Amendment of 2026-11-15), `step_log_attempt_records`, `step_log_attempt_bytes`, `step_log_run_records`, `step_log_run_bytes`,
`step_log_retention_days` (definitions in `data-model.md`). The `Quota` description's rule that lowering a limit does
not erase retained content holds for them as for every resource: a stream keeps the retention it was created with.
The `used` description gains one absence: `tenant_step_log_bytes` has no `used` for a caller without `run:logs:read`
(D041-066).

### Error table

| Refusal | Status | `code` | `details.reason` |
|---|---|---|---|
| No `run:read` or no `run:logs:read` (or no `run:logs:delete` on delete) | 403 | `FORBIDDEN` | none, as every missing permission (`error.rs:590`) |
| Classified namespace, no PII reveal grant | 403 | `FORBIDDEN` | `PII_REVEAL_NOT_GRANTED` (exists: `routes/payload_disclosure.rs:162`) |
| Run of another tenant, or unknown | 404 | `NOT_FOUND` | none; audited `access.denied.cross_tenant` when it exists elsewhere |
| `stream` not of this run | 404 | `NOT_FOUND` | none |
| Cursor or `after` not valid, altered, or of another run | 400 | `BAD_REQUEST` | none, as every cursor (`routes/primitives.rs:193-205`) |
| `q` over 128 characters, unknown `minLevel` or `source`, `from` after `to` | 422 | `VALIDATION_FAILED` | none; `details.field` names the parameter |
| `SAGAT_STEP_LOGS_READ_ENABLED=false` | 422 | `VALIDATION_FAILED` | `STEP_LOGS_NOT_AVAILABLE` (new; the pattern of `RESET_REAPPLY_NOT_AVAILABLE`) |
| Tail caps | 429 | `RATE_LIMITED` | `STREAMS_GLOBAL`, `STREAMS_PER_SUBJECT`, `STREAMS_PER_TENANT` |
| Database unavailable | 503 | `UNAVAILABLE` | none |

The one new reason, `STEP_LOGS_NOT_AVAILABLE`, is added in T041-3 to the `Error.details` description ("step logs
(feature 041) refine `VALIDATION_FAILED` with `STEP_LOGS_NOT_AVAILABLE`, the deployment's read switch is off") and in
T041-11 to the remediation table and its test.

## 3. Ends of a read

The tail and the polling fallback end by the same rule, carried in the cursor so that a reconnect or a new poll keeps
it. The cursor is `base64url` of a JSON object `{"v":1,"run":"<uuid>","xid":"<decimal>","seq":"<decimal>","end":"<decimal>"|null}`; a cursor whose `run` is not the path's run is refused (400). It holds no row identity. When a
read sees the run finished and the cursor has no `end`, the read ends its transaction and takes `end = pg_current_xact_id()` in a new, short transaction, so that the id is assigned after the read saw the run finished, and
puts it in the cursor it returns. A read whose cursor has `end` answers `complete: true` (the tail sends `event: end`)
when the run is finished, `pg_snapshot_xmin(pg_current_snapshot()) > end` in the reading statement, and the page from the
cursor is empty; an `end` in a cursor of a run that is not finished is ignored. This is D1958 of feature 018
(`crates/infra-postgres/src/batch.rs:453-459, 654-656`), applied to the run's terminal transition (design section 5).

The Studio's `followLogs()` falls back to polling `listRunLogs` from `nextCursor` and stops when `complete` is true.

## 4. What a worker does with each answer

The SDK rules of `sdks/SPEC.md` (design section 6) are, per answer of `AppendStepLogs`:

| Answer | SDK action | Counts toward the two retries |
|---|---|---|
| `CONTINUE` | next batch | - |
| `FINAL_ONLY` | numbers and drops further records locally; sends the empty final batch at the end; in answer to the final batch, its success | - |
| `STOP_ATTEMPT` | sends nothing more for the attempt; a `FINAL_RECORDED` answer to a final batch is its success | - |
| `STOP_CAPTURE`, `CAPTURE_OFF` | stops for the session until the next successful registration | - |
| `STOP_CAPTURE`, `SESSION` | if the session token it now holds differs from the one it sent, sends the batch once more with it; otherwise as `CAPTURE_OFF` | - |
| `RETRY_LATER` | waits `retry_after_ms`, sends the same bytes; a batch older than 10 s is dropped | no |
| `RESOURCE_EXHAUSTED` | as `RETRY_LATER`, with the `retry-after-ms` trailer | no |
| `UNAVAILABLE`, `DEADLINE_EXCEEDED` | resends the same bytes after 200 ms, then 1 s (each ±20%) | yes |
| `INVALID_ARGUMENT` | drops the batch (the gap is counted by the server) and goes on | - |
| `UNAUTHENTICATED`, `PERMISSION_DENIED` | stops for the session until the next successful registration | - |
| `UNIMPLEMENTED` | stops for the attempt (an older API replica) | - |
| any other | as `UNAVAILABLE` | yes |
