# Targeted re-read 15 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: dc1bf4479d4fc1c3a10551d9239494fcbcfdf420
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the changes made for `reread-4-correctness.md`, not a full review; the reviewer read the design and `ci.yml`'s
permissions and ran nothing. Clean: "not run under the durability rule" makes the design stricter; the count read
before the kill loses nothing that matters; the `ci.yml` step reaches no release or secret; the acknowledgement
carries no data; the job arithmetic recomputed holds. Both Nits were fixed at the next commit.

## Blocker

None.

## Major

None.

## Nit

1. Requirement 051-015's SHA-256 check names this platform's images and the data directories, not the database
   server's image, and Task 051-6 builds that image in `run.sh` in every job kind, so a later job could load a replaced
   artifact unchecked. Fix: name the image among the archives, compute its SHA-256 before any product starts, check it
   before `docker load` in every later job, and never rebuild it.
2. An exemption from `images.lock` by name would accept any image with that tag, since a locally loaded image has no
   registry digest. Fix: match the exempt images by the image IDs the history job recorded.

Counts: Blocker 0, Major 0, Nit 2
