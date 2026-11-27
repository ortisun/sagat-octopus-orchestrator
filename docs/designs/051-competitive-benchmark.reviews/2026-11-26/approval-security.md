# Approval confirmation of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c91af7ac2be73ff9eb4a2163d5d124a15f9bbac8
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`), then the fixes
re-read in `reread-1-security.md` to `reread-15-security.md`, every finding of each fixed before the next. Not a full
review. The reviewer read the design and ran nothing.

Both Nits of re-read 15 are resolved: the history job saves the database server's image with `sagat_commit_check`
beside this platform's images, their SHA-256s computed before any product starts and their image IDs recorded; every
later job checks each archive before `docker load` or restore and never rebuilds the server's image; the exempt images
are matched by the recorded image IDs, never by name, so a rebuilt image fails the comparison. Nothing new opens: the
pilots load no archive, and the checksums are taken before any product runs. Suspicion, not counted: the comparison
must use full image IDs (`--no-trunc` or `docker image inspect`), not the 12-character default.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
