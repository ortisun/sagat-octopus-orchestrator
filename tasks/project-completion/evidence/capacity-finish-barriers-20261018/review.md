# Native barrier proof review

Claim: test-only barriers establish exact native G Busy/prefix/fence and confirmed abort atomicity,
without claiming unknown-COMMIT recovery, retained quota or a complete lifecycle.

Fresh Astra/xhigh cycle1: P2 valid/actionable. usename=sagat_app blocker filters conflicted with
world() selecting the owner execution pool when SAGAT_TEST_AS_RUNTIME_ROLE was absent. Added a
local runtime_fixture that explicitly connects the native adapter as sagat_app and asserts the
actual database user regardless of suite mode. Existing worker/scheduler fixture role still follows
the wider mode; default-mode evidence claims native adapter runtime only. No production change.

Fresh Astra/xhigh cycle2 inspected current code, including fixed injected-marker assertion:
no remaining concrete false-positive, hang or fixture-scope issues found. Stop: no concrete
findings. Existing user selection keeps current review; no external CLI. Root alone implemented
and executed tests. Both reviews read-only. No reproduced default-mode RED is claimed: this was a
static review finding, followed by actual corrected default and full-runtime green executions.

Root also tightened initial proof: shard wait is observed while actual DB clock is still live,
then DB clock crosses2s expiry under unchanged tuple; G Busy directly preserves header/member;
fault test names exact audited injection points and requires the fixed injected marker rather
than accepting unrelated storage errors. These improvements are included in cycle2 review.
