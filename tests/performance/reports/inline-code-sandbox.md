# Benchmark: inline code sandbox (feature 005)

Produced by `tests/performance/inline-code-sandbox` (T005-034, NFR-005-PERF). Hypotheses are written down before the run and the result says whether each held; a hypothesis that fails is restated openly, not dropped.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Container engine: Docker 29.8.2 linux/arm64, runtime runc (cpus 16, memory 8318709760 bytes)
- Images: node:24-alpine, python:3.13-alpine (already on the machine)
- Load average when the run began: 9.05 8.82 10.54 (other work on the machine adds noise)
- Build profile of the test process and services: release
- Commit: f907b6f
- PostgreSQL 16 in a Docker container on the same machine; the API, the engine's worker protocol and the runner run in-process, over loopback

## Start

Straight on the sandbox (no control plane, no database): a build, then the same artifact run thirty times, each in a container of its own. *Start* is from asking for the container to the first instruction of the script (the shim's `ready`); the first run of a language after the process began is the cold one, the rest are warm. The images are already on the machine: pulling one is the network's time and is not measured.

| What | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| typescript: start (first instruction), warm | 29 | 104 ms | 109 ms | 116 ms | 116 ms |
| typescript: whole run (create, start, run, remove), warm | 29 | 443 ms | 455 ms | 457 ms | 457 ms |
| python: start (first instruction), warm | 29 | 327 ms | 343 ms | 351 ms | 351 ms |
| python: whole run (create, start, run, remove), warm | 29 | 664 ms | 684 ms | 692 ms | 692 ms |

| Language | Cold start (first run) |
|---|---|
| typescript | 100 ms |
| python | 340 ms |

## Builds

From `POST .../versions` to a ready version, as an author sees it: queue, the runner's claim, a container that checks the source, the report, the signature. The last row publishes the same source again in another script: the control plane finds the artifact by its build key and the runner is never handed a build.

| What | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| typescript: publish to ready (a build) | 15 | 1347 ms | 1367 ms | 1367 ms | 1367 ms |
| python: publish to ready (a build) | 15 | 607 ms | 623 ms | 623 ms | 623 ms |
| typescript: publish of an identical build (no container) | 15 | 46 ms | 54 ms | 54 ms | 54 ms |

## What an author waits for

A test run through the API: the draft is built and run with an input, and the author reads the result. Each includes a build.

| What | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| typescript: test run (build, run, report) | 20 | 1760 ms | 1776 ms | 1787 ms | 1787 ms |

## Script steps through the engine

A workflow of one script node, started by the API, leased by the runner through the worker protocol, run in a container and completed; the runner's serving loop and the scheduler run as they do in production. Each cell is the median of three runs of forty, with the lowest and highest in brackets.

| What | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| typescript step, one run at a time (start to SUCCEEDED) | 10 | 492 ms | 511 ms | 511 ms | 511 ms |

| Containers at once (`SAGAT_RUNNER_MAX_CONCURRENT`) | Script steps per second |
|---|---|
| 1 | 1.9 (1.9 to 1.9) |
| 4 | 6.5 (6.4 to 6.7) |

Four at once got **3.5x** the throughput of one.

## Holding a script to its limits

A hostile script run on the sandbox with a 3 second limit and 64 MiB of memory (128 MiB for the log flood): how long until it is gone. *The first runs of this benchmark ended the flood with `SCRIPT_OUT_OF_MEMORY` (64 and 128 MiB) and, once the cause was looked for, sometimes with a normal `null` result. The cause was the Node shim, which wrote the log through `process.stderr`: that stream queues what the pipe will not take and drops what is still queued when the process exits. A script that wrote faster than it was read filled its memory with the queue or lost almost all of its output, and in the second case the supervisor never saw the megabyte it needs to cut the script off (a plain `docker run` of the same script delivers 0.4 to 0.9 MB of 2 GB to a fast reader). The shim now writes the descriptor itself, so a flood meets the pipe's backpressure; a run the supervisor stopped for its output is reported as that even if the kernel ended it a moment later.*

| What | Samples | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| CPU burner: time past its 3 s limit until it is gone | 3 | 446 ms | 454 ms | 454 ms | 454 ms |
| memory bomb: until the kernel killed it (64 MiB) | 3 | 706 ms | 720 ms | 720 ms | 720 ms |
| log flood: until it was cut off (128 MiB) | 3 | 480 ms | 482 ms | 482 ms | 482 ms |

## Hypotheses

Written before the first run.

| Hypothesis | Result |
|---|---|
| Warm start p50 under 1.5 s, both languages | held (typescript 104 ms, python 327 ms) |
| Cold start under 5 s | held |
| A build under 3 s (p50) | held (typescript 1347 ms, python 607 ms) |
| An identical build costs no container, under 250 ms | held (46 ms) |
| A test run under 5 s (p95) | held (1776 ms) |
| Four containers at once get 1.5x the throughput of one | held (3.5x) |
| A script that will not stop is gone within 2 s of its limit | held (p95 454 ms) |
| A bounded environment (`environment()` above) | recorded |
