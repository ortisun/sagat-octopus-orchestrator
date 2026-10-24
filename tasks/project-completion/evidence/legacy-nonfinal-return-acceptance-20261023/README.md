# Genuine old pending nonfinal Return — acceptance

This closes only the selected pending-nonfinal-Return frontier case from the
immutable reviewed design (`15c27e05`, plan SHA
`da44b483749740913bd436a19b9c74be3e012cebeba7eb270b5642d2683e7c42`).
Immutable old production: `6c44a0e49670cf19660792cd1518d241e6c99b3b`.
Current baseline: `403464e8` plus the recorded source diff/four-file hashes.
No production, schema, grants, SDK, protocol or activation change.

Ten genuine old closed scopes retain twenty public 105000-byte outputs. Each
old aggregate is 210003 bytes; identical digests count per owner reference.
c10[0]/work publicly succeeds with JSON `1`; its Return is still PENDING and the
second item is unadmitted. L=2100001, V=2100030, E=4200031>4194304.

Current migration preserves the complete original projection and closed/target
source hash. The actual frontier binds the original scope and successful work,
pending Return, one active member and completed+1<item_count. One normal advance
starts first FAILED/RUN_DATA_LIMIT and native DRAINING before checkpoint/admission.
Exact journal envelopes, original trace, existing_outputs audit and public FAILED
hint are asserted before any further action; source, ledgers, counters, steps,
checkpoints, receipts and payloads remain unchanged.

Normal drain cancels Return and leases exactly one original-target/key line.release
with `/steps/work/output=1`. An extra advance while it is held returns Waiting.
Real public compensation success leads to COMPENSATED Run / FAILED RDL scope and
owner / DONE-COMPENSATED saga. Exactly the admitted member is DRAINED, all active
slots/reservations zero, no new forward identity/End/checkpoint/payload/backfill.
Exact finish journal/outboxes/telemetry and fresh verify/terminal pure replay pass.
DRAINED settlement increments completed_count to1; the original design's terminal
0 was incorrect. The initial and first-DrainStarted count stays0.

Final native coordinator results (each positive phase exactly one passing test):

| Selector | Phase | Harness seconds | Outcome |
|---|---|---:|---|
| nonfinal-return | old owner | 9.54 | 1 passed |
| nonfinal-return | current owner | 1.18 | 1 passed |
| nonfinal-return | old runtime | 10.04 | 1 passed |
| nonfinal-return | current runtime | 1.17 | 1 passed |
| output-view | producer / owner label | 10.39 | 1 passed |
| output-view | consumer / owner label | 30.27 | 1 passed |
| output-view | producer / runtime label | 10.58 | 1 passed |
| output-view | consumer / runtime label | 30.34 | 1 passed |
| nonfinal-return-red | old owner | 8.53 | expected failure, exit101 |
| nonfinal-return-red | old runtime | 8.62 | expected failure, exit101 |

The positive coordinator takes132.05s including sequential immutable-old/current
builds. The preserved output-view regression takes186.24s and the isolated RED coordinator127.55s. Old/new nonfinal API and execution-store pools assert actual
postgres/postgres or sagat_app/sagat_app. The existing default output-view producer
always uses sagat_app through its immutable Flow constructor; owner/runtime there
continues to describe the current consumer, preserving earlier evidence meaning.
Expected REDs are failing Rust tests (exit101), separately recognized by their
exact Checkpointed marker/count; the outer diagnostic coordinator exits0 only
when those expected failures are observed. They never become passing Rust tests
or current-consumer inputs.

The repository-declared Compose PostgreSQL service on exclusive loopback65118 is
the only server used. Private target: outside every checkout. Exact archive,
restored source/migration manifests, compiler/lock and executable hashes, raw
phase logs, selected entries, safe receipt hashes and created database names are
retained. Six final coordinator-created databases were removed (two per selector). Private receipt files and archived source directories
are removed; snapshot-before/after proves no final coordinator DB remains.
All1127 native input hashes and four fixture input hashes stay identical across
final gates. Strict integration Clippy15.56s, fmt4.84s, architecture0.49s, shell
syntax and source/doc whitespace pass. check-agent-config.py is absent in this
checkout and is not claimed as passing.

Producer C1-C3, first-guard C1-C3 and terminal C1-C2 findings were reconciled; final
fresh reviews None. Whole coordinator review stopped after3cycles and two smaller
units followed the owner's approved decomposition: helper flag-only waiting review
None; credential ordering C1-C3 final None. Root reproduced actual held-wait-lock
helper deadlocks before fixing; corrected lock probes and TERM-resistant children
pass. Six no-service cleanup branch probes pass, including missing established
receipt, ambiguous DB difference and hash failure refusing DROP. These probes are
not real-PG acceptance or Rust regression REDs. Implementation/execution by root;
consequential read-only reviews by GPT-6 Astra/xhigh. Sol execution was unavailable
and was not silently replaced. The optional external review was previously declined.

The first positive/RED coordinator runs under the earlier supervision version are
retained as pre-hardening evidence, distinct from these final native runs. Earlier
exploratory current databases were independently inspected by owner SQL (both
COMPENSATED, exact bytes/head/slots/source checks); isolated old RED databases show
one genuine checkpoint and WAITING with no stop. Those are not the ephemeral final
coordinator databases, which were already dropped. Initial compiler field-name and
owner-inspection assumptions/errors are preserved and explained in companion
consumer evidence; no business state was patched to make any test pass.

Reproduce using `tests/fixtures/legacy-collection-output/README.md`. Raw commands
and bounds are in the JSON gate metadata. Parent Q7c/Q8/Q9/master quota correction,
other frontier/CAS/competing-stop/rollout/nesting cases and the eight canonical014
checkboxes remain open. This does not establish provider-side undo or O(window)
performance, and does not activate source certification/scoped Finish production.

After final coordinator inventories were sealed, root separately removed the five
exact owned exploratory databases and their private handoffs (27→22, every other
database unchanged). Only this unit's archived source/binary copies were deleted;
source manifests, logs and hashes remain. The private ongoing-task target remains
available for the next authorized increment. See exploratory-owned-cleanup.json.
