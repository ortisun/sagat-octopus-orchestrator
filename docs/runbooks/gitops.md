# GitOps, versions, approvals, promotion and rollback

For the people who publish workflows and the people who answer for what runs in production. A **version** is a
workflow document as published: it never changes. What moves is the **pointer** to the version new runs use, and
the right to publish into a namespace. Runs already started keep the version they started on, whatever moves.
Design: [ADR-0018](../adr/ADR-0018-publishing-is-one-transaction-the-pointer-has-a-history-and-an-approval-is-of-a-document.md).
Evidence, reviews and what is not built: `specs/011-gitops-versioning/quickstart.md` and `research.md` (D183–D198, G193–G208).

## The manifest

A file kept in Git next to the documents it lists (JSON or YAML; the schema is
`specs/011-gitops-versioning/contracts/manifest.schema.json`):

```yaml
apiVersion: sagat-octopus-orchestrator.io/v1
kind: Manifest
namespace: billing-dev          # the namespace the workflows belong to
workflows:
  - name: checkout              # must be the `metadata.name` of the document
    file: workflows/checkout.json   # relative, below this file; links are followed and checked
    releaseNotes: First release of the checkout flow.   # optional, up to 2048 bytes
```

Unknown fields are refused. At most 500 workflows; a file larger than 1 MiB is refused before it is read; a name
or a file listed twice is refused. The manifest says what a namespace **has**; it does not delete: a workflow the
manifest does not list is left alone.

## The command line

`cargo build -p sagat-cli --release` builds `target/release/sagat` (not packaged yet: G203). It speaks only the
public REST API.

```sh
export SAGAT_SERVER=https://sagat.example.com/v1     # or --server; plain http only for localhost
export SAGAT_TOKEN=…                                  # or --token-file path; never an argument
sagat validate                                        # the manifest, every document, and the platform's secrets/scripts
sagat diff [--check]                                  # what publish would change; --check exits 5 when anything would
sagat publish [--dry-run] [--accept-breaking]
sagat export --out ./git --namespace billing-dev      # a namespace built in the Studio, brought under Git
sagat promote  --workflow checkout --to billing-staging --reason "release train 42" [--version 3] [--accept-breaking]
sagat rollback --workflow checkout --reason "3 misbehaves" [--to 2] [--accept-breaking]
sagat history  --workflow checkout [--limit 20]
sagat requests --workflow checkout [--state PENDING]
sagat approve  --request <id> --reason "reviewed the document in Git"
sagat reject   --request <id> --reason "not this week"
```

`--namespace` (or `SAGAT_NAMESPACE`) names the namespace for the commands that do not read the manifest;
`-f` names the manifest (default `sagat.yaml`).

| Exit | Meaning | What a pipeline does |
|---:|---|---|
| 0 | Done (or nothing to do) | continue |
| 1 | Failed; the message says what, with the platform's own code, reason, remediation and trace id | stop |
| 2 | The command line is wrong | stop |
| 3 | Nothing failed, but something waits for approvals before it takes effect | wait for the approval, run the **same command** again |
| 4 | Refused as breaking: callers or emitters of the workflow may fail | read the findings; if intended, run again with `--accept-breaking` |
| 5 | `diff --check` found differences | fail the check |

Everything is safe to run again: creation and publication carry a key derived from the document, so a run that was
interrupted or whose answer was lost is completed by running it again, and a request that has its approvals is
published by running `publish` (or `promote`) again.

## Who may do what

| Action | Permission | Roles |
|---|---|---|
| Create a draft, edit it | `definition:write` | developer, operator, admin |
| Publish, open/withdraw/publish a request | `definition:publish` | developer, operator, admin |
| Approve or reject a request | `publication:approve` | developer, operator, admin (**never the author of the request**) |
| Set a namespace's approval policy | `publication:manage` | admin |
| Move the pointer (rollback, the way forward again) | `version:activate` | operator, admin |
| Promote | `definition:publish` and `definition:write` | developer, operator, admin |
| Read policy, requests, history | `definition:read` | everyone with a role |

## Approvals

A namespace has a policy: how many people **other than the author** must approve a publication into it. The default
is **one in production and none anywhere else**; an administrator sets 0 to 3 (`PUT /namespaces/{id}/publication-policy`,
with a reason, audited).

- In a namespace that asks, `POST /workflows/{id}/publish` answers `409 APPROVAL_REQUIRED`. Open a request for the
  draft (`sagat publish` does it), have the people approve **that document** (`sagat approve`), and publish the request.
- A request names the draft revision and the digest of the document. Editing the draft supersedes it: the
  approvers' decisions were about a document that is gone. One rejection ends it; the author may withdraw it; there is
  one open request per workflow.
