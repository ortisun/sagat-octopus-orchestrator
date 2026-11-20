# Validation guide: custom roles, groups, folders and operation lists

Nothing here runs today: the feature is not implemented. This is the walkthrough the implementation must
make executable (T029-068, `cargo test -p sagat-tests --test e2e_custom_roles_folders quickstart::`), as
feature 016 did for its quickstart. Until a step has recorded evidence it is a target, not a result.
Commands under `sagat access` are new (FR-029-032); `sagat rollback`, `sagat workflow run|runs|inspect`
and `sagat dev token` exist today (`apps/cli/src/args.rs`); everything else uses the REST API.

## Specification checks (runnable now)

```sh
git diff --check
ruby -ryaml -e 'YAML.load_file("specs/029-custom-roles-folders/contracts/access.openapi.yaml")'
python3 -c "import json; json.load(open('specs/029-custom-roles-folders/contracts/permission-catalog.json'))"
```

They prove the documents are well formed, not that the design works.

## Prerequisites (target)

A stack as in `docs/quickstarts/README.md` (Compose with PostgreSQL, control plane, engine, development
identity provider), tenant `acme`, namespaces `payments` and `orders` (the workflow `invoice-run`
published in both), and members with these built-in roles: `admin-1` (admin), `dev-1` and `dev-2`
(developer), `op-1` (operator), `view-1` (viewer). There is no supported member-creation surface (G290,
O1442): the e2e suite inserts members through `tests/support`; a manual run inserts `memberships` rows as
the database owner in a development database only. Tokens: `sagat dev token --subject <SUBJECT>`. The
`worker dev` helper (feature 012) serves `invoice-run`, `refund-run` and `payroll-run` with canned answers; `payroll-run`
declares the Update `approveOverride` and the Query `status`. Synthetic data only.

```sh
export SAGAT_SERVER=http://localhost:8080 SAGAT_NAMESPACE=payments
api() { curl -sS -H "Authorization: Bearer $TOKEN" -H "X-Octopus-Namespace: $SAGAT_NAMESPACE" \
             -H 'Content-Type: application/json' "$SAGAT_SERVER/v1$@"; }
```

## Happy path (as `admin-1` unless stated)

```sh
# 1. A custom role and a namespace assignment (US01)
sagat access role create release-manager --scope namespace:payments \
  --permission definition:read --permission version:activate --permission run:read --permission run:operate
sagat access assign release-manager --subject view-1 --scope namespace:payments
# as view-1: allowed in payments, refused in orders
sagat rollback --workflow invoice-run --reason quickstart --to 1                  # activated
SAGAT_NAMESPACE=orders sagat rollback --workflow invoice-run --reason quickstart --to 1   # 403 FORBIDDEN

# 2. A local group (US02)
sagat access group create payments-oncall
sagat access group add-member payments-oncall op-1 dev-2
sagat access assign release-manager --group payments-oncall --scope namespace:payments

# 3. Folders (US03): an open one with a widening entry, a restricted one with preview
sagat access folder create billing
sagat access folder create payroll
sagat access move workflow invoice-run --to billing
sagat access move workflow payroll-run --to payroll
sagat access folder set-entry billing --subject view-1 --level runner
sagat access folder set-entry payroll --group payments-oncall --level editor
sagat access folder preview payroll --restrict        # counts; prints folderRevision R
sagat access folder restrict payroll --expected-revision R
# as view-1: can now start invoice-run (runner on the open folder billing)
sagat workflow run invoice-run --input '{"invoice":"syn-1"}'

# 4. An operation list (US05)
sagat access operation-list set payroll-run update/approveOverride --group payments-oncall

# 5. Explanation (US06)
sagat access explain --subject view-1 --permission run:start --workflow invoice-run
# → allowed; sources: folder level runner on billing (entry granted by admin-1)
```

Expected audit rows (`api /audit`): `role.created`, `role.assigned` ×2, `group.created`,
`group.member_added` ×2, `folder.created` ×2, `item.placed` ×2, `folder.entry_set` ×2, `folder.changed`
(mode), `operation_list.created`; each committed with its change, none containing a name or description.
Step 3's restriction succeeds only after a deployment administrator has activated `access-narrowing-v1`
through feature 018's instance-capability registry (its audited operator command; refused while any live
control-plane instance lacks the capability) and the workflow and run kinds are enabled in `api /version`
(`accessControl.restrictedFolders`); before that it answers `422` `ACCESS_CAPABILITY_DISABLED`.

## Failure scenarios

```sh
# as dev-1 (developer, not on payroll's list): the restricted workflow does not exist for them
api "/workflows?limit=200"                 # payroll-run absent from every page
api "/workflows/$PAYROLL_WORKFLOW_ID"      # 404 NOT_FOUND, byte-identical to a random uuid
sagat workflow runs payroll-run            # the same not-found answer as for an unknown workflow
api /workflows -X POST -H 'Idempotency-Key: qs-0001' -d '{"name":"payroll-run","draft":{}}'
                                           # 409 Conflict, the same body as any name collision (G1441);
                                           # payroll-run's draft and revision are unchanged

# as admin-1, also not on payroll's list
api "/workflows/$PAYROLL_WORKFLOW_ID"      # 404 until admin-1 adds themselves (audited)

# escalation: dev-1 holds a delegated role {role:assign} in payments and tries to hand out run:operate
sagat access assign release-manager --subject dev-2 --scope namespace:payments
                                           # 403 FORBIDDEN PRIVILEGE_ESCALATION; no row; access.denied.escalation

# a move is a grant: after admin-1 assigns view-1 a namespace role {folder:manage, definition:write},
# view-1 moves the root workflow refund-run into billing, whose entries confer runner
sagat access move workflow refund-run --to billing  # 403 PRIVILEGE_ESCALATION (lacks the runner items)

# widening an operation list needs role:assign: dev-2 (definition:write only) adds themselves
sagat access operation-list add payroll-run update/approveOverride --subject dev-2
                                           # 403 PRIVILEGE_ESCALATION; removing a principal would succeed

# operation list: op-1 leaves payments-oncall, then replays an Update key used before
api "/runs/$RUN/updates/approveOverride" -X POST -d '{"updateKey":"k-1","args":{}}'
                                           # 403 OPERATION_NOT_GRANTED; no receipt replay
api "/runs/$RUN/queries/status" -X POST -d '{"args":{}}'    # still allowed (no list on that name)

# reserved permission and in-use deletes
sagat access role create sneaky --scope tenant --permission payload:reveal-pii
                                           # 422 VALIDATION_FAILED RESERVED_PERMISSION
sagat access role delete release-manager   # 409 CONFLICT ROLE_IN_USE (lists assignments)
sagat access folder delete payroll         # 409 CONFLICT FOLDER_NOT_EMPTY
```

Revocation on the next call: remove `view-1`'s assignment, then repeat the first rollback as `view-1`; the
first request admitted after the removal commits is `403`. A run stream (`api "/runs/$RUN/stream"`) opened
by `dev-2` on a `payroll-run` run ends within 30 seconds after `dev-2` leaves `payments-oncall`
(SC-029-008), and resuming it is `404`.

## What this walkthrough does not prove

Fault injection, the opacity battery over every operation, the escalation matrix, the five-SDK scenario,
the external group source and the benchmark have their own suites (tasks.md). A passing walkthrough is one
happy path and a handful of refusals, not the guarantees of the specification.
