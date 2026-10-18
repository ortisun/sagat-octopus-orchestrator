# Genuine old-produced collection discovery counterexample

Current base: `3710a5b7`, plus the executed one-file test patch retained here. The immutable
old producer is `6c44a0e49670cf19660792cd1518d241e6c99b3b`. The unchanged coordinator
`scripts/test-legacy-collection-output-view.sh` built both binaries before starting the old
producer, waited for its process exit, and ran the new normal Keeper/Scheduler consumer.

Use the environment from the preceding
[combined integration evidence](../collection-finalization-integration-20261018/reproduce.md),
plus `SAGAT_Q5_OWNED_POSTGRES=1` and an owned `SAGAT_Q5_ARTIFACT_ROOT`, then execute that script.
The exact command and total 182.98-second wall time are recorded in `old-current-discovery-red.json`.

The owner-phase old producer passed; the current matrix failed at its first scheduler progress
assertion in 0.54 seconds. Its target had genuine L=2,100,000 and V=2,100,020, with an OPEN
full-concurrency collection and RUNNING work. Source inspection confirms the candidate query
has no applicable disjunct for this shape. This establishes the missing discovery condition;
it is not a passing terminal-drain test. The script stopped on this failure, so the runtime
role phase was not executed. Flow uses the runtime role inside the old public producer.

The executed test's subsequent tight-loop terminal expectation also had a separate test defect:
after a correct drain start, legitimate RUNNING work must wait for its real lease to expire.
The following test revision asserts the drain/cancellation request first, waits for the
observed real deadline and then uses normal maintenance. That revision is not the test run
recorded here. No worker lease or historical source was forged for this counterexample.

All 1,119 frozen current native inputs match before/after. Complete old source and migration
manifests match before/after overlay restoration. Binary identities and producer overlay hashes
are retained in `MANIFEST`; the actual executable archives remain in owned temporary storage.
Private receipts and the coordinator-owned database were removed by the original cleanup.
No upgrade, mixed-writer, finality or release acceptance is inferred.
