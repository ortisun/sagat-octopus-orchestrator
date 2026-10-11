# ADR-0018 — Publishing is one transaction, the active pointer has a history that is only added to, and an approval is of a document

Status: **Accepted** | Date: 2026-10-11 | Feature: 011 | Builds on ADR-0002 (PostgreSQL is the authority), ADR-0003 (the IR is declarative, and a version is its canonical form with a digest) and ADR-0014 (transactions begin where nothing can cut them off); changes none

## Context

Feature 011 asks that workflows can be kept in Git, published as immutable versions, promoted from dev to
staging to production with approvals, and rolled back without rewriting anything, with an audit trail of who
asked, who approved, who published, which digest and why. The platform already had immutable versions with a
digest, runs pinned to the version they started on, and a pointer (`workflows.active_version_id`) that every
publication moved. What it did not have: any way to move the pointer without publishing, any record of where it
had been, any control over who may publish into production, and a way to carry a version from one namespace to
another. The tempting answers each leave a window in which the record and the truth disagree:

1. **A rollback that republishes the old document as a new version.** It works with what exists, but it rewrites
   the story ("version 5 is the document of version 3"), loses which version was running when something
   failed, and makes "which documents have we ever run" depend on a search for equal digests.
2. **An approval as a flag on the workflow or the draft** ("approved: true"). An approval is a statement about a
   document at a moment; a flag survives the edit that makes it false.
3. **Approvals checked by the caller** (the UI or the CLI refuses, the API accepts). Every other client is then
   a way around them.
4. **A history kept by the application with the same permission as the data**, so that whatever can move the
   pointer can also edit the record of having moved it.
5. **A promotion that copies the workflow definition from the source namespace to the target as a new draft**
   that the target's owners publish. It is the same document only if nobody edits it, which is the thing to
   prove.

## Decision

**1. The pointer moves in one transaction with its record.** Publishing (and promoting, which publishes) takes
the workflow row lock, checks what must hold, stores the version, the compensation plans, the artifacts the
version pins, the change of the pointer, the triggers of the version and the audit entry, and commits; the
activation of an existing version (a rollback, or the way forward again) does the same without creating a
version. Nothing a reader can see is half done, and what a crash leaves is the state before.

**2. The history of the pointer is a table that is only added to.** `workflow_activations` has one row per
change (`PUBLISH`, `PROMOTE`, `ROLLBACK`, `ACTIVATE`) with who, why, the version before, the request that
authorised it and, for a promotion, the version it came from. The database refuses to update, delete or
truncate it (a trigger, with the error `restrict_violation`), and the runtime role has no more than `SELECT`
and `INSERT`. The last row of a workflow **is** its pointer. A row is stored strictly after the latest one of
its workflow, whatever the clock of the replica that made it says, inside the locked transaction: the order of
the rows is the order in which the pointer moved, and this was a defect, not a refinement (D198). The history
that existed before the migration is rebuilt from the versions, and a pointer that was moved by publishing an
identical document again is closed with one row that says so.

**3. A rollback is a move of the pointer to a version that exists.** It needs its own permission
(`version:activate`: operators and administrators), a reason, and it can carry the version the caller saw
active (`expectedVersion`): if someone moved the pointer meanwhile, the answer is `409 ACTIVE_VERSION_CHANGED`
with where it is now. When the caller does not say, the service compares against the version it read and
refuses if that is no longer the active one when it writes. A move that would make a caller or an emitter fail
(a trigger removed, an input property removed, retyped or newly required, an output property removed or
retyped, an awaited event type gone) is refused as `BREAKING_CHANGE`, with the codes, unless the caller says
`acceptBreaking`. Nothing is rewritten: runs in flight keep the version they started on, every version stays
what it was, and the event triggers become those of the version the pointer holds, registered again with the
same ids so a duplicate event delivered across the move starts nothing twice.

