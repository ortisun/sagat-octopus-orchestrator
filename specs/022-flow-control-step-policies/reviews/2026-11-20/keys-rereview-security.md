# Security re-review of the follow-up (`295b158b..40854f4d`)

Reviewer: `security-reviewer` (read-only), 2026-11-20. Ran nothing. (Condensed.)

**Blocker: none. Major: none.** Every earlier Blocker and Major is resolved in code: the instants of a key (operations.rs,
asserted in `rest::a_key_label…`), the cursor bound (`keyed_digest.rs`), the single reveal rule (`reveal_grants.rs`, the
fitness rule), the real-engine start test, the G1199/T022-104/D022-037 records, the pinned digest, `api.md`, the Studio
types and the target selection.

minor: m1 `GateKey` (`crates/application/src/flow_control.rs:262-270`) derives `Debug` over its digest (hygiene, no
current log); m2 the compose/runbook recipe `chgrp 10001` may name the wrong group (the Dockerfile's `useradd --uid 10001`
creates a system-range group; verify with `id`, or `chown 10001` + 0400); m3 a force-retired (leaked) version left in a key
file can still open forged cursors and bisect a digest: remove it from the files at once, or open cursors only under the
versions in force; m4 G022-014 understates the residual channel (oldest-wait order with one waiter per key; a caller who
starts runs confirms a guess through its own run), T022-104 is the real control.

nit: the ring is still parsed and wired with every switch off (the serving gate stops it); the zeroize claim is broader
than `decode_key`; the `group` patterns disagree with `Slug` (3..63); the contract's "such a caller" reads backwards; the
longest-position test uses a 16-digit instant.

Checked with no finding: the active version read in the page's snapshot (`sagat_app` has `SELECT`, one active row by a
unique index, 503 without it), the size bounds, the one reveal rule's semantics and binding, the hidden instants, the
startup check's texts, tenant boundary, injection.
