# Security review of design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 831873036bbf0a4a0e5ec51d988892a4d1f8331c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing; each claim is marked verified in the repository or inferred from GitHub's and Windmill's behaviour.

## Major

1. Third-party code with root on the runner can poison the release through the Actions cache. Verified: the release's
   `recovery` job restores a `rust-cache` entry under a computable key; `publish` downloads every artifact of its run
   and copies same-named files over real ones before attestation; this platform's script path needs the engine socket.
   Inferred: Windmill's documented worker holds the socket or runs privileged for `nsjail`; the runner user has
   passwordless sudo; root reads the runtime token, which writes cache entries whatever `permissions:` says. A malicious
   layer, run on the default branch, plants a cache entry that the next tag's build restores, and `publish` attests the
   planted archive. Fix: the rule refuses `privileged`, the socket, host namespaces, `cap_add`, `devices` and unconfined
   profiles in every competitive compose file (Windmill's scripts with `nsjail` off if needed, stated); no cache action
   in the benchmark workflow; in its own pull request, `release.yml`'s `publish` downloads by name and refuses a
   duplicate, and `recovery` drops `rust-cache`.
2. The permission rule passes a workflow with no `permissions:` block (the repository's default may be read and write).
   Fix: the exact top-level `contents: read` block, as `scripts/tests/test_ci_workflow.py` asserts; no job-level
   `permissions:`, `secrets: inherit` or `github.token`.
3. The digest rule cannot see images named outside an `image:` key (the script runtimes default to `node:24-alpine` and
   `python:3.13-alpine`, pulled by tag on first use; `docker run` lines in `run.sh`; images a product pulls itself). Fix:
   one image manifest read by the compose files and `run.sh`, every `SAGAT_RUNNER_IMAGE_*` set from it; after each turn
   `docker image ls --digests` against it; the rule scans `run.sh` and image-like values.
4. What the archives and raw data may contain is unsaid: a restored archive fixes its credentials for the campaign
   (role verifiers, tokens, signatures, Windmill's tables), whose plaintext must travel; configurations committed;
   failure dumps with passwords; a restored data directory's `postgresql.auto.conf` runs commands; the SHA-256 is
   computed by the same run. Fix: credentials generated per campaign, never development defaults, the archives public;
   placeholders in published configuration; an allowlist of uploaded files, `log_statement` none; the restore overwrites
   `postgresql.conf`, `postgresql.auto.conf` and `pg_hba.conf` and refuses members outside the directory and symlinks;
   a fetched archive's SHA-256 from the reviewed tree only.

## Nit

1. `persist-credentials: false` on checkout, checked by the rule.
2. No `${{ inputs. }}` or `${{ github.event. }}` inside `run:`; inputs through `env:`.
3. A static egress rule: `internal: true` on every network of every service, this platform's included; the start probe
   tests DNS, a public IPv4 address and IPv6.
4. Pilots under the same network rule; step-language candidates needing packages pre-resolved into a pinned image, or
   excluded.
5. `omes` pinned to a commit with its `go.sum`, unprivileged.
6. Image signatures verified where vendors sign; digests from the vendor's registry.

Fork dispatch, `secrets.` references, the private-repository refusal, tenant boundary: checked, no finding.

Counts: Blocker 0, Major 4, Nit 6
