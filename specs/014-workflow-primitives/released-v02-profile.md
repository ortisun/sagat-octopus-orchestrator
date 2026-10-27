# Released v0.2.0 active-data compatibility checkpoint

Refs: T014-016/017/040/043, FR-014-014/015/017. Use the actual v0.2.0 tag commit
bfda342073db71e84f30bdc79ccf3fbc97a7153c. The earlier unpublished6c44a0e4 fixture is a different
profile and must not be relabeled as released evidence. No migration checksum or missing-version
check is disabled.

Build an isolated release archive with test-only phase overlays. Verify production crates/apps,
Cargo files and toolchain configuration byte-for-byte against the immutable Git archive and
record the production-tree, overlay and executable hashes. Copy the old executable to an owned
private artifact directory so the shared Cargo target cannot overwrite the active phase binary.

The current coordinator owns a disposable PostgreSQL server through the repository's fixture.
The old process connects only to that server, creates/migrates its own owned database and uses
its real REST publication/start, scheduler and gRPC1.5 worker to finish a reserve activity and
leave charge READY/unleased. It also publishes/starts a legacy root event wait. Its witness
contains only database/resource identities, migration boundary and immutable-prefix hashes;
no payload bytes, tokens, session capabilities or credentials.

After successful old-process exit, attach the same exact database, assert the old migration
boundary and apply the normal current MIGRATOR. Model expiration of the stopped scheduler's
member/shard leases explicitly on the database clock, then acquire a new owner through Keeper.
This expiry injection is not elapsed-wall-time or a live mixed-owner race proof. Start current
runtime-role API/worker services against that database, complete the old READY activity through
gRPC, publish the matching legacy root event through REST and dispatch it. Both old runs must
finish and replay, with old captured/definition/journal/completed-prefix evidence unchanged.

Finally, another process of the exact old executable starts its real API on that already
migrated database and verifies the terminal runs through REST and native replay. Do not run the
old MIGRATOR against new history: serving has migrations off by default; migration is a separate
command. This is actual released-source runtime compatibility, not a redistributed binary asset.

This first checkpoint covers old admitted activity bodies and root waits. It does not prove an
old in-flight lease report to a new server, new scoped-event bodies read by old code, mixed live
event dispatchers, public scoped-event activation, all-five old SDK behavior or nested capacity.
Those remain separate acceptance gates. The test starts every required service itself and never
relies on another project's or a manually started database.
