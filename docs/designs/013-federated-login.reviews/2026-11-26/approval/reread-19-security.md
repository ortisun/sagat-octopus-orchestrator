# Targeted security re-read of design 013 for approval, round 19 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 2dae56feaece2cb31c2f10b4f660020440a98580
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-18 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the newtypes, the hand-written ring `Debug` and the rule consistent across Technical Plans 2 and 12, Task 013-8 and Architecture decision 0140.
Counts: Blocker 0, Major 0, Nit 2

1. **Nit. Technical Plan 2; Technical Plan 12; Task 013-8; Architecture decision 0140.** A rule on signatures naming key types passes key material as plain bytes (`pub(crate) fn material(&self, id) -> Option<&[u8]>`, or a `to_document` that `rotate` and `revoke` might reuse), misses `pub(in path)`, and does not say whether submodules are scanned. Fix: an allowlist of the module's non-private items (`load`, `mint`, `verify`, `seal`, `open`, key-id and state accessors, `Debug`, the purpose markers) over the module and its submodules; that negative fixture; the commands rewrite the document from their own read of OpenBao.
2. **Nit. Technical Plan 12; Task 013-8.** The newtypes are not barred from a derived `Debug`, and the loader's document type holds `material` as text, so logging a refused document or entry prints a key while the test, which formats only the ring and adapters, passes. Fix: no derived `Debug` on the newtypes; decode `material` straight into `RingKey`; extend the test to a refused document's error and log line.
