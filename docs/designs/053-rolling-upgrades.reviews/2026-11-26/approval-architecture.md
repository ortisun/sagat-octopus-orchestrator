# Approval confirmation of design 053: architecture (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 3cf5ee9737c4611e9126403c7f5dbfecc0b4f861
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`architecture-full.md`, at 7771688e),
then `reread-1-architecture.md` (at 655ecb38), whose three Majors and five Nits are confirmed resolved here. Not a full
review. The reviewer read files only, ran nothing, and could not list branches.

The later changes (the widening's one lock, `connect_command`, the descriptors, phase 2's lock order, the fence's
serialization path, the refused partitioned-index migrations, `journal-buckets-v1` unfenced, the `sagat_retirer`
grants) keep the layers: migration versions and SQL stay in `infra-postgres`, the application gets `SchemaVerdict`.
Follow-up Tasks 048-19 and 049-20 match the design. Nothing in ADR-0131's own text stops its acceptance; the
correctness approval's Major changes Requirement 053-004, not the record.

No Blocker, no Major.

## Nit

1. ADR-0132's "Enforced by" still names `a_permanent_capability_refuses_both_deactivation_phases` and
   `Capability::is_permanent()`, and ADR-0082 has no note of the refinement; update both in the acceptance commit.
2. Task 053-5 still says "the drain checks beside `ACTIVATABLE`"; they are in the descriptors of
   `crates/infra-postgres/src/instances.rs`.
3. The descriptor test has no name; name it in Task 053-5 and in ADR-0131's "Enforced by".

Counts: Blocker 0, Major 0, Nit 3
