# C# sandbox measurements

Date: 2026-10-11T21:06:24Z

- Host: Darwin 25.3.0 arm64
- Commit: 5124342fbc98ccf4b854cf4cf5010f1202777a81 (working-tree changes may be present)
- Docker: 29.5.2
- Images: [sagat/dotnet-builder:10.0.401] sha256:828e4b881095a886aec72e0ffaab18839cd7729c2d7d6696b517c5301913e982
[sagat/dotnet-runtime:10.0.12] sha256:712ec8c9bdafc41d798c076da8d4a1444069fbb8d123f1eaf50f9452337c9c85
- Load at start: 18:06  up 4 days, 10:13, 5 users, load averages: 9.54 9.08 10.61
- Rust profile: release
- Policy: Limits { memory_bytes: 268435456, cpu_millis: 1000, wall: 30s, processes: 64, output_bytes: 65536, scratch_bytes: 33554432 }

Each sample creates a new isolated container. Cold means the first sample in this process with images already present; warm samples reuse image layers, not containers or compiled results. Build measurements include compilation and artifact preparation in the sandbox adapter, but not platform signing, REST, PostgreSQL or build-cache lookup. Runtime measurements include creation, loading, execution and removal. This is a small local sample, not a production SLO.

## framework only

First build: 797 ms; first runtime start: 89 ms; first whole runtime: 429 ms.

| Measurement | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| build | 9 | 739 ms | 751 ms | 751 ms | 751 ms |
| runtime start | 9 | 87 ms | 91 ms | 91 ms | 91 ms |
| whole runtime | 9 | 430 ms | 436 ms | 436 ms | 436 ms |

## Newtonsoft.Json 13.0.4

First build: 745 ms; first runtime start: 95 ms; first whole runtime: 475 ms.

| Measurement | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| build | 9 | 752 ms | 765 ms | 765 ms | 765 ms |
| runtime start | 9 | 86 ms | 91 ms | 91 ms | 91 ms |
| whole runtime | 9 | 468 ms | 480 ms | 480 ms | 480 ms |


## Environment and outcome

Docker runs in the isolated Colima `sagat-remaining` Linux VM with 4 CPUs, 6 GiB RAM and a 30 GiB disk. The host and VM also serve other development workloads; the load above records that condition. Image downloads were completed before measurement. Both groups completed 10 builds and 10 executions with the expected result (`answer: 42`), and the sandbox reported zero leftover containers after each group. No failures occurred in these 40 operations. With nine warm samples, p95 and p99 select the maximum observation; they are not estimates of production tail latency.
