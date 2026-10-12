# Go and Java sandbox measurements

Date: 2026-10-12T03:36:24Z

- Host: Darwin 25.3.0 arm64; Colima `sagat-remaining` Linux VM, 4 CPUs and 6 GiB RAM
- Commit: `ee2063a0dd8dc505eba36d14d15b093944ec61e7` (working-tree changes present)
- Docker: client 29.8.2, server 29.5.2
- Load at start: 7.45 / 8.55 / 10.15
- Rust profile: test, unoptimized; containers use the production `DockerSandbox` path
- Policy: 256 MiB, 1 CPU, 30 s wall, 64 processes, 64 KiB output and 32 MiB
  `noexec,nosuid,nodev` scratch
- Container profile: uid 65534, network `none`, read-only root, all capabilities dropped,
  `no-new-privileges`, default seccomp and swap disabled

The command was:

```sh
DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock \
  cargo test -p sagat-infra-sandbox go_and_java_native_container_measurements \
  -- --ignored --nocapture --test-threads=1
```

Each sample creates a new isolated build container and a separate runtime container. Images were
already present. “Cold” is the first sample in the process; the five warm samples reuse image layers
but never a container or an author's build output. Build time includes compilation, artifact
construction and v3 validation. Runtime start and whole runtime come from the supervisor's measured
container path. The adapter build key, control-plane signing, REST and PostgreSQL are outside this
focused measurement.

## Native Linux ARM64 Go 1.25.3

Toolchain identity:

```text
build=sha256:4a9b6261e010265a49ca3f6bda30df7fb0f77cf4fda01c3efc1fcee32bfc7431
runtime=sha256:4978bb7b09a15ea1482d7494c96ed5cb28c7be496307f6a60ea7dd243813ae36
protocol=artifact-v3;target=linux-arm64;abi=sagat-inline-v1;compiler=go1.25.3;compression=gzip-canonical-v1
```

| Measurement | Cold | Warm samples | Warm p50 | Warm p95/max |
|---|---:|---:|---:|---:|
| Build | 747 ms | 5 | 719 ms | 720 ms |
| Runtime start | 117 ms | 5 | 119 ms | 130 ms |
| Whole runtime | 460 ms | 5 | 464 ms | 474 ms |

All six iterations compiled a fixed offline module using the baked read-only standard-library cache,
validated a native v3 artifact, executed it through the four-seal memfd loader and returned the
expected JSON/idempotency key. No container owned by the benchmark remained. The separate startup
probe produced a 1,835,156-byte static ELF and a 778,628-byte canonical gzip artifact; its complete
production build/validate/run path passed in 1.34 s.

## Java 17

Toolchain identity:

```text
build=sha256:6b83cff07882927460db1a34329900074c1b2bfbb8ba700a54ebc05bf2ff46a7
runtime=sha256:b5534c058234d7e11a8ce2b7fc15ab28a6f7dd521dd04550fe1b392c8fda3ca1
protocol=artifact-v3;target=jvm-17;abi=sagat-inline-v1;compiler=javac-17.0.16+8;json=jackson-2.17.2
```

| Measurement | Cold | Warm samples | Warm p50 | Warm p95/max |
|---|---:|---:|---:|---:|
| Build | 1,116 ms | 5 | 1,126 ms | 1,144 ms |
| Runtime start | 213 ms | 5 | 208 ms | 214 ms |
| Whole runtime | 568 ms | 5 | 578 ms | 607 ms |

All six iterations compiled `Script.java` with fixed `javac --release 17 -proc:none`, validated the
class-only v3 artifact, loaded it from memory through the parent-first trusted host and returned the
expected nested JSON/idempotency key. No container owned by the benchmark remained.

## Security and compatibility evidence from the same run

The focused production-path tests passed supervised fetch, secret, log and idempotency calls for Go
and Java; redacted a leased secret from both result and log; denied direct access to
`169.254.169.254`; rejected a signed Go target mismatch before starting a runtime; stopped a Go CPU
burn at its one-second quota; and left zero owned containers. The existing TypeScript, Python and C#
real-container suite passed 30/30 serial cases, including network, metadata, capabilities, process,
memory, CPU, disk, `noexec`, framing, cleanup and C# supply-chain regressions.

These are native ARM64 measurements only. They do not establish Linux AMD64 support; AMD64 remains
disabled until the equivalent compile, v3 validation, startup probe and sealed-memory execution pass
on a native AMD64 CI runner. Six local samples under a loaded development VM are a regression
baseline, not a production SLO or a tail-latency estimate.
