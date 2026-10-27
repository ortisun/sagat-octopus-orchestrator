# Decisions — Rust SDK

D034-001: owner requests Rust during the active0.3 delivery. Rust becomes the sixth official SDK
under feature006's unchanged semantic contract; Redis033 and remaining014 work are preserved.
D034-002: Tokio asynchronous client/worker plus declarative workflow builders, not server-internal
bindings. The public package contains no database scheduler, Redis/NATS client or workflow runtime.
D034-003: the protocol and error catalog are copied canonical wire artifacts with drift tests;
packaging includes them so clean consumers do not depend on this repository's directory layout.
D034-004: preserve historical five-language evidence and add actual Rust/six-language results;
never fabricate parity by copying golden output into the Rust runner.

D034-005: normal workers offer 1.5. Explicit protocol overrides remain available for the common
negotiation diagnostic, including offers the server refuses. Accepted versions are parsed strictly,
never exceed the offer, and never exceed canonical 1.6 (documented by the server as a superset of
ordinary 1.5 execution). Managed database leases remain restricted to the dedicated database-runner
identity; this SDK does not expose a managed database executor. Unknown future accepted minors fail.
D034-006: nonretryable receive failures stop discovery and are observable through
WorkerHost.wait_for_failure(); existing leased handlers keep their immutable session and continue.
Shutdown uses the supplied grace without a hidden extra drain interval. Retained user handlers are
cooperative; shared concurrency permits remain charged across restart until their lifecycle ends.
