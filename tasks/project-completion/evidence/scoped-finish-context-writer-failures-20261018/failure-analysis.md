# Actual failure and repairs

First full25 run:22passed/3failed, exit101,92.87s wall/56.83s harness. All1100 input
hashes match before/after that failed run; retained failure-native-family and failure-source files.
No product implementation changed. Owner-only diagnostics show root kid steps WAITING/CHILD,
child_started=false and no child Run. Flow's scheduler omitted with_children; configuring the
same already placed scheduler enables actual child creation/settlement. The historical root
handoff test already passed and did not need this change.

Audit/outbox/receipt isolated worlds completed; deferred_commit world failed workflow creation422
before injection because underscores violate the public workflow name. Keep fixed enum SQL names,
replace underscore only in the workflow slug. Final atomic world reaches the deferred trigger as
proved by the owner-only nontransactional marker and returns confirmed Storage23514, not unknown.

Independent reviews also strengthened post-lease current Run CAS, capability revocation before
retry and actual fresh header/current authority discovery after confirmed stale refusal. These are
reflected in the final passing tree; failure sources remain separately frozen. No retry erased the
initial failure, and no aggregate combines partial passes.
