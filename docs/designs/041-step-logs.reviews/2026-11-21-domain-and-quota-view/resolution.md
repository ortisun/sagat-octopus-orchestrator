# Resolution of the review of task T041-4's slice (2026-11-21)

The `security-reviewer` and the `correctness-reviewer` read the diff of `c1fc1206` (the domain's carry plan and the
quota view's withheld usage) against the design; their reports are `security.md` and `correctness.md`, verbatim. No
Blocker; two Majors (correctness) and nine Nits. Each is answered by decision D041-140 of
`specs/041-step-logs/research.md`:

| Finding | Answer | Test |
|---|---|---|
| correctness Major 1, an empty batch accepted | `BatchInvalid::Empty`; an empty carry stays valid | `a_batch_is_refused_outside_its_bounds` (seen failing: no such bound); the property test's batches hold 1 to 19 records |
| correctness Major 2, the mark-only gap after a ceiling | data-model.md §8a item 5 corrected to the code's reading, the gap of T5 | `a_mark_only_carry_after_a_ceiling_splits_its_gap_as_t5_does` |
| correctness Nit 1, the reasons of unkept records not pinned | the byte test asserts `attempt_bytes`; the property test requires every server reason to be one the rule names | hand mutation (every unkept record `attempt_records`): 2 tests fail |
| correctness Nit 2, no mark-only record above `highest_sequence` | the property test draws a mark-only carry's `highest_sequence` apart from its records | the property test |
| correctness Nit 3 and security Nit 1, the permission, the membership role and the tenant not decisive in the test | three cases added: a viewer's token over a member with both grants; a viewer member with grant rows and a developer's token; a member of both tenants with the grant only in the other | hand mutations: `run:logs:read` not checked; the membership role not checked |
| security Nit 1, a tenant with no namespace | withheld by the query; not tested, since such a tenant has no run and a usage of 0 | none |
| security Nit 2, the grant read and the usage read are two transactions | accepted, D041-140 (5) | none |
| security Nit 3, `run:read` not checked | checked, as defence in depth | none: no role holds `run:logs:read` without `run:read` |
| security Nit 4, `plan_batch` wraps on an unvalidated input | the gap saturates | none (unreachable through the validators) |
| security Nit 5, a platform metric raised by raw drops | for T041-6 and T041-36: the metric takes the saturated change of the stream's counts, D041-140 (7) | T041-6 |
| correctness Nit 4, a store error fails the whole view | kept, fail-closed, stated in the method's doc | none |

The correctness reviewer re-read the fixes at `7dd4ea3c` (`correctness-rereview.md`): both Majors and the Nits
resolved, no new Blocker or Major; its two new Nits are answered by D041-140 (4) (`run:read` as defence in depth, the
contract text with task T041-3) and by the property test's corrected doc comment.
