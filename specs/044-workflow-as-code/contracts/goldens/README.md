# The golden corpus of workflow as code (design 044, Task 044-30)

The corpus is what the six SDK runtimes, the server's command chain and the conformance suite agree on. It is produced by
`generate.mjs` (Node 20 or later, no dependency: `node:crypto` for SHA-256, `BigInt` for the generator, its own RFC 8785
serializer), so the goldens never come from the Rust code they check; `contract_workflow_as_code::goldens::the_goldens_parse_and_their_chains_recompute`
recomputes them independently with `serde_json` and `sha2`, and checks that `node generate.mjs --check` reproduces the
checked-in files. A golden changes only after its diff has been read (`AGENTS.md` rule 18).

This file is normative for what design 044 leaves to the corpus: the exact body of each command kind that the command
chain hashes (Technical Plan 6.2). Decision 044-232 of the register records it.

## Notation

- `C(v)` is the RFC 8785 canonical JSON of the value `v`, as bytes (UTF-8). Keys are sorted by their UTF-16 code units;
  numbers are printed as ECMAScript prints them (`JSON.stringify`); strings are escaped as `JSON.stringify` escapes them.
- `hex(b)` is the lowercase hexadecimal form of bytes `b`; `SHA(b)` their SHA-256.
- `D(v)` is the descriptor of a value: the object `{"sha256": hex(SHA(C(v))), "size": length of C(v)}`. A value that may
  travel as a blob enters a body as its descriptor, whatever its carrier, so the digest does not depend on the carrier
  and a replaying SDK computes it without uploading (Technical Plan 6.2).

## The chain

For command `k` (numbered 1, 2, 3... over the whole run) with kind `kind`, target `target` and body `B(k)`:

```text
d(k) = C({"seq": k, "kind": kind, "target": target, "body": hex(SHA(C(B(k))))})
H(0) = 32 zero bytes
H(k) = SHA(H(k − 1) ‖ uint32 big-endian length of d(k) ‖ d(k))
```

`kind` is the name of the `CommandKind` value of `worker.proto` without its prefix, in lower case
(`COMMAND_TIMER_START` is `timer_start`). The target per kind is Technical Plan 6.2's table.

## The body of each kind

A work command's fragment is the RFC 8785 code-profile fragment the SDK sends in `Command.fragment_json` (Technical Plan
5.1), which never holds its value field; the value travels in `Command.value`.

| Kind | Body `B(k)` |
|---|---|
| `activity`, `script` | the fragment object with the key `"input"` set to `D(input)`, and the key `"scope"` set to `"s-<n>"` when the command is scheduled in a saga scope (`Command.saga_scope`; absent otherwise) |
| `child` | the fragment object with the key `"input"` set to `D(input)` |
| `approval` | the fragment object with the key `"subject"` set to `D(subject)` |
| `timer_start`, `wait_event` | the fragment object |
| `timer_cancel`, `step_cancel` | `{"target": "c-<m>"}` |
| `saga_open` | `{"scope": "s-<n>", "parentScope": "s-<p>" or null, "onCompensationFailure": ..., "cancelPolicy": ...}` |
| `saga_close`, `saga_abort` | `{"scope": "s-<n>"}` |
| `marker_patch` | `{"markerId": id}` |
| `marker_side_effect` | `{"markerId": id, "value": D(value)}` |
| `set_state` | `{"state": state}` (always inline, so the value itself) |
| `update_accept` | `{"admittedSeq": n, "accepted": true}` |
| `update_reject` | `{"admittedSeq": n, "accepted": false, "reason": code}` |
| `update_complete` | `{"admittedSeq": n, "result": D(result)}`, or `{"admittedSeq": n, "failureCode": code}` for a failed handler; a command with both or neither is `COMMAND_INVALID` |
| `consume_signals` | `{"signals": [{"name": name, "seqs": [ascending sequences]}, ...]}`, the names in ascending UTF-16 order |
| `complete` | `{"output": D(output)}` |
| `fail` | `{"code": code, "message": D(message)}`, `D` of the message as a JSON string (`FailCommand.safe_message`; it is stored only as a `pii` payload) |
| `continue_as_new` | `{"input": input, "state": state or null, "version": "same" or "active"}` (input and state always inline) |
| `acknowledge_cancellation` | `{}` |

Every body holds only integers within ±(2^53 − 1): RFC 8785 assumes I-JSON (RFC 7493), and a larger integer would be
rounded by some serializers and not by others; a command whose body holds one is `COMMAND_INVALID`. (The vector of
`2^53 + 1` in `vectors/rfc8785.json` checks a serializer, not a command.) Numbers that are not integers are printed
as ECMAScript prints them, the closest shortest digits with ties to even; a language's default float printing (Rust's
among them) can differ on a tie and must not be used.

## Order and records

- Within one activation the commands are, in this order: the function's commands in the order it made them (an update
  is accepted or rejected when its `update.admitted` is applied, before the next event), then the consumption of
  signals, then the set-state, then the terminal or continue-as-new command, last.
