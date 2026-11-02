# ADR-0064: Access grants only add; only a restricted folder or an operation list narrows them; one evaluator decides every operation

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** supersedes none. Refines: ADR-0031 (per-name audience left to "another decision";
its operation permissions may also be held by explicit grant and through the `runner` folder level, while
its disclosure, receipt and audit rules are unchanged); ADR-0028, ADR-0035 and ADR-0018 (their permissions
may be held by explicit custom grant); ADR-0036 item 5 (the catalog grows to 53 permissions with this feature and further with other drafts;
built-in roles are additive). Cites ADR-0050 (feature 018) for the merged admission lock order, in which
this decision's access-capability check is slot 1. Confirms ADR-0033 (its permissions are reserved; audiences unchanged). Revisits the accepted
feature-009 gap G39 and the role-fixed grants of FR-003-005 (`saga:override`), FR-014-007 (the
`approval:decide` gate; eligibility unchanged) and 009 D52 (`quota:manage`, still bounded by operator
ceilings). G49 (`worker:bind`) stays open: `worker:bind` is reserved in this increment.
**Enforced by:** planned in feature 029 and not existing yet: property tests of
`sagat_domain::access::evaluate` (determinism, monotonicity, reserved exclusion, built-in floor, developer
bound, kind check, listing equivalence, no oracle); `tests/contract/custom-roles-folders/coverage.rs` (every operation declares `x-access`
and `discloses`); the per-kind opacity battery `opacity.rs` and `canary_scan.rs`; the escalation matrix
`tests/integration/custom-roles-folders/escalation.rs`; `lock_order.rs` and the race suite (fences); the
human-role trigger tests (machine principals); the `scripts/check-architecture.py` rule against role-only
decisions; the older-replica test of the operator activation of `access-narrowing-v1` through feature 018's
instance-capability registry and of the refusal of narrowing writes before it (T018-080, T029-003); the
cross-tenant battery and the runtime-privilege sweep.

## Context

A person's access is one built-in role per tenant, the same in every namespace; the application decides by
that role at well over 154 places (`Principal::authorize`, direct `Role` reads in domain, application and
store). Narrower control lives in resource lists (worker bindings, secret access lists and pools, the PII
reveal grant list). Feature 009 accepted "no custom roles" (G39) and recorded that separating the power to
bind workers from reading secrets needs them (G49). ADR-0031 grants `run:query`/`run:update` for all
declared operations of a namespace. Comparable platforms offer custom roles (Temporal Cloud: union with a
mandatory base role, no deny), groups (local and SCIM) and folders with levels for users and groups
(Windmill). Feature 029 specifies these and needs one rule for how they combine, because the combination is
what a reader must be able to predict, an auditor explain, and a reviewer check for escalation.

Constraints: authorization fails closed; a foreign or hidden resource is indistinguishable from a missing
one; revocation applies on the next admitted call (009 D51: nothing cached); audit commits with the change;
machine identities keep bindings; the domain stays pure; older binaries keep serving during a rolling
upgrade (the control-plane runbook upgrades the schema first).

## Decision

1. Effective access MUST be computed by one pure domain function of durable facts: the built-in role, the
   subject's local groups, custom-role assignments at tenant scope and in the namespace, the item's
   placement, folder mode and entries, and the operation list of a declared operation name. Its result
   MUST be independent of input order and MUST carry its sources.
2. This decision MUST NOT remove or narrow any built-in role or grant; built-in roles are additive (other
   features add machine roles and grants, honoured as they are; only `admin`, `developer`, `operator` and
   `viewer` are human, every other role is a machine role and never a principal; tests assert set properties,
   not counts). Custom roles and folder levels MUST only add. The only narrowing
   MUST be (a) a folder in `restricted` mode, inside which level permissions come only from that folder's
   entries, and (b) an operation list, which must also list the caller. There MUST be no other deny rule.
3. Reserved permissions (`worker:execute`, `script:build`, `script:execute`, `database:execute`,
   `payload:read-internal`, `payload:reveal-pii`, `membership:manage`, `group:manage`, and in this increment
   `worker:bind` and `secret:manage`) MUST NOT be obtainable through a custom role, folder level or
   operation list; a permission added later is reserved until classified (a compile-time default, not a
   compile error). Machine identities MUST NOT be principals of any custom grant.
4. Every other built-in grant is a default, not a ceiling: it MAY also be held through an explicit, audited
   custom grant. Folder levels MUST be bounded by the built-in `developer` role and apply only to their item
   kinds; `run:operate`, `run:signal`, `version:activate`, `database:manage` and the disclosure, override,
   approval-decision and ad hoc/write/result permissions are never conferred by a level. The bypass of
   secret access lists MUST stay tied to the built-in `admin` role. `quota:manage`, `egress:manage` and
   `audit:read` are administrator-equivalent: granting them MUST require an explicit confirmation, and
   `audit:read` MUST be documented as seeing past restriction.
5. A grant MUST be refused unless the actor, evaluated on committed state inside the writing transaction
   after the access fence, administers its scope and holds every permission it confers wherever it applies.
   Moving an item into a folder, widening an operation list and adding a member to a group are grants; a
   member addition MUST be audited as a widening of every list and restricted entry naming the group. An
   assignment MUST NOT depend on its granter after commit.
