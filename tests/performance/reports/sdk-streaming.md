# SDK task streaming comparison

Source: `71695d3` plus the working-tree changes; profile: release; host: `Darwin arm64`; CPU: `Apple M3 Max`.

One round per language/workload/transport on a shared machine. Idle: two workers for eight seconds. Burst: 300 real runs. Backpressure: 32 tasks of two seconds, maxInflight eight. RPC/s and throughput include runner startup/shutdown. Delivery latency is durable step.released/retry_due→public handler entry; it includes database discovery, admission, network delivery, and SDK dispatch. It subtracts the local PostgreSQL container's epoch timestamp from the runner's epoch timestamp, so every percentile carries the clocks' unknown offset. Results stay signed and unclamped; negative samples are reported as count/minimum and are measurement uncertainty, not a transport failure. Ready→lease is the durable server-side portion and is not relabeled as client delivery. Poll claim attempts equal PollTasks calls (one claim operation per request); stream claim attempts come from worker_task_stream_claims_total. Peak RSS samples the runner process tree every 50 ms and excludes the server. Peak local in-flight is measured at handler entry/exit. Slow rows show heartbeat progress under handler-capacity backpressure; admission-saturation progress is covered separately by the held-stream server test.

| SDK | Workload | Transport | Wall s | Receive RPCs | RPC/s | Claim snapshot delta¹ | Completions/s | Delivery p50/p95/p99 ms | Negative count/min ms | Ready→lease p95 ms | Peak local | Peak leases | Peak RSS MiB | Heartbeats OK/refused |
|---|---|---|---:|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|
| typescript | idle | poll | 8.161 | 33 | 4.04 | 33 | 0.00 | — | — | — | 0 | 0 | 70.30 | 0/0 |
| typescript | idle | stream | 8.214 | 4 | 0.49 | 30 | 0.00 | — | — | — | 0 | 0 | 69.84 | 0/0 |
| typescript | burst | poll | 9.588 | 385 | 40.15 | 385 | 31.29 | 8.00 / 637.00 / 786.00 | 4/-2.00 | 470.38 | 1 | 23 | 121.58 | 0/0 |
| typescript | burst | stream | 10.194 | 12 | 1.18 | 62 | 29.43 | 347.00 / 648.00 / 762.00 | 1/-1.00 | 495.04 | 1 | 23 | 119.45 | 0/0 |
| typescript | slow | poll | 9.307 | 15 | 1.61 | 15 | 3.44 | 2503.00 / 6278.00 / 6302.00 | — | 6258.25 | 8 | 8 | 96.55 | 41/18 |
| typescript | slow | stream | 9.410 | 11 | 1.17 | 12 | 3.40 | 2544.00 / 6402.00 / 6422.00 | — | 6379.74 | 8 | 8 | 96.45 | 45/14 |
| go | idle | poll | 8.119 | 32 | 3.94 | 32 | 0.00 | — | — | — | 0 | 0 | 17.92 | 0/0 |
| go | idle | stream | 8.060 | 4 | 0.50 | 31 | 0.00 | — | — | — | 0 | 0 | 18.00 | 0/0 |
| go | burst | poll | 9.107 | 471 | 51.72 | 471 | 32.94 | 7.00 / 656.00 / 889.00 | 12/-3.00 | 501.15 | 5 | 27 | 24.98 | 0/0 |
| go | burst | stream | 8.492 | 13 | 1.53 | 55 | 35.33 | 346.00 / 679.00 / 757.00 | — | 498.53 | 4 | 27 | 25.91 | 0/0 |
| go | slow | poll | 9.294 | 16 | 1.72 | 16 | 3.44 | 2629.00 / 6415.00 / 6435.00 | — | 6395.06 | 8 | 8 | 22.33 | 40/0 |
| go | slow | stream | 9.219 | 11 | 1.19 | 12 | 3.47 | 2641.00 / 6457.00 / 6476.00 | — | 6434.69 | 8 | 8 | 21.77 | 44/0 |
| python | idle | poll | 8.270 | 32 | 3.87 | 32 | 0.00 | — | — | — | 0 | 0 | 50.80 | 0/0 |
| python | idle | stream | 8.281 | 4 | 0.48 | 30 | 0.00 | — | — | — | 0 | 0 | 52.92 | 0/0 |
| python | burst | poll | 9.604 | 482 | 50.19 | 482 | 31.24 | 7.00 / 595.00 / 871.00 | 8/-4.00 | 468.60 | 1 | 25 | 55.78 | 0/0 |
| python | burst | stream | 9.605 | 14 | 1.46 | 61 | 31.24 | 314.00 / 670.00 / 751.00 | — | 497.79 | 1 | 24 | 55.61 | 0/0 |
| python | slow | poll | 9.380 | 16 | 1.71 | 16 | 3.41 | 2530.00 / 6405.00 / 6425.00 | — | 6359.76 | 8 | 8 | 54.23 | 38/0 |
| python | slow | stream | 9.475 | 11 | 1.16 | 12 | 3.38 | 2660.00 / 6477.00 / 6515.00 | — | 6477.27 | 8 | 8 | 54.09 | 43/0 |
| dotnet | idle | poll | 8.192 | 32 | 3.91 | 32 | 0.00 | — | — | — | 0 | 0 | 72.30 | 0/0 |
| dotnet | idle | stream | 8.180 | 4 | 0.49 | 32 | 0.00 | — | — | — | 0 | 0 | 69.09 | 0/0 |
| dotnet | burst | poll | 8.783 | 481 | 54.77 | 481 | 34.16 | 6.00 / 525.00 / 755.00 | 20/-4.00 | 405.27 | 17 | 23 | 98.03 | 0/0 |
| dotnet | burst | stream | 8.821 | 11 | 1.25 | 57 | 34.01 | 351.00 / 714.00 / 785.00 | 1/-1.00 | 503.66 | 17 | 29 | 101.09 | 0/0 |
| dotnet | slow | poll | 9.268 | 17 | 1.83 | 17 | 3.45 | 2504.00 / 6327.00 / 6348.00 | — | 6301.33 | 8 | 8 | 95.11 | 49/1 |
| dotnet | slow | stream | 9.358 | 11 | 1.18 | 12 | 3.42 | 2604.00 / 6428.00 / 6467.00 | — | 6432.58 | 8 | 8 | 94.28 | 51/1 |
| java | idle | poll | 8.927 | 33 | 3.70 | 33 | 0.00 | — | — | — | 0 | 0 | 152.17 | 0/0 |
| java | idle | stream | 8.937 | 4 | 0.45 | 32 | 0.00 | — | — | — | 0 | 0 | 159.88 | 0/0 |
| java | burst | poll | 11.638 | 414 | 35.57 | 414 | 25.78 | 12.00 / 615.00 / 836.00 | 9/-3.00 | 466.60 | 21 | 23 | 238.38 | 0/0 |
| java | burst | stream | 9.863 | 11 | 1.12 | 55 | 30.42 | 360.00 / 712.00 / 820.00 | — | 498.69 | 20 | 28 | 246.64 | 0/0 |
| java | slow | poll | 10.008 | 16 | 1.60 | 16 | 3.20 | 2519.00 / 6287.00 / 6309.00 | — | 6270.92 | 8 | 8 | 199.48 | 48/11 |
| java | slow | stream | 10.063 | 11 | 1.09 | 12 | 3.18 | 2578.00 / 6353.00 / 6360.00 | — | 6328.45 | 8 | 8 | 199.31 | 43/12 |

