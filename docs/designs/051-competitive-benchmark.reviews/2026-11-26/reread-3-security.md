# Targeted re-read 3 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: e095245da09e79ab0925ee6952e69bc1272cd109
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the changes after `reread-2-security.md`, not a full review. The reviewer read the design, its three earlier security
reports and `.github/workflows/release.yml`, and ran nothing. Four of the listed changes raise no finding: the
`cgroup_parent` of the driver and metrics containers, the claims checker in the `version` job, the method version's
digests and the common-rate handoff within one run. All four Nits were fixed at the next commit.

## Blocker

None.

## Major

None.

## Nit

1. The host fault injector writes fault times into the shared record directory, which product workers (third-party
   code, often root in their containers) can write: a link there could redirect the host's write, a FIFO could block
   its read, and an uploaded file there could be a link to a secret of the job. Fix: fault times and every uploaded
   file in a host-only directory mounted read-only into the driver; records opened with `O_NOFOLLOW|O_NONBLOCK`,
   checked by `fstat` and read bounded; the upload allowlist refuses links and other non-regular files.
2. `cache: false` cannot go on every setup action (`setup-node`, `setup-python` and `setup-java` take a package-manager
   name), and the benchmark workflow's "no cache action" leaves `setup-go`'s default cache. Fix: one rule for both
   workflows: `cache: false` on `setup-go` and `setup-dotnet`, no `cache:` input on `setup-node`, `setup-python` or
   `setup-java`, `package-manager-cache: false` on `setup-node`, and no `actions/cache`, `rust-cache` or `type=gha`.
3. The durability checks miss `options=-c` in a connection string and `PGOPTIONS`, which `pg_stat_activity` does not
   show; the "no `SET` grant" claim means nothing for `synchronous_commit` (user context); and a function's `SET`
   clause or a `set_config` inside a function escapes the default `pg_stat_statements.track = top`. Fix:
   `check_competitive_benchmark` refuses `options`, `-c` and `PGOPTIONS`; `pg_proc.proconfig` and
   `pg_db_role_setting` read at each turn's start and end; `pg_stat_statements.track = all`; the grant claim limited to
   `commit_delay`.
4. The privilege refusals cover compose files but not `run.sh`'s own `docker run` lines, which start the metrics
   containers that must measure the daemon's processor time. Fix: refuse `docker run` and `docker create` in `run.sh`
   of any image not built at the commit, apply the compose list as flags to the rest, and read the daemon's and shims'
   processor time from the host's `/proc` in `run.sh`.

Counts: Blocker 0, Major 0, Nit 4
