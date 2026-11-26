# Targeted re-read of design 046 for approval: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cd82af9376ce4a8d40196f11541a3ca1ac754207
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; nothing of substance left out. The last targeted security re-read
after review round 3, on the approval rule of Owner decision 2026-11-25-2 (fix Blockers and every security finding,
re-read the fixes). Its history in this folder: `reread-1-security.md` (Major 1, Nit 5), `reread-2-security.md` (Nit 6),
`reread-3-security.md` (Nit 3), `reread-3b-security.md` (at `4b6a3e6d`, nothing open), then `reread-4-security.md` (Major 3, Nit 2) after
the architecture and correctness answers, and `reread-5-security.md` (Major 1); every finding fixed before the next pass.
The reviewer read the diff of the last fix, the design, the data model and the contracts, checked the lock behaviour
against PostgreSQL's documented and source behaviour, and ran nothing.

## The previous finding

Answered. Decisions take a shared advisory lock per ecosystem and the switch takes it exclusively; the two conflict, and a
new heavyweight request that conflicts with a waiting one queues behind it, so overlapping publications cannot get ahead
of a waiting switch (only re-takes of a lock the same transaction holds skip the queue, and they add no holder). Also
checked and clean: the lock order (a decision takes its rows, then the shared lock; the switch takes the exclusive lock,
the snapshot row, then block inserts and no version row, so no cycle); the lock lifetime (transaction-level, nothing leaks
across pooled connections); privilege (`sagat_app` no longer needs UPDATE on `package_advisory_snapshots`); no tenant
oracle (the key is fixed per ecosystem). One claim was wrong but opens no attack: advisory lock functions are executable
by every role, so the key does not belong to the function alone; only the trusted control plane could misuse it.

## New findings

1. Nit. ADR-0121 (decision 4 and decision 6) still says the re-evaluation page and the finish take the snapshot row
   `FOR SHARE` and that "their writers take them `FOR UPDATE`": as `sagat_app` that lock is refused, and granting UPDATE
   to make it work would make the switch starvable again. Fix: "the ecosystem's snapshot lock shared" and "the switch
   exclusively".

Counts: Blocker 0, Major 0, Nit 1

## The author's answer

Fixed in ADR-0121 decisions 4 and 6, and the data model's wording about the key corrected (a convention of trusted code,
not a privilege). Neither file is the design's text, so the reviewed commit stands.
