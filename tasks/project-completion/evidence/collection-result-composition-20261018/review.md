# Bounded review reconciliation

Root is the sole repository writer and test executor. Reviewers used fresh contexts and
read-only artifacts. The owner declined the optional external review earlier; none ran.

- **Q9a discovery proposal C1: None.** The SQL adds a conservative metadata/length hint before
  owned-shard queue limits. Shared repeatable-read and native locked verification remain the
  authority. Missing joins are not interpreted as zero. No constant scan-cost claim is made.
  Root additionally clarified the existing misleading index-cost comment; no behavior changed.
- **Old/current scheduler test C1:** two actionable fixture problems, corrected: process exit
  does not expire a 60-second database lease; fitting-case prerequisites must run before global
  scheduler maintenance. Same-owner normal Keeper renewal supplies actual authority.
- **Old/current test C2:** one actionable waiting-oracle problem, corrected: actual RUNNING
  work is cancelled only after acknowledgment or real expiry. C3 found **None**. The test now
  observes the live-worker drain first and then waits for its recorded deadline. The executed
  pre-correction counterexample is separately retained under `collection-discovery-red-20261018`.
- **Child result C1:** the original oracle omitted persisted owner pointers. Root added both
  `lines` and `later` owner/scope digest equality plus retained JSON assertions.
- **Classified result C1:** audit/log canaries omitted authorized public values and declared
  keys. Root added the public value and all declared property names to both absence checks.
- **Finish atomicity C1:** three actionable gaps, corrected: all tenant payloads/physical usage
  must be visible in snapshots, source-bound replay must not silently drop its source list,
  and the late injected fault must be distinguished from statement timeout. C2 found **None**
  for the corrected rollback/retry test with replay explicitly deferred.
- **Metadata replay API/consumer C1:** no API issue; the constructor retains source `Some([])`
  separately from legacy `None` and carries no body. The caller incorrectly reused the live
  store/cache. Root changed it to a fresh `PgCollectionStore` and fresh `DefinitionCache`
  using the fixture's actual API runtime pool. Getter-mirroring constructor assertions from
  the draft were omitted; the meaningful test is native exact replay and changed-source refusal.

Two first review attempts failed because the selected model was at capacity. Retrying the same
model succeeded; those capacity failures were not review results. No fourth whole-protocol
review occurred. Artifacts here are review inputs, not evidence that tests passed; actual
commands, snapshots and harness output have separate records.