- Every accepted command has exactly one visible record, delivered as `HISTORY_COMMAND_RECORDED` (a marker as
  `HISTORY_MARKER_RECORDED`) with its command number, whatever the journal kind of Technical Plan 5.1 that writes it
  (`step.released`, `timer.started`, `step.cancelled` of a timer cancel, `saga.entered`, `saga.completed` of a close,
  `run.stop_requested` of a terminal command or an acknowledgement, and so on). Its outcomes (a timer that fired, a
  step that ended) are other events.
- The record of an activity or a script scheduled in a saga scope carries that scope (`HistoryCommandRecord.saga_scope`,
  `sagaScope` in the JSON mapping), the same `s-<n>` its body hashes; a replaying SDK compares it with the kind and the
  target, and a report of a mismatch names the scope the function used (`actual_saga_scope`). `Command.saga_scope` on
  any other kind is `COMMAND_INVALID` (Decision 044-244).
- A step ends with one visible event: an activity or a script with `HISTORY_STEP_OUTCOME`; a wait with
  `HISTORY_WAIT_OUTCOME`, an approval with `HISTORY_APPROVAL_DECIDED`, and either of them, when cancelled, with
  `HISTORY_STEP_OUTCOME` `STEP_OUTCOME_CANCELLED` (error code `CANCELLED`, origin `EFFECT` when the completion's own
  `step_cancel` cancelled it before its attempt began, among the completion's own rows); a timer with
  `HISTORY_TIMER_FIRED`, the record of the function's `timer_cancel`, or a cancelled step outcome from the stop drain; a
  child with `HISTORY_CHILD_FINISHED` (status `CANCELLED`, origin `EFFECT` and no child run when it is cancelled before
  its run starts). A step whose attempt began is not cancelled by `step_cancel` alone: its outcome comes later, with the
  origin `ENGINE`, from its attempt, its reconciliation or its review (Decision 044-245).
- A transaction that creates a run journals `run.started` first, at sequence 1; a Signal-with-Start's
  `signal.buffered` and an Update-with-Start's `update.admitted` follow it (Decision 044-246).
- Closing a saga scope writes only its command record; a saga outcome event (`compensated`, `partial`,
  `compensation_failed`) exists only after an abort.
- The closed list of update rejection reasons: `VALIDATION_FAILED` (the function's validator refused it),
  `HANDLER_NOT_REGISTERED` (the function had registered no handler for the update's name when its `update.admitted`
  was applied), and the engine's `RUN_ENDED` and `RUN_CONTINUED` (Decision 044-251).

A deferred command is chained when it is accepted, with the number it takes then; until then its deferral record holds
`hex(SHA(C(B)))`. A re-emitted `complete` may name its output by its descriptor alone, whatever carrier its deferral
used, or carry it again with the same descriptor; a re-emitted continue-as-new carries its input and state inline
(Decision 044-259). The goldens show the carrier of the first emission; the fake server of `sdks/typescript/test/`
accepts the other forms the rule allows.

## The lease of a run

`commands.json`'s `lease` holds what the run's workflow leases carry besides the history: `gatedEntries` (the run's
current version's gated activity entries) and `stateMaxBytes`, the version's `state.maxBytes` (Decision 044-257): the
declared value, 65,536 when `declarations.state` declares none, and absent (0 on the wire) when the version declares no
state. `generate.mjs` derives `stateMaxBytes` from `declarations`, never by hand, and refuses a carried state over it.

## The deterministic primitives (Technical Plan 1.8)

- The generator is xoshiro256** whose four 64-bit state words are the 32-byte seed read as four little-endian unsigned
  integers; an all-zero seed is replaced by the first four outputs of SplitMix64 started at 0.
- `random()` is `(next() >> 11) × 2^-53`; `randomInt(n)` for `0 < n ≤ 2^32` draws `x = next()` until
  `x ≥ (2^64 − n) mod n` and returns `x mod n`; `uuid()` takes 16 bytes from two `next()` values, each little-endian,
  then sets the version nibble (byte 6, high nibble) to 4 and the variant bits (byte 8, two high bits) to `10`.

## Files

- `vectors/rfc8785.json`: inputs as JSON text and their canonical bytes in hexadecimal (non-BMP keys and number printing
  included).
- `vectors/primitives.json`: seeds in hexadecimal and the first outputs of `next`, `random`, `randomInt` and `uuid`.
- `vectors/chain.json`: command lists with each body, `d(k)` in hexadecimal and `H(k)`.
- `scenarios/<scenario>/` (and `scenarios/<scenario>/<run>/` for the further runs of a scenario: a child, a successor, a moved
  run; a scenario of a deferrable task holds a `PENDING` file until its task, the corpus's form of the suite's
  `.json.pending`): per conformance scenario of the design's Verification Plan, `function.md` (the reference
  function in pseudo-code), `commands.json` (the expected commands per activation), `history.jsonl` (the run's visible
  history as `HistoryEvent` lines in the proto3 JSON mapping of `worker.proto`) and `chain.json` (the chain after each
  activation).
