# Targeted re-read 1 of design 052: egm-comprehension (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 299cd5afe72ec104131614e2d655a7546a0a9ead
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`comprehension_report.md`, not a full review.

Full round: Majors 1, 4 to 13 resolved; Majors 2 and 3 partly (Nit 2 below, and where the pool-refused rows fall);
Nits 3 to 12 and 14 resolved, Nits 1, 2 and 13 partly.

## Major

1. Requirement 052-041 reads two ways: "probes that did not meet contention reset nothing" makes a lease not reset the
   count, so "consecutive" is either in a row or in total. Fix: a lease resets, a refusal neither counts nor resets;
   whether the second fetch starts at zero.

## Nit

1. The Technical Plan keeps the flag on the in-flight entry; the requirement keeps it in a contended map.
2. ADR-0133's Consequences say "a poll alone keeps the oldest-first order".
3. "Stream permits" reads as wake permits.
4. "Each of the three server remedies" after a list including the gate.
5. Nothing says the instance identifier is random.
6. "Fails on nine of ten" against a probability for nine or more; what if ten pairs still lack power.
7. "The ChildRoot capacity guard's suite" unexplained.
8. ADR-0133's Enforced by omits two tests and a comma.

Counts: Blocker 0, Major 1, Nit 8

All nine were fixed at the next commit, with the pool-refused rows, the wake's counters and the bare terms of the
first round.
