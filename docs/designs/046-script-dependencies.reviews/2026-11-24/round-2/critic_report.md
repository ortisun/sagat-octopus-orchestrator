Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: egm-design (egm-review stage 7, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 7, Nit 3

Round 1: findings 1 to 7, 9 to 11 and 13 answered; 8 in part (finding 1 below); 12 superseded (finding 2 below).

1. **Major.** Materialization counts against the attempt timeout that starts at lease; a short timeout fails every cold first run; the gateway is on the run path for new runners. Fix: materialize before the lease, or say how the attempt clock starts after it; a risk row.
2. **Major.** The digest is reported by the container that parses the hostile archives, and the bytecode that Python runs is outside it. Fix: recompute in a fresh check container with the volume read-only; checked-hash bytecode or bytecode inside the digest.
3. **Major.** No rule for keeping unpacker and runtime images; a patched image makes every bundle version 5 unrunnable on fresh runners. Fix: retention while a version names them; a re-materialization or re-signing path.
4. **Major.** The advisory import is one unbounded transaction holding the row every finish needs. Fix: a diff by `modified_at`, staged outside the lock; hold the row only for the swap and the malware blocks; measure the window.
5. **Major.** Store collection deletes live files: offline imports not yet locked, files of test runs and of resolves and installs in flight, and a reference gained after the re-read. Fix: leased pins, imported bundles kept until removed, a grace period, a race test.
6. **Major.** Who holds which credential: uploads to the store from the control plane; the check's crate list; the gateway's callback to the control plane. Fix: an internal upload route; the job-status callback and its credential, or a short token lifetime.
7. **Major.** npm's `before` likely forces full version documents through the 32 MiB bound. Fix: drop `before` and rely on the gateway's filter and the control plane's check, or measure.
8. **Nit.** A two-target resolution can fork into two versions of a name; the lock size estimate is low.
9. **Nit.** Who charges the fetch budget.
10. **Nit.** Malware blocks written in Phase A are enforced only by Phase B's lease check.
