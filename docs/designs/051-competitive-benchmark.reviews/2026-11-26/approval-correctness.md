# Approval confirmation of design 051: correctness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c91af7ac2be73ff9eb4a2163d5d124a15f9bbac8
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`), then
`reread-1-correctness.md` to `reread-4-correctness.md`, whose findings are all confirmed resolved here (the vendor
fallback, the count before the kill, the server image built in the history job, the extension's tests in CI, the
acknowledgement, the re-run overhead). The arithmetic rechecked holds: throughput job 336.5 minutes, campaign 18,765
runner minutes, wall time 2,387 minutes, every job under the 340-minute plan check. Not a full review. The reviewer read
the design and ran nothing.

No Blocker, no Major.

## Nit

1. The pilots fall outside the image-ID exemption, which only the history job records, and the history job's
   30-minute budget was measured without the server image's build and save. Each pilot builds both images, records
   their IDs and is checked against them; later jobs receive the IDs beside the SHA-256s.
2. The exemption may be too narrow: the driver's container, the Temporal sidecar and this design's Go workers are also
   built at the commit and absent from `images.lock`; list every image built at the commit and saved, matched by ID.
3. "Not run under the durability rule" is excepted neither by Success criterion 051-001 nor by Requirement 051-010's
   ranking; add it to both.
4. The pre-kill read is in no task, and its sentence reads as if the redo LSN were subtracted from the refusal count;
   add it to Task 051-7 and reword.
5. Task 051-2 adds the `ci.yml` step that tests `sagat_commit_check`, which Task 051-6 writes; move that part to Task
   051-6.

Counts: Blocker 0, Major 0, Nit 5