- The approvals are counted again when the request is published, against the policy in force then. If an
  administrator raised the policy after the approvals, `NOT_ENOUGH_APPROVALS` asks for the extra ones.
- A person decides once. Two accounts of one human are two people (G207).

## Promotion

`sagat promote` takes the document of a version, byte for byte, and publishes it into another namespace of the
same environment or a later one (dev, staging, prod), as the workflow of the same name. The target's policy decides:
a namespace that does not ask publishes at once (`PUBLISHED`), one that does gets a request bound to that document
(`REQUESTED`, exit 3); if it runs that document already, nothing happens (`ALREADY_ACTIVE`). It stops at:

| Reason | Meaning | What to do |
|---|---|---|
| `DRAFT_HAS_CHANGES` | the workflow in the target has a draft with changes nobody published | publish or discard them there, then promote again; it is never overwritten |
| `BREAKING_CHANGE` | the document breaks callers or emitters of what the target runs now | read the codes; `--accept-breaking` if intended |
| a secret | the document names a secret the target does not have | create it in the target first |
| `UNKNOWN_SCRIPT` | the document pins a script artifact the target tenant does not have | build it first (G199) |
| `REQUEST_OPEN` | a request is open for this workflow in the target | the tool publishes it if it has its approvals, and says so if it has not |

The history of the target says where each version came from (`sagat history`).

## Rolling back

`sagat rollback --workflow X --reason …` moves the pointer to the version before the current one (the one that was
active before the last change; `--to N` names another). It adds a row to the history and changes **nothing else**:
the versions are as they were, runs in flight keep the version they started with, the event triggers become those of
the version the pointer holds. It is refused as `ACTIVE_VERSION_CHANGED` if somebody moved the pointer while you
were deciding, and as `BREAKING_CHANGE` if going there would break callers or emitters.

**Git still holds the document you came from.** The next `sagat publish` of that manifest would activate it again
(`diff` says so: "this is the document of v3, published before"). A rollback is the emergency; the fix is the
revert in Git. A rollback is not subject to the approvals of the namespace (G196): the permission and the history are
the control.

## Reading what happened

- `sagat history --workflow X` (`GET /workflows/{id}/activations`): every publication, promotion, rollback and
  activation, newest first, with who, why, the request, and where a promotion came from. The database refuses to
  change or remove a row. The newest row is the active version.
- `sagat requests --workflow X`: the requests, with each person's decision and reason.
- The audit trail (`GET /audit`, administrators): `publication.policy_changed`, `publication.requested`,
  `publication.approved`, `publication.rejected`, `publication.withdrawn`, `workflow.activated`, plus the publication
  entries that already existed. Reasons are kept whole.

## Metrics

On the control plane's `/metrics` (labels are closed sets; nobody, no workflow and no request is a label):

| Metric | Labels | Counts |
|---|---|---|
| `publication_refusals_total` | `reason` (`APPROVAL_REQUIRED`, `BREAKING_CHANGE`, `ACTIVE_VERSION_CHANGED`, `REQUEST_OPEN`, `DRAFT_HAS_CHANGES`, `REQUEST_STATE`, `ALREADY_DECIDED`, `NOT_ENOUGH_APPROVALS`, `AUTHOR_CANNOT_DECIDE`, `NOT_THE_AUTHOR`) | refusals of the rules above |
| `publication_events_total` | `event` (`requested`, `approved`, `rejected`, `withdrawn`, `published`) | what happened to requests |
| `version_activations_total` | `kind` (`rollback`, `activate`) | moves of the pointer that changed it |
| `promotions_total` | `outcome` (`published`, `requested`, `already_active`) | promotions |

A burst of `AUTHOR_CANNOT_DECIDE` or `REQUEST_STATE` is people not finding their request; a burst of `BREAKING_CHANGE`
is a pipeline that needs `--accept-breaking` or a fix in the document; `ACTIVE_VERSION_CHANGED` is two operators.

## Running it safely

- The token comes from a file or the environment and is never printed or put in an argument. It is sent only to the
  address you gave, redirects are not followed, and plain `http` is refused unless the host is this machine.
- A manifest is data from a repository: a file leading outside its directory (a link included) is refused, a file
  larger than the platform takes is not read, the document must be the workflow the manifest says it is.
- Output lines that come from the platform are its words (code, reason, remediation, trace id); quote the trace id
  when you ask for help.

## What is not here

No retention of artifacts or of the history (G193, G194), no Studio screens (G198), no scripts in the manifest (G199),
no policy by workflow (G200), no packaged binary (G203), no token exchange (G204). See `research.md`.
