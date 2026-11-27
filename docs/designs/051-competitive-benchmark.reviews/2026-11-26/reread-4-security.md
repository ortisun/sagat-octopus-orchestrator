# Targeted re-read 4 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 3ff1566b4c105aa3836349ae2b64d3d00f2748bf
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-3-security.md` (all four resolved), not a full review; the reviewer read files only and ran
nothing. Remark, not counted: a third-party metrics image can no longer be started by `docker run` from `run.sh`; it
becomes a compose service under the compose rules, not an exemption. All three Nits were fixed at the next commit.

## Blocker

None.

## Major

None.

## Nit

1. `O_NOFOLLOW` covers only the last part of a path, and the upload copy may check a file and copy it in two steps: a
   worker that replaces a directory of the record path with a link to a host path redirects both the injector's read
   and the copy into a public artifact. Fix: open every record and uploaded file beneath a handle on the record root
   with `openat2` and `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS`, `fstat` that open file and copy from it.
2. `pg_stat_statements` stores the setting's name of a `set_config` call as a placeholder, so the scan cannot see a
   `set_config` of `synchronous_commit`. Fix: fail the rung on any `set_config` statement outside a per-product list the
   pilot fixes and publishes, each entry tied to the vendor source line and a setting not about durability.
3. The plan paragraph applies the `options`, `-c` and `PGOPTIONS` refusal to the compose files only, while Requirement
   051-004 also names the configurations and connection options; a `PGOPTIONS` in a vendor image's environment is
   outside both (suspicion: not checked whether each client library reads it). Fix: name the configuration files in the
   plan, and refuse a `PGOPTIONS` entry in every product container's `Config.Env` from `docker inspect` at each turn's
   start.

Counts: Blocker 0, Major 0, Nit 3
