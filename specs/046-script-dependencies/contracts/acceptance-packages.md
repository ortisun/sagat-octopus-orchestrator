# Acceptance packages (design 046, Success criterion 046-001)

Pinned so that the acceptance run does not drift. Each must publish and run in a script on every required target, with
the default policy, in the acceptance run of the release. The versions are fixed by the probe of Task 046-12 (the newest
version older than the quarantine on the day of the probe) and recorded here with their hashes before the run.

| Python | Why | npm | Why |
|---|---|---|---|
| `pandas` | native wheels, deep tree (`numpy`) | `axios` | common HTTP client |
| `httpx` | pure Python, many small dependencies | `zod` | pure TypeScript types at run time |
| `pydantic` | native core (`pydantic-core`, Rust) | `date-fns` | many files |
| `requests` | the most common client | `lodash` | large single package |
| `boto3` | large tree, many files | `@aws-sdk/client-s3` | scoped, deep tree |
| `pyyaml` | import name differs from the package name | `esbuild` | prebuilt binary chosen by `os` and `cpu` |
| `cryptography` | native wheel, `abi3` tags | `sharp` | prebuilt native addon chosen by `libc` |
| `lxml` | native wheel with bundled libraries | `uuid` | tiny |
| `orjson` | native wheel, Rust | `pg` | optional native dependency |
| `python-dateutil` | old-style metadata | `yaml` | name collides with a Python import name |
