# Resolution of the confirmation re-reads of decision D041-121 (21e4bd74)

Three read-only confirmation re-reads by the reviewers whose second re-reads had a Blocker or a Major, on a snapshot of
`21e4bd74`, given the diff since `9361a112`. The security reviewer's second re-read had none. They are saved verbatim
beside this file:

| Reviewer | Blocker | Major | Earlier Blockers and Majors |
|---|---|---|---|
| `correctness-reviewer.md` | 0 | 0 | Blocker 1 and Major 1 closed |
| `architecture-reviewer.md` | 0 | 0 | Majors A to H closed |
| `egm-design.md` | 0 | 1 | Blocker 1 and Majors 1 to 4 closed; Major 5 partly closed, restated as its Major 1 |

The answers are point 9 of decision D041-121 (`specs/041-step-logs/research.md`), made before any round of the fourth
run.

## Major

| Finding | Answer |
|---|---|
| Design Major 1: the (2b) rule cannot see the refills' effect on other completions (a refilling report holds the usage row to its commit), and branch (ii) fires for almost any failure | A report whose carry refilled names itself for the rest of its transaction (`spike041carry:refill`), so the sampler counts the commit waits behind it apart (`commit_waits_by_refill`); test `only_a_carry_that_needs_a_refill_opens_a_subtransaction` failed before ("x" against "x:refill") and passes now. The harness prints each completion's time less its own carry's (`complete_minus_carry_p99_ms`). The rule: (i) the refills' own completions, not decided by this run (a fifth run with an ensure in the spike's hook, or the owner); (ii) the carry's own statements when the completions less their carry meet the allowance, a design change; (iii) otherwise the commit queue (behind refilling reports, which the built ensure removes, and the content class's) and the runner's load, not attributable, to the owner with the environment question. No expectation between (ii) and (iii) is declared |

## Minors and Nits taken

- **Medians** (correctness minor 1, design minor 2): each (2b) figure is the median over the three valid rounds used for the gate; `carry_spike_judge.py` computes the medians of (i) and (ii).
- **A round that stops in (h) or (d)** (correctness minor 3): the judge voids it.
- **The tagged count** (correctness nit 2, architecture N11, design minor 3): a row whose `complete_slow_paths_tagged` differs from its refill and mark-only carries has no attribution; the judge checks it.
- **"At most twice"** (correctness minor 2, architecture N5): for the step-log charge, before the commit; a report that stores output also takes the row at its commit (gap G041-024).
- **The fallback counter's source** (architecture N1): a carry that took its block, never one the application planned mark-only.
- **Room from batches and the marker** (architecture N3): they return none; the store forgets the room and the next report runs the ensure; the affinity counter also counts a pid other than the one kept with the connection.
- **The clock** (architecture N4): the calls return the lease's age (`idle_for`), aged by the store's monotonic clock.
- **The reopening thresholds** (architecture N9, design minor 5): `mark_only:busy` carries above 0.1% for question O041-041; carry fallbacks above 1% of carrying reports, or the same busy rate, for option 1c; both carried into task T041-9's criteria.
- **Branch (i)'s way forward** (architecture N8, design minor 7): stated.
- **Stale texts** (design minor 6, correctness nit 3, architecture nits): the budget table's staleness and C0 row, the fallback's causes, the subtransaction risk row, the longest refill wait, Complexity Tracking's metrics, ADR-0097's "its own budget" and "Decided by", the metric owners' duplicates, the acceptance clause's citation.
- **Declaration nits** (design nits 1 and 2, correctness nit 4): *r* and *j* counted from 0; the start pools' setup leases declared; "collisions" restated as the lease's ownership or grant.

## Not taken now

- **A workflow step that runs the lease tests before the rounds** (architecture N6, design minor 4): those tests start their own database and would add a moving part to the gate job; the record names the commit on which they passed instead.
- **The fitness test's selection in CI** (architecture N10): the selector change belongs with task T041-32's step-log rule.
- **The planned charge computed twice** (architecture N2), **a stale-room test with a reused pid**, **the `invalid` bound**, **the lock-order writers**, **the Helm helper and PgBouncer component**, **the T041-9 subtransaction criterion**, **the contract description and drift check**, **"Enforced by" lists** (architecture N7 and carried Minors): tasks T041-36, T041-5, T041-9 and T041-3, which build them.
- **A pause between setup retries** (nits): nothing in the benchmark holds the fence during setup.
- **Option (a) of the busy answer** (passing `busy` to the carry): the owner's, with question O041-041.

## The narrow re-read of the design critic (8b12401f)

`egm-design-narrow.md`, saved verbatim: 0 Blocker, 2 Major, on the answer above.

| Finding | Answer |
|---|---|
| Major 1: branch (ii) still fires for almost any narrow failure, and the refill-holder evidence is read only after it | The refill branch comes first: (i) when the commit waits with a refilling report in their chain are at least half of (a)'s commit waits, or the completions without the slow paths meet the allowance. (ii) is stated as an upper bound: its lever is first measured by a diagnosis and becomes a design change only if its measured reduction covers the excess. (i) no longer claims the built design passes: the fifth run with the ensure before each report is the test |
| Major 2: `commit_waits_by_refill` counts only the head of the queue behind a refill holder | The sample follows each waiter's blocking chain (a recursive query over `pg_blocking_pids`, at most 8 levels) and counts a waiter once when a refilling report is anywhere in it, in the measured database only. Test `the_commit_waits_behind_a_refilling_report_count_its_whole_queue` (two `COMMIT`s queued in turn behind a refilling holder) failed on the head-only sample, (2, 1) against (2, 2), and passes now |
| Minor 2: no reading when the attribution is missing | A row without an attribution makes (2b) at 8 × 128 not decided, with (iii)'s consequence |
| Minor 3: option 1c's trigger cannot fire for (a) | Restated relative to refills: carry fallbacks above half of the carries' refills (ensures answered `made` plus carry fallbacks) |
| Nit 2: a re-run's second carry overwrote the first | The carry times of one step add up |

Not taken: Minor 4 (the new queue test reads `pg_stat_activity` from another backend, which is what the sampler relies on); Nit 1 (the markdown table of a round, whose JSON carries the field).

## The second narrow re-read of the design critic (18898a98)

`egm-design-narrow-2.md`, saved verbatim: 0 Blocker, 0 Major; both Majors of the first narrow re-read closed. Taken:
Minor 1 (the one-half threshold's basis stated: it detects only a dominant refill queue), Minor 2 (a diagnosis that
falls short takes (iii)'s consequence), Minor 3 (the refill share alone takes precedence over a void attribution),
Minor 4 (three queued waiters, each counted once; a hand mutation to `count(c.waiter)` fails it, (3, 4) against
(3, 3)), Nit 1 (52 tests). Not taken: Nits 2 to 5 (wording of the queue, the depth cap, the sampler's cost, the judge's
key names), which change no figure the rule reads.

