# Architecture review of the owner rows and the digest-key force (T022-108, ADR-0093 decision 5; 2026-11-20)

Reviewer: `architecture-reviewer` subagent, read-only, given the working-tree diff with the constitution, AGENTS.md, the
ADRs 0056, 0088, 0089, 0093, 0094, the feature's spec, plan, data model, research and tasks, the migration and ADR
skills and the architecture principles (not the author's conclusions).

## Findings

1. **Major (M1): ADR-0093 decision 5 departs from accepted ADR-0088** (decision 8's drain-first sentence, decision 2's
   order) **and ADR-0056 decision 5** (oldest first), and only ADR-0056 decision 7 is recorded as superseded; the loss of
   order across a split key was not told to the owner.
2. **Major (M2): a key can have more than two rows** (a second force while the first drains), so the accepted bound is
   understated.
3. **Major (M3): the runbook's commands fail as written** (a bare integer does not match a `smallint` parameter), and the
   emergency procedure can commit without doing anything when a rotation is already in progress.
4. **Major (M4): the code now contradicts approved spec text** (FR-022-056, SC-022-028) and G1195 does not list it;
   T022-095's evidence and owner-gate lines are not updated; the force should be its own follow-up task.
5. minor: ADR-0093's decision 1 and option 1 read inconsistently after the amendment, and the one-way rollback after a
   force is not stated (m1); the guard against a new waiter is credited to the wrong mechanism, and two comments still
   say a key is never split (m2); D022-046 claims an acceptance of the ceiling's double count that the owner did not give
   (m3); G022-002's "Accepted" and the OD-2026-11-19-4 row's citation of `contracts/errors.md` overstate (m4); no lock
   warning in the runbook (m5); the reason's size bound (m6); the down migration's round trip (m7); no metric for rows
   under a retired version (m8); removing the leaked key from the ring is untested (m9).
6. nits: the lock barrier counts any waiter and has no deadline; `BEFORE_FORCE`'s comment and a constant assertion;
   blank lines in research.md; ADR-0056's revision paragraph still calls ADR-0089 proposed.
- Answered: adding decision 5 on acceptance is acceptable (ADR-0093 was proposed and decision 5 is O1170 (b) as put to
  the owner); the migration is additive and sorts after every migration seen on other branches; no approved text was
  edited; the owner rows' ids, quote and status are consistent.

## Resolution (author, 2026-11-20)

| Finding | Resolution | Evidence |
|---|---|---|
| M1 | Fixed: ADR-0093's "Refines / supersedes" names ADR-0088 decision 8's last sentence and the bounded exception to ADR-0088 decision 2 and ADR-0056 decision 5; decision 5 and the runbook state the order is kept within each row, not across them; notes added to ADR-0088 and ADR-0056; O1174 puts the reading of the bound to the owner | ADR-0093, ADR-0088, ADR-0056 |
| M2 | Fixed: `FORCED_VERSION_DRAINING` keeps a key at two rows at most | `migrations::the_force_is_the_operators_audited_and_retires_only_a_retiring_version`, red before the fix |
| M3 | Fixed: `::smallint` everywhere; a table of the emergency order per case; read every outcome before `COMMIT`; `SET lock_timeout` | runbook (not executed by a test: recorded under G022-006 with the commands' missing API) |
| M4 | Fixed: G1195 names FR-022-056's clause, SC-022-028 and the table; the force is T022-108 (`Owner-gate: OD-2026-11-19-2`) with its own evidence; T022-095 cites the owner row | tasks.md, research.md |
| m1 | Fixed: decision 1 says "unless the operator forces it out"; the consequences state the one-way rollback | ADR-0093 |
| m2 | Fixed: the migration and D022-046 credit the in-force function and `gate_row_write_errors`; the comments of `keyed_digest.rs` and `flow_control.rs` say a forced key has at most two rows | code comments |
| m3 | Fixed differently: the double count no longer exists (the round counts the versions in force, correctness M2) | `rotation::a_key_filed_again_after_a_force_is_not_refused_by_the_live_keys_ceiling` |
| m4 | Fixed: G022-002 reads "Accepted for one key per transaction; O1173 open"; the OD row cites the runbook and D022-047 | research.md, owner-decisions.md |
| m5, m6 | Fixed (runbook lock advice; the reason bounded as typed) | runbook; the force test |
| m7 | Covered: the rotation migration's test rolls back past `20270202000000` with no force recorded and goes forward again (`the_rotation_migration_retires_what_the_old_command_left_and_its_down_refuses_during_a_rotation`); the refusal with a force recorded is in the force test | both tests |
| m8 | Recorded: G1196 extended to the rows under a forced version | research.md |
| m9 | Accepted, not tested: no release, grant or timeout path reads the ring for a stored digest (the security and correctness reviews confirmed it in `gate_queue.rs` and `readiness_gate.rs`); a test that restarts a process with a smaller ring lands with the deployment wiring of G1198 | review only |
| nits | Fixed: the barrier matches its statement and has a deadline; the constant assertion removed; blank lines; ADR-0056's paragraph says ADR-0089 was accepted | tests, research.md, ADR-0056 |
