# Version1 pure successful leaf collection shape

Basefbf66a0f. Three core files: workflow export, new pure workflow/collection_shape.rs and domain
integration test. No dependency or existing execution behavior change. Commands use locked cargo,
CARGO_INCREMENTAL=0,CARGO_BUILD_JOBS=3,RUST_MIN_STACK=16777216; warm target optional. No database,
OpenBao, S3, worker/scheduler or SDK client is executed by this unit.

Test-first missing API fails compilation (not behavioral RED). First implementation7/8; invalid
short-slug fixture prevented its managed prefix case from construction. Independent review found
that managed DB+collection is already domain-refused, so the unreachable predicate case was
removed without claiming DB predicate test coverage. Final8/8,4.30s command; strict domain/alltarget
Clippy10.18s, fmt4.11s, architecture24crates. Two bounded reviews; final no findings.

Tests cover1/8 ordered activities,0/9 refusal, companion/reconcile/script/effect/budget metadata,
ordinary versus lowered script source, wrong/nested owner, timer/error bodies, root work after
collection and shared local IDs with another body's independent compensation declaration.
The public checked result contains only qualified owner/body/activity/return IDs and profile
version1, deliberately no versionID/run/settled flag. Matching persisted version/source facts,
immutable closure storage, actual producer/restart proofs and scoped Finish remain open.
Classifier work depends on bounded static definition size, not runtime sibling/history count;
this is not actual SQL-bound/100k/resource evidence. No publication or nested activation.
