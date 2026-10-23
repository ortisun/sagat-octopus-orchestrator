# Observed first runtime failure: departure oracle

C2 first exact default run exited101: 0passed/1failed/0ignored,8.68s wall,6.32s harness, at the clean Keeper departure assertion. It never reached the reopened public refusal.

Owner inspection of the actual owned DB found64 retained owner rows,0liveleases,0scheduler_members, one RUNNING run and the original reserve LEASED. `crates/infra-postgres/src/partitioning.rs::release` updates expires_at, preserving owner/generation; withdraw deletes membership. The fixture incorrectly counted retained expired owner rows. The correction counts only active owner leases and also asserts the local Placement is empty. Native behavior is unchanged, no SQL business state or expiry is rewritten, and the failed DB/log/source pins remain preserved. This is an actual fixture failure, not a claimed production RED or a flaky retry.
