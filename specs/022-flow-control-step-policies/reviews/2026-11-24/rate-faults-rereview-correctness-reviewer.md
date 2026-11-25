# Correctness re-review of the answers to the review of task T022-080 (2026-11-24)

Reviewer: the repository's read-only `correctness-reviewer`, given commits `d74e99be` (decisions D022-073, D022-074,
D022-075) and `16814c42` (their code and tests), the earlier review `../2026-11-21/rate-faults-correctness-reviewer.md`,
the specification, ADR-0088 and ADR-0089. It read the tree at `ad8cf6b1` and ran nothing. Three Majors, seven minors,
two nits. The answers are decision D022-076 in `research.md`.

## The earlier Majors

- Major 1: closed. Round 0 reaches the grant's deadline check under the window's lock (`gate_queue.rs:305`); round 1
  asserts `skipped == 1`.
- Major 2: closed. The moved grant is five windows ahead and `gates_granted == 0` is asserted without a condition.
- Major 3: mostly closed; what is left is minor 2 below.
- Major 4: closed for the grant routine; the admission side is untested (minor 3).
- Major 5: closed by decision D022-073, which opens Majors 1 and 3 below.
- Major 6: closed. The held journal row stops the deciding transaction after its gate writes.

## Majors

1. **A start queued behind an eligible waiter while a pass works on its key loses its wake-up**
   (`readiness_gate.rs:1492`, `:1498`; `maintenance.rs:254-258`, `:281-321`; `passes.rs:5672-5721`). With room,
   `key_due` lowers the hint only when it is null or later than `clock_timestamp()`; while a pass is on the key the hint
   is already at or before now and stays, so the pass's rest finds the hint it selected and rests the key (a semaphore
   to null, a window to its counted grants' age-out). The new start was not in the pass's listing. Permit case:
   `limit: 3`, two holders end, waiter W queued; the pass lists W; start N queues behind W; the pass grants W and rests
   the key; N waits for an unrelated release or its queue timeout. Slot case: `limit: 3, per: hour`, the rest sets the
   hint an hour ahead. Breaks ADR-0089 decision 3 ("never late") and the bound of decision D022-073 item 5. Test: hold
   W's semaphore row, run a pass with `concurrency: 1`, start N at the pass's barrier, release; the next pass must grant
   N (today 0).
2. **On a key shared by several window lengths, a waiter queued at a full key keeps a hint computed for longer
   windows** (`readiness_gate.rs:1484-1498`, `:1705-1712`; `maintenance.rs:313-316`). Key K shared by A (1 an hour) and
   B (1 a second): HA granted at t0, Wa queued, the rest sets t0 + 1 h; HB granted at t0 + 10 s; Wb queues at
   t0 + 10.5 s at a full window and the hint stays t0 + 1 h; Wb is grantable at t0 + 11 s and times out at ten minutes.
   Breaks "early or the age-out instant, both never late" (decision D022-073 item 4).
3. **A reset is refused while its key has room** (`readiness_gate.rs:366-381`, `:479-481`; FR-022-041; SC-022-022).
   Strict order makes a gate with room and an eligible waiter refusing, and `start_may_wait` refuses any cause that may
   not queue: `limit: 3, per: hour`, one slot counted, one eligible waiter (a dual-gated waiter blocked by its permit
   stays eligible indefinitely): a reset is refused `RATE_LIMIT_SLOT_UNAVAILABLE` although `1 + 1 < 3`. FR-022-041
   refuses a reset only "without a free slot".

## minors

1. Order within a pass breaks when rows are deleted rather than aged (`gate_queue.rs:238-262`): two frees committed
   between the refusal of X and the decision for a younger Y of the same limit let Y be granted first (FR-022-042).
2. The rate place still diverges from the grant routine: `RATE_PLACE_SQL`'s `ahead` (`flow_control.rs:1049-1055`)
   counts waiters the grant routine does not count; the run listing returns the filed limit for a `WAITING` run
   (`flow_control.rs:449`), as `get` does when the place read fails (`runs.rs:2374`).
3. Admission-side mixed windows are untested: `c.ms = w.period_ms` changed in `has_eligible_waiter`
   (`readiness_gate.rs:835`) survives every test.
4. Decision D022-074's recorded limit is never asserted: binding `q.limit` instead of `decided(q)` for the permit
   holder (`gate_queue.rs:348`) survives the lowered-ceiling test.
5. The permit strict-order test's hint check cannot fail (`run_admission.rs:748-752`).
6. The lowered-ceiling fault test depends on the runner's speed (`rate_limits.rs:1234-1244`): five starts within one
   second; it fails loudly rather than passing falsely.
7. A raised ceiling can starve a filed waiter until its timeout (more than the "under-delivery" decision D022-075
   item 7 names).

## nits

1. The `counted` common table expression (`gate_queue.rs:238-249`, `readiness_gate.rs:823-832`) counts up to three
   windows per waiter tried, up to 256 a key a pass; no budget line covers it.
2. ADR-0089 decision 4 is headed "replaces strict oldest-first"; decision D022-073 brings strict order back at
   admission: the ADR's text should say so.

## The reviewer's answers to the six questions

Strict order is written as specified at both gates and both levels and never over-admits; it has the liveness holes of
Majors 1 and 2 and the reset drift of Major 3. Counting as of the selection never over-grants and cannot refuse
forever. Windows are half-open `(T - p, T]` everywhere, no off-by-one. Lowered ceilings are applied at grant and
admission, nothing revoked, the decided limit recorded and checked by `verify_run`. The new reads take no new locks and
stay in the deciding transaction. Each new test fails without its fix, and no race uses a sleep, except the weak checks
of minors 3, 4 and 5.