## Observations and limits

This single round shows the intended network effect without claiming a universal speedup. Idle receive traffic fell from 32–33 polling calls to four stream windows per SDK, while reconstructed server claim snapshot deltas were 30–32 per stream row. In the burst workload, receive calls fell from 385–482 to 11–14; reconstructed stream claim snapshot deltas were 55–62. Streaming therefore reduced network RPCs much more than database discovery work.

Burst throughput was mixed: TypeScript was about 6% lower, Go about 7% higher, Python and .NET were effectively unchanged, and Java was about 18% higher. Stream p95 ready-to-handler latency was higher for every SDK in this round. The signed samples include negative observations down to −4 ms, shown in the negative-sample column. This reveals clock disagreement but does not bound the unknown PostgreSQL-VM-to-host offset; absolute cross-clock delivery latency must be interpreted with that limitation. Slow-workload throughput was nearly equal between transports.

Every slow row reached eight concurrent handlers and completed successful heartbeats. Refused heartbeat calls are reported alongside successes rather than hidden. Runner RSS stayed similar between transports within each runtime; this is a 50 ms sampled process-tree peak, not an allocation profile or server-memory measurement. The finite receive queue and admission cap are verified by dedicated transport/server tests rather than inferred from RSS.

The run used one shared Apple M3 Max host, one round per row, and sequential subjects. Wall time includes runner startup and shutdown. Runtime warm-up, OS scheduling, PostgreSQL container scheduling, and the order of rows can affect these numbers. The [raw captured artifact](sdk-streaming.raw.md) preserves the measured output; its stream claim column is the process-global cumulative counter. This tracked report converts those values to differences between successive captured stream snapshots (30, 62, 12, 31, 55, 12, 30, 61, 12, 32, 57, 12, 32, 55, 12).

¹ The old capture rendered metrics before `run.stop()`. A reconstructed stream delta may therefore include a late claim from the previous stream row between its snapshot and shutdown. These are snapshot deltas, not exact scenario-attributed claim counts. Poll counts come from per-scenario RPC counters. The corrected harness now takes explicit before/after snapshots within each scenario; this correction does not retroactively change the raw measurements. Receive RPC counts, throughput, latency and RSS do not rely on the reconstructed claim column.


Reproduce with the release `perf_sdk_multilanguage` target, selecting
`streaming::compare_polling_and_streaming --ignored --nocapture --test-threads=1`,
with the five SDK runtimes and Docker available. The execution used `SAGAT_SDK_REUSE=1`
after the final conformance preparation. Production and SDK sources were subsequently
committed as `517c657`; the benchmark and its reporting correction accompany this report.
