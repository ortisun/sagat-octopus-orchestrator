# Temporary dependency overrides

## rskafka 0.6.0 / Zstandard (2026-10-27)

The release audit found four vulnerabilities and ten unsound-API advisories in the Zstandard
family pulled by rskafka 0.6.0. Its current registry release requires zstd 0.13; that requirement
cannot resolve the patched 0.14.3. Removing `compression-zstd` would break consumption of existing
Kafka topics using that compression. No advisory ignore is added.

The root workspace temporarily patches rskafka to `vendor/rskafka`. Its 84 files are the exact
registry archive contents, except that the normalized Cargo.toml requires zstd 0.14.3 instead
of 0.13. There are no upstream Rust source changes. The root lock pins zstd 0.14.3,
zstd-safe 8.2.0 and zstd-sys 2.1.2; standalone SDK dependencies remain independent.
This is a security update to the affected codec, without a Kafka protocol or public API change.

Upstream archive: [rskafka 0.6.0](https://crates.io/api/v1/crates/rskafka/0.6.0/download),
SHA256 `849b87417a191e37e16b8893eba8928bbd10e1989e6b57c4f8531549eaa29bdc`.
It was checked against the prior root Cargo.lock before extraction. License files are retained.
`vendor/rskafka.provenance.json` records every original content digest; `scripts/check-vendor.py`
pins the canonical manifest digest and 84-file count independently, reverses only the one reviewed
manifest replacement and refuses changed, missing, additional or linked files. CI runs that check
and its negative controls. Docker copies and refreshes the snapshot alongside application sources.

The upstream reports include [context reuse](https://rustsec.org/advisories/RUSTSEC-2026-0391.html),
[legacy-frame decompression](https://rustsec.org/advisories/RUSTSEC-2026-0403.html),
[seekable decoding](https://rustsec.org/advisories/RUSTSEC-2026-0398.html) and
[the wasm allocator](https://rustsec.org/advisories/RUSTSEC-2026-0410.html).
The lock update also removes all ten reported unsound APIs from the dependency graph.

Verification commands:

```sh
python3 scripts/check-vendor.py
python3 -m unittest discover -s scripts/tests -p test_vendor_integrity.py
cargo audit --deny warnings
cargo test -p sagat-tests --test e2e_events_webhooks_messaging --locked -- us03::us03_kafka_redelivery_poison_messages_and_a_lost_offset_commit_leave_one_run_per_event --exact --nocapture
```

The real Kafka case now produces one partition with Zstandard and one without compression;
it retains its original dedupe, poison-message, offset-loss and restart assertions. An audit
pass alone is not its runtime result. Actual results are recorded in the feature-004 validation
notes after execution. Use an owned outside-repository target and the documented Docker fixture.

Remove this override, its snapshot and guard when a published compatible rskafka release depends
on a patched Zstandard version. Resolve normally, inspect the lock diff, run the same audit and
broker regressions, and keep this removal condition in the review. Do not alter vendored Rust
source as an incidental fix; any further override needs its own source verification and review.
