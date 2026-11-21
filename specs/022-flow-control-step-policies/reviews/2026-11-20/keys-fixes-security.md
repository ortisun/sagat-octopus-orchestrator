# Security review: the key-ring wiring (G1198) and the fixes of the keys reviews (`f330ff66..295b158b`)

Reviewer: `security-reviewer` (read-only), 2026-11-20, given the diff and the spec only. It read the diff, the three keys
reviews, FR-022-022/053/057/059, NFR-022-002/006, D022-055..060, the gaps table and both OpenAPI documents, and the code
around each change. It ran nothing. (Report as returned, lightly condensed; its closing file list is omitted.)

## Summary

- **Blocker:** none.
- **Major (1):** a hidden label can be rebuilt from the instants. `Run.concurrency.since` and the listing's
  `oldestHolderSince`/`oldestWaitSince` come from the same columns, so a caller without the reveal grant can still link
  runs that share a key (the linking of keys-review security M1).
- **minor (1):** `RunPermit`, `KeyPosition` and `ConcurrencyKeyView` print the full 32-byte digest in `Debug`; everywhere
  else `Debug` shows a digest only as its label.
- **nit (4):** cursors sealed under the newest version a process holds, not the version in force; the `zeroize` claim is
  only partly true; G1199's text is stale against the capability now listed; the canonical `group` parameter has no
  pattern.
- **Fixed from the keys review (checked in code):** M1 (label hidden, fails closed), M2 (cursor sealed), M3 (canonical
  entry), minor 1 (filter test), minor 2 (instant before 1970), nits 2 and 3.

## Major 1: hidden labels can be rebuilt by matching instants between the run read and the key listing

Where: `crates/infra-postgres/src/flow_control.rs:418-423` (`cc_since` is `h.acquired_at` or `w.enqueued_at`);
`flow_control.rs:634-662` (`oldest_holder` is the first `h.acquired_at`, `oldest_wait` is `s.oldest_wait_at` = min
`enqueued_at`); `crates/api-http/src/dto_flow_control.rs:35, 69-70` serialize both to the microsecond whatever the
caller's reveal right.

Scenario: a viewer without a grant reads `GET /runs` (A `HOLDING` since T1, B `WAITING` since T2, both in `customers`)
and the listing (a key with `oldestHolderSince == T1`, `oldestWaitSince == T2`): A and B share one customer, with every
`keyLabel` `null`. A developer without the grant can confirm a guess: start a run with the guessed value and look for a
key whose oldest holder or oldest wait equals the run's `since`; none means the run joined an existing key.

Smallest fix: when the reveal check is false, `null` the two instants on listing items, beside the label, and assert in
the reveal test that a viewer's listing has no instant equal to any `Run.concurrency.since`. A weaker channel remains
(per-key counts and the oldest-wait order); the fail-closed alternative is an aggregated listing per (pool, scope,
group) for such callers. Record the choice next to G1185.

## minor 1: `Debug` of `RunPermit`, `KeyPosition` and `ConcurrencyKeyView` prints the full digest

`crates/application/src/flow_control_keys.rs:28, 93, 112, 125-132`. A `?run` or `?page` field, or an `assert_eq!`
message, writes the digest, whose first six bytes are the label D022-060 hides. No current log prints them (hygiene, not
an observed leak). Fix: a hand-written `Debug` showing the label only, as `FiledKey` and `KeyedDigest` have.

## nit

1. Cursors are sealed under the newest version held (`keyed_digest.rs:278`): during a staged rotation a cursor of a
   replica that has version N+1 is a `400` on one that does not. Seal under the version in force, or document "the same
   file everywhere".
2. `zeroize` only partly holds (`apps/control-plane/src/config.rs:201-203`): `read_to_end` grows the `Vec` by
   reallocation and frees unwiped copies; `decode_key` returns the key bytes by value. Use
   `Vec::with_capacity(LIMIT + 1)`, or reword D022-057.
3. G1199 still says "No process may list the capabilities before these are applied" and is Open, while both binaries now
   list `flow-control-concurrency-v1`; the run waiters per key are bounded (`CONCURRENCY_QUEUE_FULL`). Update the row.
4. The canonical `group` parameter drops the fragment's `pattern` (the server still answers 422).

## Checked with no finding

- Cryptography: cursor key `HMAC(K, "sagat.flow-control.cursor.v1\0")` cannot equal a digest (the schemes differ at byte
  19); ChaCha20-Poly1305 with a random 96-bit nonce; associated data scheme, version, tenant, namespace; constant-time
  tag check; untrusted text checked (prefix, length cap before decoding, even length, lowercase hex, minimum length
  before slicing); an unknown version and a forged `k1.` text are refused; `decode` refuses negative instants.
- Tenant boundary: the cursor is opened with the caller's tenant and the path namespace before admission without the
  database; every new statement binds tenant and namespace; the reveal SQL binds both; the reveal check runs after the
  tenant-filtered read; the cross-tenant 404 is audited and tested; the route is in the battery.
- Authorization: `OpsRead` in `admit` and the service; feature 009's rule (role grants `PayloadReadInternal` and a grant
  row); fail closed (the store and `RunPermit::new` produce `None`; only the two services reveal); the journal DTO
  carries no gate detail; the revocation race is accepted in D022-060.
- Admission and the test door: admitted only switched on, activated and served; a start's `rateLimit` refused;
  `decide_start_gates` refuses without a ring; `serving_rate_gates_for_tests` behind a dev-only feature and the
  architecture rule; `BUILT` holds only the concurrency gate.
- Key material: a file named by a variable, no raw-value variable; bounded read; errors name the variable only;
  `KeyRing` and `FlowControlKey` redact themselves (tested); the startup refusal logs versions only; Helm mounts an
  `existingSecret` 0440 and fails without it; compose overlay with `create_host_path: false`, the file ignored by git.
- Startup refusals: `Missing` without a file; `Invalid` for unreadable, oversized, non-text, malformed, duplicated or
  empty rings; a version in force missing from the file stops both binaries before registration.
- Least privilege: `sagat_app` has `SELECT` on `memberships` and `payload_reveal_grants` and `EXECUTE` on
  `flow_control_key_versions_in_force()`; the replaced trigger keeps `SECURITY DEFINER`, its pinned path and its revoke.
- Injection and cost: literals with bound values; group parsed; limit clamped; index walk with a 2 s timeout mapped to
  503; a failed cursor costs one AEAD open before admission.
