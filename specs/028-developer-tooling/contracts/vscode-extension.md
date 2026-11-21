# VS Code extension — commands, settings, trust, messages (draft)

Package: `frontend/apps/vscode-extension` (npm workspace, TypeScript). The extension is a client of the `sagat` CLI
only (D1420). Minimum CLI: the release that ships `--json` output schema v1 (`sagat version --json` reports
`outputSchemaVersion` and `supports`); an older CLI disables every command and says which version is needed.

## Activation and documents

- Activates on `workspaceContains:**/sagat.yaml` and on JSON/YAML documents whose top level has
  `apiVersion: sagat-octopus-orchestrator.io/v1` with `kind: Workflow` or `kind: Manifest`.
- Diagnostics: on save (and on type after 500 ms idle when `sagat.validateOnType` is true) the extension writes the
  unsaved buffer to a private temporary file (mode `0600`, removed after use) and runs
  `sagat workflow validate <tmp> --json` (or `sagat validate --json` for a manifest). Each finding's JSON pointer is
  mapped to the text range of the value it designates with a position-preserving JSON/YAML parser; an unmappable pointer
  lands on the document's first line. Code, help and docs link are kept (`Diagnostic.code` with the docs target).

## Commands

| Command id | Title | Trust | What it runs |
|---|---|---|---|
| `sagat.validate` | Sagat: Validate document | any | `sagat workflow validate <file> --json` |
| `sagat.previewGraph` | Sagat: Open graph preview | any | nothing (webview; graph model from `@sagat/workflow-graph`) |
| `sagat.signIn` | Sagat: Store token | trusted | stores a token in secret storage (input box, password mode) |
| `sagat.signOut` | Sagat: Forget token | any | deletes the stored token |
| `sagat.doctor` | Sagat: Check connection | trusted | `sagat doctor --json` |
| `sagat.publishAndRun` | Sagat: Publish to dev and run | trusted | `sagat workflow publish <file> --json`, then `sagat workflow run <name> --json` |
| `sagat.serveAnswers` | Sagat: Serve canned answers | trusted | `sagat worker dev --mocks <file> --workflow <file>` (a terminal the user sees) |
| `sagat.followRun` | Sagat: Follow run | trusted | `sagat workflow inspect <run> --follow --json` |
| `sagat.cancelRun` | Sagat: Cancel run | trusted | `sagat workflow cancel <run> --reason <text>` |
| `sagat.testServer.start` / `.stop` | Sagat: Start/stop test server | trusted | `sagat test-server start|stop --json` |
| `sagat.testServer.advance` | Sagat: Advance test clock | trusted | `sagat test-server advance --by <ISO duration> --json` |

Every command checks `workspace.isTrusted` in code, not only through `when` clauses.

## Settings

| Setting | Scope | Restricted in untrusted workspaces | Meaning |
|---|---|:-:|---|
| `sagat.cliPath` | machine | yes | absolute path of `sagat`; default: found on `PATH` |
| `sagat.server` | application (user or machine only) | yes | API URL (`https`, or `http` on loopback — the CLI enforces it) |
| `sagat.grpc` | application (user or machine only) | yes | worker endpoint |
| `sagat.namespace` | resource | yes | namespace slug (selects, never authorizes; the server checks access) |
| `sagat.allowNonDev` | application (user only) | yes | allow publish/run/answers outside `dev` namespaces (default false) |
| `sagat.validateOnType` | any | no | validate while typing |

Workspace-level values of `cliPath`, `server`, `grpc` and `allowNonDev` are ignored in every workspace, trusted or
not: a repository must not choose the binary that runs, the origin that receives the token, or widen the guard
(security review S11). The extension reads them with the user/machine-scoped configuration API and says in its output
channel when a workspace value was ignored.

## Credentials

The token lives in the extension's secret storage (`context.secrets`) under a key derived from the normalized origin
(scheme, host, port) of the server it was stored for. A command passes a token only when the configured `sagat.server`
has that origin; otherwise it asks the user to store a token for the new origin. The child process receives it as
`SAGAT_TOKEN` in its environment; it is never an argument, a setting, a file of the workspace, a webview message or a line of an
output channel. Child-process output is shown after the CLI's own control-character filtering (feature 012 D210). The
extension never logs the environment of a child process.

## Webview

- `enableScripts: true`, `localResourceRoots: [<extension>/media]`, no remote resource, no `retainContextWhenHidden`
  for runs with payloads (none are sent).
- CSP: `default-src 'none'; img-src ${webview.cspSource} data:; style-src ${webview.cspSource}; script-src 'nonce-<random>'; font-src ${webview.cspSource}`.
- Messages (validated against a closed schema on both sides; unknown messages are dropped):
  - host → webview: `{type: "graph", version: 1, nodes: [...], edges: [...], findingsByNode: {nodeId: count}}`;
    `{type: "runState", runId, nodes: {nodeId: status}}` (statuses only, no payload); `{type: "theme", kind}`.
  - webview → host: `{type: "ready"}`, `{type: "reveal", nodeId}` (selects the node's text range),
    `{type: "toggleTable"}`.
- The graph and its table alternative use the Studio's design tokens mapped to the editor theme; keyboard navigation
  and screen-reader labels as in the Studio (FR-007-008).

## Telemetry and logs

No telemetry of its own (D1423). An output channel logs commands run (name and exit code), never arguments that came
from documents, never tokens.

## Tests

Unit tests (pointer → range mapping, message schemas, settings precedence), integration tests in the editor's test host
(`@vscode/test-electron`) against a fake CLI that records argv and environment (token canary), and one end-to-end run
against a real `sagat` and test server in CI.
