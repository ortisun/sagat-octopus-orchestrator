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
- `ActivityEnvelope` holds a code version's activity entries (`EntryName`: exact or `<stem>.*`; `PublisherFields`);
  `covering` finds a name's one entry and `admit` checks a command's repeated publisher fields against it, answering
  the entries of its compensation and its lookup. Construction takes the names of `approvalPolicies`. [envelope.rs:ActivityEnvelope] [envelope.rs:ActivityEntry]
  [envelope.rs:EntryName] [envelope.rs:AdmittedActivity] [envelope.rs:EnvelopeRefusal] [envelope.rs:EnvelopeError]

## Invariants
- An exact entry wins over a prefix entry and a longer prefix over a shorter one, whatever the order of the entries;
  only an exact entry lists secrets; no entry is under `script.` or `sagat.`; every `compensateWith` and
  `reconcileWith` names a covered activity, whose own entry gives the derived step its secrets. [envelope.rs:ActivityEnvelope::new]
  [envelope.rs:ActivityEnvelope::covering]
- A `requiresApproval` names a declared policy, and an entry covered by a broader prefix entry restates its
  `requiresApproval`, `highRisk: true` and `irreversible: true` (Decision 044-253). [envelope.rs:ActivityEnvelope::new]
- A command may repeat a publisher field of its covering entry only with the entry's value (secrets as sets); a field
  the entry does not set, or sets otherwise, is refused with its member (Decision 044-241). [envelope.rs:ActivityEnvelope::admit]

- `H(0)` is 32 zero bytes and `H(k) = SHA-256(H(k − 1) ‖ uint32 big-endian length of d(k) ‖ d(k))` (Technical Plan 6.2,
  the corpus README); the domain folds it and the port only hashes and serializes. [chain.rs:CommandChain::extend]
- Command numbers go 1, 2, 3...: a command that is not numbered one more than the count is refused before anything is
  hashed. [chain.rs:CommandChain::extend]

## Verification

`cargo test -p sagat-domain --locked execution::code`; the golden equality of the chain is
`cargo test -p sagat-infra-ir --locked --test command_chain`.
