# Bounded quota parent closure audit

Read-only GPT-6 Astra/xhigh review plus root source/evidence inspection. No new
native tests were run for this audit. The affected under-limit/ordinary cancellation
regression is accepted separately before checkbox updates. Existing matrices are
separate executions with overlapping coverage; their counts must not be summed.

## Q7c — path/frontier accounting and same snapshot

| Obligation | Accepted source/evidence |
| --- | --- |
| Exact path positive-write guard and effective E=L+V | b0a7ea5e; new charges remain L only |
| Same-snapshot revision/stop/code/E | scope_frontier.rs one read-only REPEATABLE READ transaction, same connection for header and scope/member reads |
| Old pending nonfinal Return | b8f44488, legacy-nonfinal-return-acceptance-20261023; real compensation/native finish |
| Old free-admission scope | 528e10fd, legacy-free-admission-acceptance-20261023; no admission/new obligation |
| Only-live-work Waiting bypass | 59618184, legacy-live-frontier-acceptance-20261023; isolated old RED and exact current DrainStarted |
| Exact equality E4194304 | 52c827c7, legacy-equal-limit-acceptance-20261023; exact Admitted0/native delta |
| Selected real failure/cancellation priority | 21294435, legacy-failed-member-acceptance-20261023; genuine failed member custom stop, preserved public CANCEL regression |
| Stale frontier CAS/fresh decision | f821de85, legacy-frontier-cas-acceptance-20261023; observed real locks, exact RevisionConflict105→107, no stale effects, fresh stopped drain/real compensation |

This meets the explicit bounded Q7c/§11.3–11.5 accounting/snapshot/frontier gates.
It does not activate production source certification/scoped Finish or close broad
quota, report recovery, rollout, nesting or canonical feature tasks.

## Q7b.5 — current ordinary-inline public refusal COMMIT loss

Accepted .5a–d end at426629d1. Root re-read six final gate JSON/raw harness results
in ordinary-public-aborted-loss-20261023: aborted loss, committed loss, unarmed
reopen baseline, each default/runtime invocation, all1passed/0failed/0ignored exit0.
Both invocation profiles ACTUALLY use sagat_app; no owner/runtime privilege matrix.
Production and the public recovery test modules are unchanged since426629d1.

Actual committed/deferred-aborted COMMIT reply loss uses exact receipt/COMMIT gates
and same-PID observation, typed UNAVAILABLE, unresolved-state/Run-lock witnesses,
complete aborted rollback/live original capability, full API/store/auth-cache/pool
reconstruction, original request/key/session reuse, canonical committed replay or
aborted first apply followed by exact replay. No new public unknown-outcome enum.

The original public-pool-bootstrap-decision-20261018 plan proposed ordinary AND
Applied P1 recovery, but its review explicitly left the full P1 context unaccepted.
The later accepted .5c/.5d contracts select ordinary-inline and leave Applied
separate. Thus .5 closes expressly CURRENT ORDINARY INLINE only. Broader Q7b/Q10c
still require Applied, remaining authority/fence/terminal controls, post-ACK cold
scheduler/OS recovery and genuine-old/output-dependent/recovery compositions.
No evidence for those remaining controls is inferred from this parent closure.
