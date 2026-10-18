# Publication diagnostic review

Three bounded fresh-context read-only GPT-6 Astra/xhigh cycles; no external CLI, following the
owner's selection. Reviewers traced source and did not run tests. The root ran the recorded tests.

1. Mapping every UnknownTarget to INVALID_SCOPE loses ordinary root compatibility. Actionable:
   retain named-body provenance at the IR boundary with EscapingBodyEdge, keep root GRAPH and
   the existing dry-run UNKNOWN_TARGET code, and cover both public operations. The initial body
   end oracle was wrong: canonical schema rejection precedes graph checks and stays SCHEMA.
2. InvalidScope also represented effective expression byte limits. Actionable: a quoted600-é
   expression is schema-valid but1202 bytes. The new HTTP test reproduced INVALID_SCOPE instead
   of EXPRESSION. Introduce typed ExpressionByteLimit in the domain, including switch predicates;
   map the typed cause to EXPRESSION without inspecting messages. Preserve the1024-byte bound.
3. The legacy Expression wrapper rejects above2048 bytes before the domain's effective bound.
   Actionable: a quoted700-€ expression has702 characters/2102 bytes. The new public regression
   reproduced SCHEMA instead of EXPRESSION. Check effective expression bytes at the typed IR
   node conversion boundary before constructing the legacy wrapper. Ordinary legacy-node
   conversion retains its existing behavior; structured nodes and switch predicates use1024.

The root surfaced the third-cycle finding and bounded stop to the owner, reconciled it with a
failing public test and fixed that concrete path. No fourth review occurred; the final adapter
precheck was verified by the recorded tests, not by an additional independent review verdict.
The final HTTP suite covers schema precedence, root compatibility, scope failures, unsupported
object literals,1202-byte body/switch expressions and the2102-byte wrapper counterexample.
