# TMP-only C2 correction: supplied public pool bootstrap

## Exact base and outputs

- Exact C1 base snapshot:
  `tasks/project-completion/evidence/public-pool-bootstrap-20261018/review-artifacts/root-formatted-c1/api_reopen.rs.snapshot`
- Base SHA-256: `b504becb198a1d9d8445c5021726a329757ad92c8ae87e9290c72a0a9335f0a1`
- C2 patch: `/tmp/sagat-public-pool-bootstrap-c2.patch`
- Patch SHA-256: `2b94ed477f3f545fd1c8b6ce8a83d3cc093fcd2c17a864248791e3a7d37136c2`
- C2 mirror:
  `/tmp/sagat-public-pool-bootstrap-c2-draft/tests/integration/workflow-primitives/api_reopen.rs`
- Mirror SHA-256: `7ec75e4b16a21af1605263ea0171f1cfabcd41dd17cebb39186978595c4b9aa3`
- `git apply --check /tmp/sagat-public-pool-bootstrap-c2.patch`: passed against the exact root formatted C1 working source.
- Delta stat: one source file, 108 insertions, one deletion.

The patch modifies only `tests/integration/workflow-primitives/api_reopen.rs`. It does not modify
`tests/support/api.rs`, the proxy, production code, migrations, grants, contracts, C1 evidence, or
any other repository path.

## Bounded affected review unit

The affected unit remains the single P0 test:

`api_reopen::supplied_single_connection_pool_backs_actual_public_completion_after_reopen`

The C2 delta addresses the four accepted receipt/lifecycle findings without widening the constructor
or fault-injection scope.

### 1. Exact persisted 250,000-byte output

After the first public completion returns, an owner-pool query selects the exact original tenant,
run and step in `SUCCEEDED`, joins `steps.output_digest` to the same-tenant `payloads.digest`,
and returns `octet_length(payload.content)` plus the retained bytes.

A fixed-message boolean assertion requires both:

- exactly 250,000 retained bytes; and
- byte-for-byte equality with the private original JSON output.

No assertion or diagnostic formats the payload.

### 2. Exact durable worker-session token hash

A separate private witness reads `worker_sessions.token_hash` for the exact original tenant,
session id and worker subject. Before setup teardown it is compared with
`Token::hash_of(saved_session_token)` and the saved tenant/session identity.

The complete witness is compared again:

- after the supplied-pool API reopens the existing database; and
- after the public completion and exact replay.

The existing membership, binding and session identity witness remains unchanged.

### 3. Whole-scenario deadline

The named Tokio test now wraps a private scenario future in one 90-second timeout. The deadline
starts before `hold_server`/fixture setup and includes every owner-pool operation, test DDL,
observation query, gate rollback, receipt/payload/session reads, trigger removal, scheduler
settlement, and unarmed relay cleanup.

The supplied PostgreSQL session retains its explicit 30-second statement and idle-in-transaction
timeouts. Normal successful cleanup remains unchanged: drop the client/API references, close the
supplied pool under five seconds, then drop the unarmed proxy so its existing abort guard handles
ordinary EOF.

### 4. Independent canonical receipt hash

The test builds its own completion fingerprint without calling a private production helper:

1. length-prefixed `b"complete"`;
2. length-prefixed nonnegative attempt encoded as big-endian `u32`;
3. length-prefixed nonnegative generation encoded as big-endian `u64`;
4. length-prefixed exact original `output_json` bytes; then
5. SHA-256 over the complete fingerprint.

A fixed-message boolean assertion compares this oracle to the exact stored
`command_dedupe.request_hash`. Existing receipt width, full-row stability, resource result, scope,
key, tenant, timestamp, and exact replay checks remain in place.

## Scope and execution status

This is a source-only TMP draft. No repository file was changed by this C2 task. No formatter,
build, Clippy, test, SQL session, container action, commit, external write, or subagent was run.
Only read-only source inspection and `git apply --check` were performed.

This correction still proves only the supplied-pool public bootstrap with a benign 250,000-byte
completion and replay. The relay remains unarmed. It does not prove scoped refusal rollback,
COMMIT reply loss, recovery after an unknown outcome, process restart, failover, or release
acceptance.

## Intended root sequence

```sh
git apply --check /tmp/sagat-public-pool-bootstrap-c2.patch
git apply /tmp/sagat-public-pool-bootstrap-c2.patch
cargo fmt
```

Then root should obtain a fresh delta review and rerun the focused strict Clippy and the named P0
test in the required normal and explicit runtime-role invocations, preserving the C1 source-pin and
result accounting conventions.

