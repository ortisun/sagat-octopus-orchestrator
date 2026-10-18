# Actual COMMIT response-loss proof review

Claim: the test-only relay loses a native COMMIT response while the real server still executes
that COMMIT, then verifies actual committed/aborted resolution and same-command recovery. This
must not substitute a discarded already-returned response for native uncertainty.

Harness fresh Astra/xhigh cycle1: P2 valid/actionable. close() moved its JoinHandle into timeout;
cancellation/timeout could detach it. Fixed by awaiting self.task.as_mut(), clearing ownership
only after join, so Self::Drop still aborts on cancellation/panic/timeout.
Cycle2: P2 valid/actionable. start() held a bare task across authentication before Self existed;
cancelling start bypassed explicit abort. Fixed using min0 lazy pool and fully constructed Self
with task before the first pool.acquire/auth await; startup cancellation/panic now drops the owner.
Cycle3: no remaining concrete protocol/cleanup/false-proof issue. Stop: no findings; no fourth
cycle. Startup/close cancellation ownership is source reviewed, not separately fault injected.

Independent sequence fresh Astra/xhigh cycle1: no concrete issue in actual COMMIT/gate observation,
own guard probes, distinct member Busy versus same-command prefix timeout, both server outcomes,
fixture authority or same-key recovery. Root later added an explicit unchanged Busy-peer assertion.
The wrong hint-module import was a compile-only test-fixture error and was corrected mechanically.

Sol/high supplied read-only wire/protocol draft; root alone implemented and tested. Root adapts
reply loss to occur while PostgreSQL is still COMMIT-blocked, preserving upstream, and pins the
backend read across the control select. All post-COMMIT messages are withheld, including actual
outcome; c2s freezes so queued SQLx rollback cleanup cannot retract dispatched COMMIT. Existing
user choice keeps current review; no external CLI, production proxy, caller or quota proof.
