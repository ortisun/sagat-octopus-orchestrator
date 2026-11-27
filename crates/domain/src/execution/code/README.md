# crates/domain/src/execution/code

## Overview

What the domain knows of a code run (design 044, workflow as code). Pure: no storage, no clock, no hashing library.

## Interface

- `CommandKind` names the 22 kinds of command of `worker.proto`, with the chain's lower-case name of each and the work
  and terminal predicates. [chain.rs:CommandKind]
- `CommandDigester` is the port of the two byte-level operations of the chain (SHA-256 and the RFC 8785 bytes of
  `d(k)`), implemented by `Sha256CommandDigester` in `crates/infra-ir`. [chain.rs:CommandDigester]
- `CommandChain` is `H(k)` with `k`; `extend` and `extend_all` fold commands into it, `restore` reads it back.
  [chain.rs:CommandChain] [chain.rs:ChainedCommand] [chain.rs:ChainError]

## Invariants

- `H(0)` is 32 zero bytes and `H(k) = SHA-256(H(k − 1) ‖ uint32 big-endian length of d(k) ‖ d(k))` (Technical Plan 6.2,
  the corpus README); the domain folds it and the port only hashes and serializes. [chain.rs:CommandChain::extend]
- Command numbers go 1, 2, 3...: a command that is not numbered one more than the count is refused before anything is
  hashed. [chain.rs:CommandChain::extend]

## Verification

`cargo test -p sagat-domain --locked execution::code`; the golden equality of the chain is
`cargo test -p sagat-infra-ir --locked --test command_chain`.
