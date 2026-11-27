# Reviews of the batch `bdd0a3a9..bb756cfb` (T022-109 held signals, decision D022-088, the batch-3 and batch-4 Majors), and the rulings (2026-11-25)

The repository's read-only `correctness-reviewer` and `security-reviewer` read the code at `bb756cfb`. No Blocker.
Correctness: 2 Majors, 5 minors, 2 nits. Security: 3 minors, 2 nits, 4 suspicions. Every ruling is delegate
(OD-2026-11-24-4), documents first, built with a test seen failing first.

## Majors (correctness)

| Finding | Ruling |
|---|---|
| Major 1: a waiter whose grant always fails (a version this binary cannot load, an execution error, a constraint the holder insert or the held signal's delivery violates) ends its key's pass on every pass, so younger waiters of the key wait until that waiter's queue deadline (`passes.rs`, `Granted::Failed`) | Only a busy run and a failure classified transient (pool or statement timeout, serialization, deadlock) end the key's pass; any other failure skips that waiter as before, is counted and logged, and the waiter's run is reported by the orphan or a new failure alert; a held signal whose delivery fails is dropped with its audit reason instead of failing the grant (security suspicion 2). Test: a trigger that refuses one waiter's holder insert; the younger waiter is granted within one pass |
| Major 2: `grant_waiter` returns a refusal by dropping its transaction; the pool rolls it back later in another task, so the run's lock lingers and a second waiter of the same run (two root nodes of one run on one key, or a start gated on two keys) finds it busy and ends the key's pass | Every return that does not commit rolls back explicitly and awaits it (`RunTx::rollback`). Test: two waiters of one run adjacent on one key, the first refused by its other gate; a younger waiter of another run is granted in the first pass. The intermittent `run_admission::a_cancellation_dequeues_a_queued_start_and_frees_a_held_permit` is re-examined against this cause |

## minors

| Finding | Ruling |
|---|---|
| Correctness minor 1: a Signal-with-Start whose signal no entry-path step waits for, or whose payload exceeds the output bound, is refused when the key has room but answered `held` and later dropped when it queues | The creation transaction computes the targets and the output bound as the delivery would and refuses the same way in both cases; a held signal is only ever one that the grant can deliver. Tests for both refusals while the start queues |
| Correctness minor 2: the grant of a queued start locks the active-run counter row even when it then refuses | The row is locked only when the definition's entry can end the run in the grant's transaction (a property computed at publication); the plain case takes no extra lock. Test: the existing lock-order case plus a held counter row that does not block a refused grant of a start that cannot end |
| Correctness minor 3: nothing checks that a held row belongs to a queued run | `verify_run` and `assert_consistent` check that a held row exists only while the run's start waits; the drop on a run's end applies whenever a run ends whose start never ran (security suspicion 1), not only on a commit with a dequeue or timeout event |
| Correctness minor 4: ADR-0089 decision 4 does not mention the busy and transient-failure hold-back of D022-088 | A dated amendment under ADR-0089 decision 4, delegate (OD-2026-11-24-4) |
| Correctness minor 5: a group's sums can span pages, so a group may report holders above its limit | A `spansPages` flag on a group row whose sums were read across pages, documented in the OpenAPI |
| Security minor 1: a held signal is delivered under the caller's name without a fresh permission check, unstated for signals | FR-022-060 and D022-087 say it in words: a held signal is not re-authorized, like the start it rides with; cancelling the run is the remedy; T022-106 gains a held-signal case that revokes `run:signal` and asserts the delivery and the audit subject |
| Security minor 2: a dropped held signal leaves no record of who sent it | The drop's audit record names the held row's actor |
| Security minor 3: two doc comments say a signal is never buffered (`routes/execution.rs:794`, `runs.rs:1034`) | Corrected |
| Security suspicion 3: a cursor replayed later mixes old partial sums with current rows; a page boundary reveals whether a group crosses 200 keys | The sealed cursor carries its issue instant and is refused after fifteen minutes (`400`, a cursor that expired) |
| Security suspicion 4: a signal waited for only inside a body may be held and then always dropped | Covered by correctness minor 1: held only when a root step of the entry path waits |

## nits

| Finding | Ruling |
|---|---|
| Security nit: no metric for held, delivered and dropped signals | A counter with a closed outcome set (`held`, `delivered`, `dropped_not_waiting`, `dropped_output_bound`, `dropped_run_ended`), no tenant or run label |
| Security nit: the three new allowed entries of the privilege audit cannot see a table added to `effective_sql` later | A test that runs the alert detection as `sagat_app` (the alert tests under `SAGAT_TEST_AS_RUNTIME_ROLE=1`) |
| Correctness nit: no fault test injects a commit failure after the held signal's delivery or races its timeout against its grant | Both fault cases |
| Correctness nit: `effective_sql` guards `IterationConcurrency` with a `debug_assert` only | A `Result` (or a type that cannot name it) |
