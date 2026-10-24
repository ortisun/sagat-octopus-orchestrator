# Coordinator review boundaries

Whole-coordinator reviews were capped at three cycles. Actionable findings:
C1 detached administration/test child cancellation, receipt mismatch deletion,
and explicitly empty selector fallback. Corrected through one collective owned
process group for new selectors, bounded administration, validated receipt match,
and unset-only default selection. C2 found the exact receipt.tmp credential path;
cleanup now removes exact private paths before slow failure cleanup.

C3 found reentrant waitpid-lock deadlock in phase/administration signal handlers
and late credential deletion on the successful path. These are distinct bounded
helper-wait and credential-order units, following the owner's prior instruction
to split decisions after the three-cycle limit. No fourth whole-protocol review
is requested. The first whole positive and isolated RED coordinator runs passed
before these supervision corrections; their evidence is retained but does not
stand as final hardening acceptance.

The actual held-lock probe reproduces deadlock in BOTH old helper blocks and
terminates them after the explicit three-second diagnostic bound. Flag-only
handlers and non-reaping wait observation fix both; after probes exit143 and
return from the handler under the held lock. No business state was edited.