6. A decision MUST keep today's order: refuse before any lookup when the caller could not hold the
   permission on that kind (ceiling or any of their levels), then answer missing and hidden targets alike.
   Every REST operation MUST declare its access class and the item kinds its answers disclose (`x-access`);
   listings MUST use the evaluator's visible-folder set, never a separate SQL rule. Narrowing MUST be enabled
   per item kind only after the opacity battery (as every built-in human role) and a canary scan prove that
   kind never leaks (bodies, headers, rate effects, name-keyed writes, key replays), because a route that
   still consults only the built-in role would fail open.
7. Narrowing MUST be enabled only when every serving binary honours it, through feature 018's
   instance-capability registry (registration before serving, heartbeat, re-check on every API admission
   and stream re-evaluation), deployed fleet-wide in a release before this feature. Activating
   `access-narrowing-v1` MUST be an audited operator act of a deployment administrator through 018's
   registry command, refused while a live control-plane instance lacks it; it MUST NOT be a side effect of a
   tenant request. Until it is recorded, restricted folders and operation lists MUST be refused. A
   readiness probe alone is not sufficient (it leaves a window, and Compose does not route by readiness).
8. Store-side rechecks of sensitive and item decisions MUST use the same evaluator behind shared tenant and
   namespace access fences that access writers take exclusively, in ADR-0050 decision 7's slots: the
   capability check is a read in slot 1; memberships → access fences → namespace row are slot 4a, after the
   receipt, aggregates and existing runs; with a bounded writer `lock_timeout`; an
   access writer MUST NOT wait on a non-access row while holding a fence. No decision MAY be cached across
   requests unless the cache is versioned by every access change.
9. Actions on behalf of a recorded subject (018 schedule actions, 026 links) MUST be rechecked through the
   evaluator at fire or submit time; only admissions that act for no recorded subject are exempt from
   folders. 020 list queries and saved views, 021 resources and 025 MCP listing and calls use the same
   evaluator when this feature is enabled.
10. Built-in `admin` MUST NOT bypass a restricted folder silently; a `folder:manage` holder can always
   administer a folder's entries (judged against their namespace ceiling), so no folder can be orphaned, and
   putting oneself on the list is an audited change. Restriction governs the platform's API surfaces only;
   worker bindings and audit reads see through it, and the documentation MUST say so.

## Considered options

1. **Additive grants with explicit narrowing by restricted folders and operation lists (chosen).**
   Predictable (unions commute), explainable (sources), compatible (nothing changes until a tenant creates
   access objects), and folders gain the capability that motivates them: hiding work from namespace-wide
   roles.
2. **A policy language with allow and deny rules (Cedar/OPA style).** Expressive, but precedence makes
   decisions hard to predict and explain, every route needs attribute plumbing, and a deny rule against the
   built-in floor changes the meaning of every existing role. Rejected for this product stage (D1445).
3. **Additive-only folders (no restricted mode), like Temporal custom roles.** Cannot fail open, but cannot
   hide anything; the feature input asks for folder ACLs that control who sees and acts. Rejected.
4. **Folder lists that intersect with the namespace role everywhere (narrow-only folders).** Cannot express
   "this viewer may run the workflows of one folder" without a namespace-wide grant. Rejected.
5. **Folder levels mirroring built-in operator and admin powers.** Lets a folder owner hand out
   `database:manage` (which selects secrets by name) or `run:operate` to anyone. Rejected for item 4.
6. **Administrator bypass of folder lists (Windmill).** Avoids lockout but makes restriction invisible in
   the audit. Replaced by item 10.

## Consequences

- Good: one function to test and explain; tenants that define nothing see no change beyond an additive
  capability field; delegation is bounded by construction; G39 is answered. G49 is not: `worker:bind` and
  `secret:manage` stay reserved, and `audit:read` holders see past restriction.
- Bad: every role-only decision must move to the evaluator, guarded by a new architecture rule; new routes
  must declare `x-access`; per-request evaluation and fences add cost to be measured. A name collision can
  reveal that a hidden item's name is taken (G1441); aggregates count hidden items (G1442); timing is not
  equalized (G1457); a visible parent still shows a hidden child's outcome (G1456).
- Harder: lock ordering (fences, `NOWAIT`, inventory with barrier tests) and release ordering (018's
  instance-capability registry must be deployed first; binaries older than it cannot be detected or
  protected, G1459, 018 G1607). Each API admission reads the registry epoch (measured in the benchmark).
- Operational: rollback disables narrowing per kind; binary downgrade requires removing every restriction
  and list and then the requirement. Runs in flight are unaffected: no IR, protocol or transition change.

## Evidence

None yet: this is a proposal, revised after independent security and architecture reviews on 2026-10-26 and
an architecture re-review on 2026-10-27.
Analysis: `specs/029-custom-roles-folders/research.md` (baseline paths, Temporal and Windmill semantics with
URLs, D1440–D1479, G1440–G1461, review findings); the normative function, administration rules and vectors:
`specs/029-custom-roles-folders/contracts/evaluation.md`. Acceptance requires the owner's review; release
requires the checks under **Enforced by** to exist and pass.
