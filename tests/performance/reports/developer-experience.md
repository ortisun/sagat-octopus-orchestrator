# Benchmark: developer experience (feature 012)

Measured by `tests/performance/developer-experience/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## What this is, and what it is not

The specification asks for an onboarding under fifteen minutes with the dependencies installed, "in UX evaluation, without promising a universal time" (`NFR-012-PERF`, `SC-012-001`). **This benchmark does not evaluate that**: it measures only what the *machine* takes on the way, so that the part of the fifteen minutes that is not the person's reading, typing and thinking is known. Nobody was timed. It also does not include building the container images of the stack (the machine this was run on could not run containers; the `local-environment` job of CI builds them) or downloading dependencies (the package caches were warm).

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 13.04 13.87 12.81 (other work on the machine adds noise)
- Build profile of the test process and the tool: release
- Commit: df66de6 plus uncommitted changes to tracked files
- Toolchains: node: v26.8.1; go: go version go1.24.2 darwin/arm64; python3: Python 3.14.7; dotnet: 10.0.401; java: openjdk version "23.0.2" 2025-01-21
- PostgreSQL in a server named by SAGAT_TEST_DATABASE_URL (or a container) on the same machine; the platform and the tool run in-process, so there is no network between them (the tool speaks HTTP to the API on the loopback)

## Latency

| Operation | Samples | p50 | p95 | max |
|---|---:|---:|---:|---:|
| `sagat doctor` | 5 | 4.4 ms | 5.5 ms | 5.5 ms |
| `sagat new hello-world` | 5 | 0.78 ms | 0.99 ms | 0.99 ms |
| `sagat workflow validate` | 5 | 2.8 ms | 5.9 ms | 5.9 ms |
| `sagat workflow publish` | 5 | 9.5 ms | 10.6 ms | 10.6 ms |
| `sagat worker bind` | 5 | 4.8 ms | 5.2 ms | 5.2 ms |
| `worker dev` + `workflow run --wait`, start to SUCCEEDED | 5 | 780.1 ms | 782.7 ms | 782.7 ms |
| `sagat dev token` | 100 | 0.56 ms | 0.64 ms | 2.1 ms |
| one two-step run served by `worker dev`, `workflow run --wait` | 30 | 276.8 ms | 769.5 ms | 771.5 ms |
| the commands above, together (what the machine takes) | 5 | 802.3 ms | 810.6 ms | 810.6 ms |

The six commands together, on a fresh platform: 810.6 ms. Their rows have only five samples, so their p95 is the slowest one.

## Each SDK's hello-world

`prepare.sh` from a clean build (the example's `node_modules`, `.venv`, `bin`, `obj`, `target` removed first), and `run.sh` (the better of two runs): serve the two activities, start a run, wait for `SUCCEEDED`.

| Language | prepare.sh | run.sh | prepare < 300 s | run < 30 s |
|---|---:|---:|---|---|
| typescript | 1.83 s | 661.9 ms | held | held |
| go | 1.25 s | 623.0 ms | held | held |
| python | 5.12 s | 749.4 ms | held | held |
| dotnet | 3.46 s | 720.2 ms | held | held |
| java | 5.59 s | 1.97 s | held | held |

## Hypotheses

| Hypothesis | Measured | |
|---|---|---|
| the quickstart's commands take the machine under 10 s together (p95) | 810.6 ms | held |
| one run served by `worker dev` takes under 3 s (p95) | 769.5 ms | held |
| a token from the development provider under 50 ms (p95) | 0.64 ms | held |
| each example's `run.sh` under 30 s | slowest 1.97 s | held |
| each example's `prepare.sh` under 300 s | slowest 5.59 s | held |

## What the figures say, and do not

- A run served by `worker dev` is dominated by the worker's polling and the engine's scheduling interval, not by the work; it is a few hundred milliseconds to a few seconds, which is what a developer waits each time they try a change.
- The JVM and the .NET host start slower than Node, Go or Python; the examples are one process that is the worker and the client, so that start is in every `run.sh` figure.
- `prepare.sh` is the cost of building the SDK from this checkout; a published package would replace most of it with a download.
- The commands ran in-process against a PostgreSQL on the same machine: over a real network each call costs its round trip more. The same holds for the compose stack, which is a set of containers.
