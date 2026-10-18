# Dormant scoped Finish application contract

Refs:T014-014..017/040; local B1b.2b.6/.11 only. See command JSON/logs. Public test was
installed before module/export and failed E0432 naturally; the implementation then passed6/6.
Independent Python vector uses152 bytes; exact payload/control hashes retained. Two fresh
bounded implementation reviews ended with no findings after fixing invalid-fingerprint and
same-length owner oracles. Flat-context decision separately reviewed None, not natively tested.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
cargo test --locked -p sagat-application --test scoped_leaf_finish
cargo clippy --locked -p sagat-application --all-targets -- -D warnings
cargo fmt --all -- --check
python3 scripts/check-architecture.py
```

No native adapter, source proof, caller, privilege, release activation or nested guarantee.
Source hashes below record the tested artifacts after verification; no before/after freeze
claim is inferred. External second review was declined earlier; no external CLI invoked.
