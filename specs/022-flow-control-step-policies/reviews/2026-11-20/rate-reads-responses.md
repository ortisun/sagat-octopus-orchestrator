# Responses to the reviews of task T022-078 (the rate gate's reads, 2026-11-20)

Reviews: `rate-reads-security-reviewer.md`, `rate-reads-correctness-reviewer.md`, `rate-reads-architecture-reviewer.md`.
Decision D022-067 in `../../research.md`. Task T022-078 is not ticked while a Major below is open.

| Finding | Resolution |
|---|---|
| Security Major 1, architecture Major 4, correctness minor 3 (the estimate links runs for a caller without the reveal grant) | Fixed (decision D022-067 item 8): the estimate is withheld from such a caller; the position stays and is recorded under gap G022-014. `rate_rest::a_run_shows_its_rate_slot_and_a_waiting_run_its_place` reads as a member without the grant, red under the hand mutation that drops the check |
| Security nit 1, nit 2 (the charged-route inventory, the API reference) | Fixed: `docs/runbooks/api-rate-admission.md` row; `docs/reference/api.md` regenerated after reading its diff (one operation added) |
| Correctness Major 1 (the k-th lease for k of 2 and more untested), Major 2 (the filters on the waiters ahead untested), Major 3 and architecture Major 3 (no check of `oldest_wait_at`'s value) | Open: next change (a `limit: 2` test with two holders and three waiters; paused, ended and timed-out waiters ahead; a `key_consistency` row for `oldest_wait_at` with a tamper test; a unit test of `estimated_grant`) |
| Architecture Major 1, Major 2, security minor 2 (the page cost has no check; `nextSlotAt` scans every grant and every waiter of a key) | Open: next change (an index-usable `nextSlotAt` and a bounded set of periods, with the `EXPLAIN` twin of `rest::a_page_of_keys_reads_only_its_own_keys`) |
| Architecture Major 5, correctness minor 1 (no test of the estimate of a dual-gated run, nor of `waitingOn: concurrency`) | Open: next change |
| Security minor 3, architecture minor 5, correctness nit 3 (the place's read has no timeout and can fail the whole read) | Open: next change (a read-only transaction with the listing's timeout; on failure no place) |
| Correctness minor 2 (a paused or timed-out run still gets an estimate), minor 4, minor 5, minor 6; architecture minors 1 to 4, 6 to 8; the nits | Open: next change, or recorded there |
