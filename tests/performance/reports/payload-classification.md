# Benchmark: payload classification (feature 016)

Measured by `tests/performance/payload-classification/main.rs`. These are observations on one shared machine, not latency or capacity promises.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Rust: rustc 1.98.0 (88d9e12ae 2026-08-18)
- Load average when the run began: 34.50 33.23 25.75 (other work on the machine adds noise)
- Build profile: debug (unoptimised)
- Commit: c3f3858b plus uncommitted changes to tracked files
- Harness: the registered `perf_payload_classification` target
- Execution: one in-process thread, monotonic `Instant`; no database or network
- Allocation measurement: macOS `heap <pid>` live-allocation snapshots; counts requested live nodes and bytes, not allocator fragmentation or RSS

## Dataset and method

- Two JSON payloads of exactly **262144 bytes**: a wide object with 128 public string values against 1065 declared string subschemas, and 17 nested public arrays with a terminal string beyond the response-depth boundary.
- Each carrier is exactly **65536 bytes**. The wide carrier spends almost all of that limit on compiled property/type/length constraints; `$comment` only fills the final sub-128-byte remainder. The deep carrier uses `$comment` padding and measures the depth boundary, not worst-case validator complexity.
- Cold rows create a new classifier/version for each of 60 samples. Warm rows prime one immutable `(version, carrier)` and then measure 300 cache hits. Timing covers `Classifier::project`; response encoding is performed afterward and reported separately.
- Live allocations are three macOS `heap <pid>` snapshots: before filling the cache, while 256 compiled entries are retained, and after the classifier is dropped.

## Latency and response size

| Shape | Cache | Samples | p50 | p95 | p99 | Maximum encoded response | Failures |
|---|---|---:|---:|---:|---:|---:|---:|
| wide (128 public strings) | cold | 60 | 24.051 ms | 28.605 ms | 41.538 ms | 146241 | 0 |
| wide (128 public strings) | warm | 300 | 7.462 ms | 9.076 ms | 13.631 ms | 146241 | 0 |
| deep (17 nested arrays) | cold | 60 | 5.001 ms | 5.327 ms | 9.325 ms | 2381 | 0 |
| deep (17 nested arrays) | warm | 300 | 3.013 ms | 3.232 ms | 3.920 ms | 2381 | 0 |

## Cache behavior and retained heap

| Measurement | Result |
|---|---:|
| wide warm hit ratio (prime included) | 99.668% (300 hits, 1 miss) |
| deep warm hit ratio (prime included) | 99.668% (300 hits, 1 miss) |
| logical full cache | 256 entries × 65536 bytes = 16777216 bytes |
| baseline live heap | 7341 allocations / 2040352 bytes |
| live heap at logical full cache | 4693937 allocations / 436400800 bytes |
| after dropping the classifier | 7341 allocations / 2040352 bytes |
| full-cache delta | 4686596 allocations / 434360448 bytes (414.24 MiB) |
| live bytes released when the classifier was dropped | 434360448 bytes (414.24 MiB) |
| 257th distinct carrier | 1 eviction |
| revisiting the first carrier | 2 total evictions (the first was absent) |
| emitted `classification_cache_total` during this run | hit=600, miss=380, evict=2 |

The cache retained **414.24 MiB** of allocator-requested live bytes at the logical 16 MiB/256-entry limit, so the current 16 MiB control is a schema-byte weight and does **not** bound compiled-validator heap or process RSS. Before claiming a physical memory budget, measure per-entry compiled weight across representative schemas and either include that weight in eviction or lower the entry/byte limits. This benchmark does not change cache semantics.

## Bounds and failures

- Both maximum payloads and both maximum carriers were accepted. All 720 measured projections succeeded.
- Maximum encoded responses were 146241 bytes (wide) and 2381 bytes (deep), below the 262144-byte response bound. Both are truncated by documented disclosure bounds; truncation is a successful bounded response.
- Benchmark harness failure: none.
- No latency threshold existed before this run (NFR-016-003), so the percentile table establishes a baseline rather than claiming an SLO.
- `heap` reports requested live allocation nodes and bytes. It does not include allocator fragmentation or resident pages, and the live-byte result must not be presented as RSS.

## Gate status

SC-016-008's required measurements are present for this environment: maximum payload/schema, p50/p95/p99, cache ratio, live allocation count, retained logical heap, encoded response size and failures. The numeric bounds held in this run. D281 limits serialized schema bytes, not compiled heap or RSS; no physical 16 MiB memory guarantee is inferred. The registered target and concurrent cache accounting/admission tests must pass before T016-053 closes. This measurement preserves the approved schema-weight policy and needs no new owner decision.
