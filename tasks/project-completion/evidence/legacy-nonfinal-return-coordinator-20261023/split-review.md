# Bounded corrections after coordinator C3

No fourth whole-coordinator cycle. Two bounded units follow the owner's approved
split-decisions instruction:

1. Helper cancellation: flag-only signal handlers, non-reaping waitid observation,
   and termination/reaping outside handlers. Actual old held-wait-lock probes
   deadlock; corrected probes exit143. TERM-resistant child probes pass. Fresh
   read-only Astra/xhigh unit review: **None**.
2. Credential ordering: success and failure cache checked receipt hash and validated
   DB before deleting exact private files. Successful cleanup uses the same strict
   fresh-snapshot single-new-DB ownership path. C1 findings (stale success snapshot
   and absent/masked hashing failure) were valid and actionable. Both corrected.
   Five extracted-function probes pass, with administration stubbed: normal success,
   preserved original failure, receipt mismatch, ambiguous DB difference and hash
   failure. These establish ordering/branch handling, not real PostgreSQL effects.
   Fresh unit C2 found a valid missing-established-receipt fallback gap. A real
   extracted-function probe reproduced the unwanted DROP before correction.
   Explicit receipt-established/cache-failed state closes that fallback. All six
   probes now pass. Fresh credential-order unit C3: **None**; stop condition met.
   Final native coordinator runs remain separate.

Signals are tested against an explicitly held Popen wait lock, never sleeps.
The collective-group supervisor retains its unreaped leader through final KILL;
new native test/admin children share that group and spawn no OS subprocesses.
No new general-purpose process supervisor or production capability is claimed.
