# Benchmark: payload classification (feature 016)

Measured by `tests/performance/payload-classification/main.rs`. These are observations on one shared machine, not latency or capacity promises.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Rust: rustc 1.98.0 (88d9e12ae 2026-08-18)
- Load average when the run began: 18.93 19.42 23.20 (other work on the machine adds noise)
- Build profile: release
- Commit: ff603681
- Harness: the registered `perf_payload_classification` target
- Execution: one in-process thread, monotonic `Instant`; no database or network
- Allocation measurement: macOS `heap <pid>` live-allocation snapshots; counts requested live nodes and bytes, not allocator fragmentation or RSS

## Dataset and method

- Two JSON payloads of exactly **262144 bytes**: a wide object with 128 public string values against 1065 declared string subschemas, and 17 nested public arrays with a terminal string beyond the response-depth boundary.
- Each carrier is exactly **65536 bytes**. The wide carrier spends almost all of that limit on compiled property/type/length constraints; `$comment` only fills the final sub-128-byte remainder. The deep carrier uses `$comment` padding and measures the depth boundary, not worst-case validator complexity.
- Cold rows create a new classifier/version for each of 60 samples. Warm rows prime one immutable `(version, carrier)` and then measure 300 cache hits. Timing covers `Classifier::project`; response encoding is performed afterward and reported separately.
- Live allocations are macOS `heap <pid>` snapshots: before filling the cache, while the cache holds what its budget admits after 256 distinct versions of the wide carrier were compiled, and after the classifier is dropped. Five further carriers are compiled one at a time and their retained heap is compared with the weight the cache charged for them.

## Latency and response size

| Shape | Cache | Samples | p50 | p95 | p99 | Maximum encoded response | Failures |
|---|---|---:|---:|---:|---:|---:|---:|
| wide (128 public strings) | cold | 60 | 1.972 ms | 2.726 ms | 4.700 ms | 146241 | 0 |
| wide (128 public strings) | warm | 300 | 0.621 ms | 0.890 ms | 1.280 ms | 146241 | 0 |
| deep (17 nested arrays) | cold | 60 | 0.230 ms | 0.442 ms | 0.668 ms | 2381 | 0 |
| deep (17 nested arrays) | warm | 300 | 0.174 ms | 0.242 ms | 0.366 ms | 2381 | 0 |

## Walk cost

Steps are the walk's own counter (D641), not time. The limit is 4000000.

| Shape | Steps | Share of the limit | Bytes kept for number spellings |
|---|---:|---:|---:|
| wide (128 public strings) | 3545 | 0.09% | 0 |
| deep (17 nested arrays) | 193 | 0.00% | 0 |

## Estimated weight against measured heap

| Carrier | Serialized bytes | Estimated weight (bytes) | Measured retained heap (bytes) | Measured / estimated |
|---|---:|---:|---:|---:|
| wide object (largest carrier) | 65445 | 4192576 | 1819856 | 43.4% |
| 3,000-branch allOf | 57045 | 3654976 | 3079552 | 84.3% |
| large enum | 38934 | 2495872 | 855008 | 34.3% |
| 32 patterns (the per-carrier cap) | 1933 | 4322112 | 2576416 | 59.6% |
| minimal object | 88 | 9728 | 4928 | 50.7% |

## Cache behavior and retained heap

| Measurement | Result |
|---|---:|
| wide warm hit ratio (prime included) | 99.668% (300 hits, 1 miss) |
| deep warm hit ratio (prime included) | 99.668% (300 hits, 1 miss) |
| cache after 256 distinct wide carriers | 15 entries, estimated weight 62976000 bytes of a 67108864 byte budget, 241 evictions |
| baseline live heap | 7712 allocations / 2064128 bytes |
| live heap with the cache full | 282324 allocations / 27517792 bytes |
| after dropping the classifier | 7712 allocations / 2064128 bytes |
| full-cache delta | 274612 allocations / 25453664 bytes (24.27 MiB) |
| live bytes released when the classifier was dropped | 25453664 bytes (24.27 MiB) |
| one more distinct carrier | 242 total evictions |
| revisiting the first carrier | 243 total evictions (it had been evicted, so it compiled again) |
| emitted `classification_cache_total` during this run | hit=600, miss=385, evict=243 |

The cache held **15 entries** (limit 256) with an estimated weight of **60.06 MiB** (budget 64 MiB) after 241 evictions, and the allocator-requested live bytes it retained were **24.27 MiB**. The estimate is an upper bound on every shape in the table above, so the budget bounds compiled-validator heap. It is not process RSS: `heap` does not count allocator fragmentation, resident pages or the per-thread caches that regex matchers build while serving requests.

## Bounds and failures

- Both maximum payloads and both maximum carriers were accepted. All 720 measured projections succeeded.
- Maximum encoded responses were 146241 bytes (wide) and 2381 bytes (deep), below the 262144-byte response bound. Both are truncated by documented disclosure bounds; truncation is a successful bounded response.
- Benchmark harness failure: none.
- No latency threshold existed before this run (NFR-016-003), so the percentile table establishes a baseline rather than claiming an SLO.
- `heap` reports requested live allocation nodes and bytes. It does not include allocator fragmentation or resident pages, and the live-byte result must not be presented as RSS.

## Gate status

SC-016-008's required measurements are present for this environment: maximum payload/schema, p50/p95/p99, cache ratio, live allocation count, retained logical heap, encoded response size and failures. The numeric bounds held in this run. The cache budget is an estimate of compiled bytes (decision D643): the table above shows it above the retained heap for every measured shape, in the profile named under Environment. It is a bound on allocator-requested heap, not on RSS, and it assumes the shapes measured here are representative; a carrier shape that compiles larger than 64 bytes per serialized byte plus the pattern allowance would break it, which is why the measurement is repeated whenever the carrier limits or the validator version change.
