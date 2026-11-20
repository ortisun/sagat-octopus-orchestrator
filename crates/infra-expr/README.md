# crates/infra-expr

## Overview

sagat-infra-expr is a pure bounded evaluator for a CEL-like subset and scalar templates.

## Interface

- Compiled, Env, Template and public ceilings expose compile/evaluate/render operations. [src/eval.rs:Env] [src/eval.rs:Compiled] [src/template.rs:Template] [src/limits.rs:MAX_SOURCE_BYTES]
- [src](./src/README.md) — Pure bounded evaluator for a small CEL-like subset and scalar templates. [src/error.rs:ExprError]

## Invariants

- The grammar/function set is closed and source/tokens/depth/lists/strings/rendered output are bounded. [src/parser.rs:function] [src/limits.rs:MAX_TOKENS] [src/limits.rs:MAX_RENDERED_BYTES]

## Side Effects

- Evaluation performs deterministic CPU/memory work without I/O or clocks. [src/eval.rs:Compiled::eval] [src/template.rs:Template::render]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-expr --locked
```

These commands were not run in this documentation-only pass.
