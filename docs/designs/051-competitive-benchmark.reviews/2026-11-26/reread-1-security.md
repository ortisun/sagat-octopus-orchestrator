# Targeted re-read 1 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: ed1d47cf2b4f097aa6d44b68d6680f90deb0bf24
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`security-full.md`, not a full review. The reviewer read files only and ran nothing.

Full round: Majors 2 and 3 resolved; Major 1 partly and Major 4 mostly (below); Nits 1 to 5 resolved, Nit 6 partly
(digests from the vendor's registry). Task 051-0 covers the release workflow's only cache (checked).

## Major

1. This platform's script path needs the engine socket, which Requirement 051-020 refuses without an exception, and the
   per-run containers are outside compose and the probe. Fix: one named exception, the sandbox-runner service, read-only,
   no capabilities, `no-new-privileges`; its run containers with `--network none`, non-root, from `images.lock`,
   checked after each turn.
2. Where the Temporal sidecar and `omes` run is unstated; a host process has the runner's sudo and could plant cache
   entries that `ci.yml`'s `rust-cache` restores on pull requests, faking a gate. Fix: every third-party program as a
   compose service under the rules; the rule refuses a third-party binary started from `run.sh` on the host.

## Nit

1. No `${{ secrets.` reference at all, not only `github.token` and `secrets: inherit`.
2. How the driver reaches products on internal networks: as a container on each product's network.
3. The archives' SHA-256s only as the history job's outputs, downloaded by exact name, never from an artifact or a
   summary a pilot could plant.

Counts: Blocker 0, Major 2, Nit 3

All five, and Nit 6's digest source, were fixed at the next commit.
