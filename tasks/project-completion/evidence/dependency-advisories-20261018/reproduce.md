# Integrated dependency advisory gate

Source checkpoint83c2cada. Same CI pinned cargo-audit0.22.2. cargo audit --deny warnings exits0
in2.29s, scans627Cargo.lock dependencies against1338RustSec advisories from database commit
999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b. Raw output/version and exact commands retained.
Current .cargo/audit.toml policy unchanged, including five existing justified exceptions:
RUSTSEC-2023-0071 and RUSTSEC-2026-0346/0347/0348/0349. Passing under the current policy is not
zero known advisories. No Cargo.lock, dependency, ignore policy or CI change. This required gate
is independent of pending scoped-capacity implementation. Other integrated release gates remain
open; this alone does not close T014-043. Rerun if dependency/policy/advisory inputs change.
