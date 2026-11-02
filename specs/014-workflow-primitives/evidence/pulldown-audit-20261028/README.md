# gRPC build-dependency audit correction — 2026-10-28

Refs: T014-043, T033-007. Scope: existing workspace build dependency only.

CI PR56/run40686861523/job122811344560 and the local audit refused
pulldown-cmark0.13.4 with RUSTSEC-2026-0414. The selected patch0.13.5 and checksum
already occur in the merged official Rust SDK lockfile. The diff changes only
this package's version/checksum; all other dependency entries and audit policy
are unchanged, as independently compared in pulldown-lock-comparison.json.

| Gate | Actual result |
| --- | --- |
| Workspace audit RED |exit1, exact RUSTSEC-2026-0414 refusal |
| Workspace audit after patch |exit0,631 dependencies,1.70s |
| Independent Rust SDK audit |exit0,227 dependencies,1.58s |
| gRPC crate |21 unit +2 compatibility +3 scoped protocol tests passed;29.29s command |
| Generated Rust worker source |byte-identical before/after; exact digest in comparison JSON |
| Strict workspace/all-target Clippy |exit0;169.35s |
| Format / architecture / agent configuration |all exit0;24 workspace crates |

Source: [RustSec advisory](https://github.com/RustSec/advisory-db/blob/main/crates/pulldown-cmark/RUSTSEC-2026-0414.md)
and [upstream patch release](https://github.com/pulldown-cmark/pulldown-cmark/releases/tag/v0.13.5).
The advisory database revision is recorded in the lock comparison. The CI excerpt
contains only the refusal, not checkout/authentication setup. No protocol baseline
was blessed, no runtime Markdown endpoint was introduced, and no advisory ignore
was added. Existing audit-policy exceptions remain unchanged. These focused checks
do not close general nesting or final release acceptance; new CI must pass.

A bounded read-only review identified imprecise prose about the transitive build
chain and parser flags. The ledger now names the exact chain and pinned option
expression; the functional lockfile diff is unchanged.
