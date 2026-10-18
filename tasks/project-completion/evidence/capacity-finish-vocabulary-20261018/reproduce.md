# Dormant Finish vocabulary

Base3651eb34. commands.json/gates.json retain exact commands, exits and wall times. Warm target
optional; CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216. No SDK bless or
SAGAT_UPDATE_BASELINE used. This is vocabulary, not a native Finish implementation.

Audit missing variants cause actual API compile failure, not behavioral RED. First hint attempt
has UUID Serialize fixture compile failure, separately retained; .to_string fixes test values.
Actual corrected hint RED0/1,0.04s: canonical channel absent before producer code exists.
GREEN audit7/7, hint1/1, AsyncAPI evolution/conformance4/4 (2.02s). Final audit rename rerun7/7.
Synthetic hint tests reject omitted required fields, extra credentials/payload, wrong types and
invalid revision/epoch minima; absent/string/null optional trace examples pass. They do not prove
actual native outbox or receipt dedup. Existing actual outbound conformance remains intact.

Only new CapacityMemberFinished baseline entry was added manually; every prior parsed message
is identical. No existing SDK expected output changed. Pinned AsyncAPI CLI3.6.0 validates with
0errors/0warnings and one existing-version informational notice (document3.0.0). Strict locked
workspace/all-target Clippy83.89s, fmt4.04s, architecture24crates pass. No fullworkspace tests,
SDK58, broker delivery, native lifecycle, group eligibility, old-writer/resource/nested acceptance.
