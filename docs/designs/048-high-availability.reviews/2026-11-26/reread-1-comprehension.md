# Targeted re-read 1 of design 048: egm-comprehension (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 853ac19a69cfd090bb791d9ffcaac1bc89cc4df2
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`comprehension_report.md`, at 1e283510), not a full review.

Full round: Majors 1, 2, 4, 5, 6, 7, 9, 10 resolved; Major 8 moot; Major 3 partly (how the drill reaches the primary;
where the negative control runs); Nits 1, 2, 5, 7, 8 resolved; Nits 4 and 9 moot; Nits 3, 6, 10 partly (a cell
"initialized", design 053's lint, what "a statement that writes" is, the throttle's values).

## Major

1. Fault (2) stops "the platform's containers that share its host label", a label defined nowhere.
2. Success criterion 048-003 does not say whether its clauses hold on every repetition or only on qualifying ones.
3. (Full-round Major 3, partly.) As above.

## Nit

1. E and "the next probe" are Kubernetes terms; say what they are on Compose.
2. Success criterion 048-001 omits fault (3)'s counting from the standbys' return.
3. The worker heartbeat is at most 10 s apart, but the criterion uses 12 s.
4. The connection limit "equal to" a budget the chart does not define; "registry's connection" and "registry pool".
5. Where the probe's `pg_hba` line lives is said two ways.
6. "Task 048-11 before Task 048-14 gates ..." does not parse; "Tasks 048-11".

Counts: Blocker 0, Major 3, Nit 6

All nine, and the partly resolved Nits, were fixed at `a29b72b2`.
