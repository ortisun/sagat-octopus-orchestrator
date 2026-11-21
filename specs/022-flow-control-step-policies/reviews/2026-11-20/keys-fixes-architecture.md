# Architecture review: the keys fix-up and G1198 (`f330ff66..295b158b`)

Reviewer: `architecture-reviewer` (read-only), 2026-11-20, given the diff and the documents only. It ran nothing.
(Report as returned, lightly condensed.)

## Verdict

Layering is sound: cursor sealing in `sagat-application` beside the HMAC machinery; label disclosure decided in the
application services, the store returning hidden labels; the key file read in `apps/control-plane/src/config.rs` and
reused by the engine; the domain untouched; the migration additive (CHECK `NOT VALID` then validated, grants covered,
safe down); `runtime_privileges`' macro resolution sound with a fixture; D022-057..060 weigh real options. The canonical
OpenAPI merge breaks three gates that pin or derive from that document. Count: 3 Blocker, 3 Major, 13 minor, 8 nit.

## Blocker

- **B1.** `tests/contract/payload-classification/openapi.rs:343-390` digests every operation not marked `x-implemented`
  016/017/018 or `x-planned` 035; `listConcurrencyKeys` (`x-implemented: '022'`) changes the pinned digest. Fix: exclude
  `'022'` with a comment as for 017/018; never recompute the digest.
- **B2.** `docs/reference/api.md` lacks the new row: `contract_developer_experience` fails. Fix: bless after reading the
  one-row diff, rerun without the switch.
- **B3.** `frontend/apps/studio/src/api/schema.d.ts` lacks the new operation and schemas: `npm run check:api` fails, and
  `scripts/select-test-targets.py` sets `frontend=true` only for changes under `frontend/`. Fix: regenerate and commit;
  make a change of the canonical OpenAPI select the frontend checks, with a test.

## Major

- **M1.** Feature 009's reveal rule has a second implementation (`flow_control.rs` `reveals_personal_data` re-writes
  `reveal_grants.rs` `has_effective_payload_reveal_grant`). Fix: one unlocked function in `reveal_grants.rs` called from
  `flow_control.rs`, and a fitness rule that `payload_reveal_grants` is read only in `reveal_grants.rs`.
- **M2.** G1198 is closed on evidence that does not exercise the composition or the startup refusal:
  `check_flow_control_keys` and `missing_key_versions` have no test; `process_env` is never called though D022-057 cites
  it. Fix: a real engine started with `process_env()` after `activate_concurrency` registers, lists the capability and
  lets the activation pass; a database whose active version the file lacks makes the start fail naming the variable.
- **M3.** The capability is now listed while G1199 says "No process may list the capabilities before these are applied";
  T022-104 (refuse `secret` key locations) is unbuilt and not weighed; D022-037 is not marked amended. Fix: amend G1199
  (concurrency half met), say in D022-057 how T022-104 gates activation, annotate D022-037.

## minor

- m1. `RunStore: FlowControlKeyReader` makes every run store implement the listing, a disclosure query and an accessor
  that hands the key ring to any service; a narrow reveal port and the cursor keys set at the composition root would fit.
- m2. Cursors sealed under the newest held version: a 400 across replicas during a rolling addition; seal under the
  version in force.
- m3. Without a ring a page that has a next one is a 503; say so in the contract and the runbook.
- m4. The zeroize claim is broader than the code (reallocation, `KeyRing::parse` intermediates).
- m5. The chart's new rule has no case in `contract_scalability_ha`.
- m6. The plan test covers custom plans only; add `force_generic_plan` and a 503-on-timeout test.
- m7. `queued_runs_count_errors` is evaded by an alias or a `concat!` split.
- m8. `ConcurrencyKeyView.label` is a public field the adapter could set; make it private with a reveal method.
- m9. Derived `Debug` prints the full digest on `RunPermit` and `KeyPosition` (NFR-022-002 covers logs).
- m10. The compose recipe (`umask 077`) makes the file unreadable by the image's user 10001 on Linux.
- m11. No live conformance test of the new operation; no route-to-contract parity check.
- m12. Keys-review correctness m5 and part of m8 are neither addressed nor recorded.
- m13. D022-057 sets a deployment interface that becomes public with 0.3.0; add a dated note to ADR-0093.

## nit

n1 the switches parsed twice (`config.rs:701`); n2 `zeroize` not a workspace dependency; n3 `missing_key_versions`
re-exported as if test-only though production calls it; n4 the canonical `group` lacks the pattern and the tags differ;
n5 `STEP_GATES_DOOR` now polices the rate door, rename; n6 `Options::flow_control` still opens the rate door on the API
store; n7 the control plane sorts its capabilities and the engine does not; n8 D022-055 items 2 and 4 do not point to
D022-059 and D022-060.

## Fitness map (condensed)

| Rule | Check | Gap |
|---|---|---|
| Layers inward, no cycles | `check-architecture.py` | none |
| Rate door test-only | `TEST_ONLY_FEATURES`, door rule | n5 |
| Listing iff built, switched on and keyed | registry unit tests | composition untested (M2) |
| A version in force missing from the ring stops the process | none | M2 |
| Admission admits only switched on, activated, served | `admission::…` | none |
| Labels only to callers who may reveal | `rest::a_key_label…` | M1, m8 |
| Cursor sealed, bound to its namespace | unit and REST tests | m2 |
| A page costs its own keys | plan test (custom plan) | m6 |
| `key_consistency` fails on schema drift | `consistency::a_renamed_column…` | none |
| Canonical contract additive; derived artifacts follow it | baselines | B1, B2, B3 |
| Cross-tenant 404, audited | battery, `rest` | none |
| Admission takes the counter row before the key | `concurrency_locks::an_admission…` | none |
| The chart refuses the switch without a Secret | none | m5 |
| One implementation of the reveal rule | none | M1 |

No duplicated work: the only other 022 branch, `feature/022-sdk-builders`, has none of the new symbols.
