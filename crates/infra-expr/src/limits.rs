//! Every bound of the subset in one place. They exist because expressions are written by tenants:
//! parsing and evaluating one must use a small, predictable amount of stack, memory and time.

/// Longest accepted source text.
pub const MAX_SOURCE_BYTES: usize = 1024;
/// Most tokens in one expression. Every AST node consumes at least one token, so this also bounds
/// the size of the tree and the recursion depth of evaluation.
pub const MAX_TOKENS: usize = 256;
/// Deepest nesting of parentheses, lists, calls, ternaries and prefix operators.
///
/// Measured with the reference implementation: a 2 MiB stack in a debug build overflows at a
/// nesting depth of 12, which is why this parser is not the reference one. With this parser the
/// worst accepted shape needs about 192 KiB of stack in a debug build and 32 KiB in release.
pub const MAX_DEPTH: usize = 16;
/// Most elements in a list literal.
pub const MAX_LIST_LITERAL: usize = 64;
/// Longest string an evaluation may build.
pub const MAX_STRING_BYTES: usize = 64 * 1024;
/// Most elements in a list an evaluation may build.
pub const MAX_LIST_ELEMENTS: usize = 4096;
/// Most `${...}` parts in a template.
pub const MAX_TEMPLATE_PARTS: usize = 8;
/// Longest rendered template (an idempotency key).
pub const MAX_RENDERED_BYTES: usize = 512;
