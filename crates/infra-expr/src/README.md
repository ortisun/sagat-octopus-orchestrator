# crates/infra-expr/src

## Overview

Pure bounded evaluator for a small CEL-like subset and scalar templates.

## Interface

- `ExprError`, `Env`, `Compiled` compile/variables/eval/eval_bool/eval_key and `Template` compile/variables/render expose parsing, evaluation and typed failure. Public limits centralize every resource ceiling. [error.rs:ExprError] [eval.rs:Env] [eval.rs:Compiled] [template.rs:Template] [limits.rs:MAX_SOURCE_BYTES] [limits.rs:MAX_TOKENS] [limits.rs:MAX_DEPTH] [limits.rs:MAX_LIST_LITERAL] [limits.rs:MAX_STRING_BYTES] [limits.rs:MAX_LIST_ELEMENTS] [limits.rs:MAX_TEMPLATE_PARTS] [limits.rs:MAX_RENDERED_BYTES]

## Invariants

- The grammar permits listed scalar/list/access/logical/arithmetic/comparison/in operations and a closed function/string-method set; comprehensions, regexes and unknown calls are rejected. Limits include 1024 source bytes, 256 tokens, depth 16 and 64 literal items. [parser.rs:Expr] [parser.rs:function] [parser.rs:method_named] [lexer.rs:lex] [parser.rs:Parser::enter]
- Integer arithmetic is checked, division by zero errors and remainder requires integers. Concatenation and templates are bounded (64 KiB strings, 4096 list elements, eight template expressions, 512 rendered bytes); embedded template results must be scalar. Computed missing keys are redacted as `<computed>`. [eval.rs:arithmetic] [eval.rs:add] [eval.rs:select] [template.rs:Template::render] [limits.rs:MAX_RENDERED_BYTES]

## Side Effects

- Evaluation/rendering and environment construction use deterministic CPU/memory work and bounded allocations without I/O, clocks or telemetry. [eval.rs:Compiled::eval] [eval.rs:Env::with] [template.rs:Template::render]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-expr --locked
```

These commands were not run in this documentation-only pass.
