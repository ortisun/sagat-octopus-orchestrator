// Package sagat is the shared vocabulary of the Go SDK of the Sagat Octopus Orchestrator: the
// canonical error every call returns, the token source, the retry policy and the W3C trace context
// helpers. The SDK itself is three packages built on it:
//
//   - client: start, read, list, cancel and signal runs and publish events over the REST API;
//   - worker: a host that runs activities, compensations and reconcilers for the worker protocol
//     (gRPC);
//   - workflow: side-effect-free builders of the workflow IR the server validates and versions.
//
// An SDK is not a workflow runtime: it does not execute workflow code, replay anything or decide a
// transition. The server owns every state and every retry decision (sdks/SPEC.md).
//
// The import path ends in "/go" but the package is named sagat, so import it with a name:
//
//	import sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
package sagat