**4. An approval is of a document.** A namespace has a policy: how many people other than the author must
approve a publication into it (production by default one, anywhere else none, an administrator can set 0 to 3,
audited with a reason). A namespace that asks refuses a direct publish with `409 APPROVAL_REQUIRED`; the draft
goes through a **request**, which names the draft revision and the digest of the document. Approvers approve
that. Neither the person who asked nor the person who saved the document decides (the platform keeps the last
person to save the draft), a person decides once, one rejection ends it, the author may withdraw it,
editing the draft supersedes it, and there is one open request per workflow (a partial unique index). When the
request is published the approvals are counted again against the policy **in force then**: a policy raised
after the approvals asks for the extra ones, and a request published twice makes one version. The check is in
the transaction that publishes, not in a client.

**5. A promotion publishes the same document.** `promote` takes the canonical document of a version, byte for
byte, and publishes it into another namespace of the same environment or a later one (dev, staging,
production) as the workflow of the same name, creating it if need be. It stops at what the target does not have
(a secret the document names), at a draft in the target that holds changes nobody published (it never
overwrites one), and at a change that breaks the target's callers (unless accepted). The target's policy
decides whether it is published at once or becomes a request bound to that document; if the target already runs
that document nothing changes. The history of the target says where the version came from.

**6. The artifacts a version pins cannot be removed from under it.** Each version records the digests of the
script artifacts its document pins (`workflow_version_scripts`), with a foreign key `ON DELETE RESTRICT`. The
database refuses to remove an artifact any published version depends on, because a rollback can make any
published version active again. This is stricter than the specification's "used by active runs" and is the
guard the specification asks for until a retention policy exists; the policy itself is not built (G193).

**7. The command line is a client of the public API and nothing more.** `sagat` reads a manifest kept in Git,
checks every document with the same codec the server uses (same crate: there is no second implementation to
drift), compares digests with the active versions, shows what would change, and applies it through the REST
operations anyone can call. Every approval, refusal and breaking-change rule above is enforced by the server;
the tool adds convenience (the diff, the plan, exit codes a pipeline can branch on, a refusal of breaking
changes before anything is sent) and never authority. It can be interrupted and run again: creating a workflow and
publishing a new version carry a key derived from the document, so the same publication is the same operation;
re-activating a version that was published before gets a key of its own each time, because a key that has been used
is answered as it was whatever the pointer says now.

## Consequences

- A pipeline can promote and roll back with the tools it has, and the answer to "who decided that this reached
  production, on what document and why" is a query over two tables that nothing can edit.
- Moving the pointer serialises per workflow (a row lock held for a handful of statements) and not across
  workflows; a benchmark in `tests/performance/reports/gitops-versioning.md` puts a number on it.
- A rollback is not subject to the approvals of the namespace. This is deliberate (an emergency needs the
  version that ran last week, which was approved when it was published), and it is also a way around the
  policy for anyone with `version:activate`: the permission is the control, and every use of it is a row of
  the history and an audit entry (G196).
- Versions published before a namespace asked for approvals are trusted as they are (G197).
- "Another person" is another subject who did not save the draft last: the platform does not know the author of the
  commit in Git, so for a namespace published from a pipeline review in Git is the control and this is a second one
  (G210).
- The history grows for ever (G194), and artifacts pinned by any version stay for ever (G193).

## Alternatives rejected

1. **Republish the old document** (1 above): rewrites the story and makes the answer to "what was running" a
   search.
2. **A flag on the draft** (2): outlives the edit that falsifies it.
3. **Approvals in the clients** (3): the API would be the way around them.
4. **A history the application could edit** (4): the record of having moved the pointer must not be in the
   hands of whatever moves it.
5. **A promotion that copies a definition as a draft** (5): the proof that the document is the same would be
   left to people.
6. **A policy per workflow or by pattern**: more expressive and not asked for; one number per namespace is
   explainable and has a default that is safe (G200).
