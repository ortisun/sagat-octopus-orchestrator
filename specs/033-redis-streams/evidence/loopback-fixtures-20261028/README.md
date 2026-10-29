# Redis fixture loopback correction — 2026-10-28

Scope: FR-033-008, SC-033-003/004, T033-007. Test fixtures only.

- `redis-loopback-ci-red`: real Docker inspection failed because HostIp was0.0.0.0.
- `redis-loopback-native-green`: compilation error from placing ImageExt mapping before
  GenericImage-only methods; corrected builder ordering, not a behavioral RED.
- `redis-loopback-native-green-v2`:29 native cases passed,0 failed/ignored,57.47s runtime.
  Original DNS no-fallback and TLS/authentication tests are unchanged.

Final bounded-inspection, workspace lint and released-callback gates are recorded
separately after execution. These focused gates do not close the final release gate.

Initial bounded-inspection gates (before review changes):

- `redis-loopback-final-native`:29 passed,0 failed/ignored;50.78s runtime.
- `redis-loopback-workspace-clippy`: strict workspace/all-target lint exit0;176.26s.
- Format, architecture (24 crates), agent configuration: exit0.
- `redis-loopback-released-callback`: actual released0.2 writer/reader and current
  coordinator both pass;68.67s including build,5.07s coordinator runtime.
  Source-bound profile metadata and compressed current source manifest are included.

The callback profile confirms the old login/session/process fence, unchanged old
metadata and one new Redis delivery through the corrected fixture. The other five
released profiles were not rerun for this fixture-only change; their earlier
source-bound results remain separate. Full CI and final release acceptance remain open.

Review reconciliation: the first read-only review identified independent Docker
client configuration and implicit port0 semantics. The final implementation uses
Testcontainers' public configured-client accessor and one shared explicit HostConfig
binding with empty HostPort. The simultaneous-fixture assertion rejects a fixed
host port. The second/final bounded review reported no actionable finding.
Empty HostPort allocation follows the [Moby port-binding implementation](https://github.com/moby/moby/blob/docker-v29.1.3/daemon/container_operations.go).
Final reviewed-tree gates are listed separately below after execution.

Final reviewed-tree results:

| Gate | Actual result |
| --- | --- |
| Redis native, runtime role |29 passed,0 failed/ignored;64.00s runtime |
| Strict workspace/all-target Clippy |exit0;120.85s |
| Format / architecture / agent configuration |all exit0;24 workspace crates |
| Actual released callback cutover |old writer/reader1 passed; current coordinator1 passed,4.81s runtime;149.73s command including the build-lock wait |

`reviewed-profile/` contains the unique selected outcome, its matching build and
transcript digests, and the compressed exact source manifest. All2007 code, SDK,
workflow, script and toolchain inputs still match that manifest after evidence
archival. No production code changed in this correction. PR/release CI is pending.
