# Validation report: stream operators (design 042)

Measurements and acceptance runs of design 042, dated, with the environment. Numbers are what was measured, not
promises; NFR-042-001 states its targets on a stated environment.

## 2026-11-14 — T042-9: a collection chain at scope entry (slice S1)

**Environment.** Apple M3 Max, 16 cores, 48 GiB, macOS 26.3, shared with other sessions' builds and test runs (not an
idle machine). Rust 1.100.0. PostgreSQL 16 (`postgres:16-alpine` in Docker 29.9.3, the test fixture's container, the
scheduler and the database on the same host). Branch `feature/042-collection-chain`.

**Commands.**

```sh
cargo test -p sagat-infra-ir --lib collection_chain::tests::measure -- --ignored --nocapture            # debug
cargo test -p sagat-infra-ir --lib --release collection_chain::tests::measure -- --ignored --nocapture  # release
RUST_TEST_THREADS=1 cargo test -p sagat-tests --test integration_workflow_primitives -- --ignored collection_operators_measure --nocapture
```

**The evaluator alone** (`collection_chain` over materialised elements, 20 calls each):

| Chain | Build | p50 | p95 | max |
|---|---|---|---|---|
| Eight stages (`map`, `filter`, `map`, `distinct`, `map`, `skip`, `map`, `take`) over 1,000 small integers | release | 0.93 ms | 1.26 ms | 1.26 ms |
| Eight `map` stages of 4 MiB each (sixteen 256 KiB elements): the worst case at the ceilings | release | 11.80 ms | 13.13 ms | 13.13 ms |
| Eight `map` stages of 4 MiB each (1,000 elements of 4 KiB) | release | 15.29 ms | 16.75 ms | 16.75 ms |
| Eight stages over 1,000 small integers | debug | 5.74 ms | 6.28 ms | 6.28 ms |
| Eight `map` stages of 4 MiB each (sixteen 256 KiB elements) | debug | 505.79 ms | 527.06 ms | 527.06 ms |
| Eight `map` stages of 4 MiB each (1,000 elements of 4 KiB) | debug | 505.21 ms | 513.17 ms | 513.17 ms |

**Scope entry through the scheduler** (`advance_collection_from_run`, debug build of the test binary, 20 entries each,
one replica, real PostgreSQL; the entry includes the run read, the evaluation in its blocking task and the entry
transaction):

| Entry | p50 | p95 | max |
|---|---|---|---|
| `forEach` without a chain, 1,000 small elements | 82.36 ms | 122.67 ms | 211.09 ms |
| The same list with the eight stages above | 44.37 ms | 59.27 ms | 142.87 ms |
| At the ceilings: eight `map` stages of 4 MiB over a blob of sixteen 256 KiB elements | 748.07 ms | 765.98 ms | 767.33 ms |

**`scheduler_tick_seconds`.** The duration of a whole `tick()` (what the histogram records) carrying one entry, each in
a fresh database so that the tick carries nothing else, five ticks each, debug build:

| Tick | p50 | p95 |
|---|---|---|
| One worst-case chained entry (blob, eight `map` stages of 4 MiB) | 855.64 ms | 906.83 ms |
| One entry without a chain (1,000 small elements) | 276.48 ms | 330.15 ms |

**Re-entries.** `collection_chain_reentries_total` grew by 0 for every reason during the measurement: no entry was tried
twice.

**Reading.**

- The chain's own cost is small next to the entry transaction for a list of small elements: under 1.3 ms at p95 in
  release, so NFR-042-001's 20 ms for eight stages over 1,000 small elements holds for the evaluation. The two entry
  series above do not isolate it: the first series ran first, on a cold database and cold caches, and with 20 samples on
  a shared machine the order dominates the 1 ms the chain adds (the chained series was faster). That comparison is
  inconclusive, not a speed-up.
- The worst case at the ceilings costs about 13 ms in release and about 0.5 s in a debug build, where it is half of the
  1-second wall-time budget. A tick carrying one such entry took about 0.58 s longer than a tick with an unchained
  entry (debug); in release the evaluation is about 40 times cheaper. A replica built in debug must not be used to judge
  the `wall_time` ceiling (G042-009): release builds are what ship.
- The evaluation runs in a blocking task after the fence read and before the entry transaction, so it holds no lock and
  no runtime worker; the tick waits for it because the collection pass awaits its candidates.

Not measured here: a release build of the whole engine at the ceilings, several chained entries in one tick (the pass
runs up to `concurrency` candidates at once, each in its own blocking task), and a separate database host.

## 2026-11-15 — after the review of S1

**Environment.** As on 2026-11-14 (Apple M3 Max, 16 cores, 48 GiB, macOS 26.3, shared machine; Rust 1.100.0;
PostgreSQL 16 in Docker), branch `feature/042-s1-review-fixes`.

**FR-042-012, the entry of a `forEach` without operators** (`collection_operators_unchanged`). The input is now a raw
JSON document, `{"lines": [{"v": 1, "k": "a"}, 2, "x", [1.0, -0], -0, -0.0, 1e2]}`, so that `-0` reaches the codec
(the `json!` macro writes it as the integer `0`); `serde_json` reads `-0` as the double `-0.0`. The test was run on
three trees, each with the same test file and nothing else changed:

```sh
# at each commit: git checkout <commit> -- crates apps tests Cargo.lock Cargo.toml, then the test file of this branch
RUST_TEST_THREADS=1 cargo test -p sagat-tests --test integration_workflow_primitives -- collection_operators_unchanged --nocapture
```

| Tree | Statements | Transactions | Rows written | Snapshot bytes |
|---|---|---|---|---|
| `a4f6e9db` (develop before design 042) | 69 | 2 | 22 | `[{"k":"a","v":1},2,"x",[1.0,-0.0],-0.0,-0.0,100.0]` |
| `0e94bf0e` (slice S0) | 69 | 2 | 22 | the same |
| this branch | 69 | 2 | 22 | the same |

The 63, 2 and 17 of 2026-11-14 were the same measurement over four elements; the pin now holds the seven above.

**Resident memory of a chain at the ceilings** (release build of the `sagat-infra-ir` test binary, measured with
`/usr/bin/time -l` on the binary itself):

```sh
B=$(cargo test -p sagat-infra-ir --lib --release --no-run 2>&1 | grep -o '/Users[^)]*sagat_infra_ir-[a-f0-9]*')
/usr/bin/time -l $B --exact collection_chain::tests::an_empty_output_is_an_empty_collection
/usr/bin/time -l $B --exact collection_chain::tests::measure_the_chain_at_its_ceilings --ignored --nocapture
```

| Process | Maximum resident set size |
|---|---|
| The test binary running a chain over an empty list (the baseline) | 7.8 MB |
| The measurement: eight `map` stages of 4 MiB, twenty times each, sixteen 256 KiB and 1,000 4 KiB elements | 66.9 MB |

About 59 MB above the baseline for a chain whose stages are 4 MiB: about fourteen times the stage's serialised size,
since an element is held as a `serde_json::Value` and as its bytes, for the stage's input and its output. With two
chains at once per replica (D042-153) that is about 120 MB a replica may hold for chains. The evaluator's times in the
same run: eight stages over 1,000 small elements p95 1.04 ms; eight `map` stages of 4 MiB p95 15.16 ms (sixteen
elements) and 16.33 ms (1,000 elements).

**A tick's hold-up.** One tick enters at most four chained collections, two evaluated at a time (D042-153). From the
numbers above, the chains of one tick hold it for at most about 60 ms of evaluation in a release build; this is computed,
not measured as a whole tick.
