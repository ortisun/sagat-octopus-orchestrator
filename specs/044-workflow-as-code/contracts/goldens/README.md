# The golden corpus of workflow as code (design 044, Task 044-30)

The corpus is what the six SDK runtimes, the server's command chain and the conformance suite agree on. It is produced by
`generate.mjs` (Node 20 or later, no dependency: `node:crypto` for SHA-256, `BigInt` for the generator, its own RFC 8785
serializer), so the goldens never come from the Rust code they check; `contract_workflow_as_code::the_goldens_parse_and_their_chains_recompute`
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
| `activity`, `script`, `child` | the fragment object with the key `"input"` set to `D(input)` |
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
| `update_complete` | `{"admittedSeq": n, "result": D(result)}`, or `{"admittedSeq": n, "failureCode": code}` for a failed handler |
| `consume_signals` | `{"signals": [{"name": name, "seqs": [ascending sequences]}, ...]}`, the names in ascending UTF-16 order |
| `complete` | `{"output": D(output)}` |
| `fail` | `{"code": code, "message": D(message)}`, the message a JSON string (it is stored only as a `pii` payload) |
| `continue_as_new` | `{"input": input, "state": state or null, "version": "same" or "active"}` (input and state always inline) |
| `acknowledge_cancellation` | `{}` |

A deferred command is chained when it is accepted, with the number it takes then; until then its deferral record holds
`hex(SHA(C(B)))`.

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
- `scenarios/<scenario>/`: per conformance scenario of the design's Verification Plan, `function.md` (the reference
  function in pseudo-code), `commands.json` (the expected commands per activation), `history.jsonl` (the run's visible
  history as `HistoryEvent` lines in the proto3 JSON mapping of `worker.proto`) and `chain.json` (the chain after each
  activation).
