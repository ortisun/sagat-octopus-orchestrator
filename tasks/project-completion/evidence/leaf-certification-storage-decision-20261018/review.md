# Dormant source storage decision review

Claim: the bounded decision makes lifetime/identity/privilege/implicit-FK/audit edges explicit,
without asserting source UPDATE closure or accepting an adapter/runtime.
Fresh Astra/xhigh cycle1: valid/actionable. Ordinary run FK checks can be disabled with privileged
session_replication_role=replica, contradicting the stated original-run retention boundary.
Added unconditional ALWAYS run DELETE/TRUNCATE retention and an ALWAYS id-change guard; existing
ALWAYS admission guard covers tenant/namespace/workflow/version. Root also identified id UPDATE
as necessary to prevent key escape without DELETE. PostgreSQL16 official docs verified FK bypass:
https://www.postgresql.org/docs/16/runtime-config-client.html#GUC-SESSION-REPLICATION-ROLE
Fresh Astra/xhigh cycle2: no concrete findings in bounded storage decision and source inventory.
No tests executed by reviewers. No SQL/producer/adapter/nesting acceptance follows.
Sol/high supplied read-only schema/retention design and actual producer inventory. Root retained
lossless scalar depth1 owner/index representation (no duplicate identity), dedicated fact resource
vocabulary and exact namespace/run FK. Schema-only runtime is SELECT until writer proof.
The reservation consumer accepts CHECKPOINTED with positive remainder; source predicate must
explicitly prove zero. Current addressed step UPDATE preservation remains reducer/producer proof.
User keeps current review; no external CLI. Stop at no findings; root sole writer/test executor.
