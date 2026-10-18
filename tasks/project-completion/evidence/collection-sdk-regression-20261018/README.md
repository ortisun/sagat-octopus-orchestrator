# Five-SDK collection regression — 2026-10-18

On e51b3390, the existing collection-runtime scenario passed in TypeScript, Go, Python, C# and Java against the current real API/scheduler/PostgreSQL stack. The one aggregate test passed in 114.62 s including 1m18s compilation; harness 33.74 s.

This is one selected scenario using existing cached SDK runners (`SAGAT_SDK_REUSE=1`), no language filter, no blessing and no archived clients. It is not a fresh SDK build or a repeat of the complete 60 suite. Explicit runtime-role mode was enabled. The complete command is in collection-runtime.json; selection/environment details are in environment.json.

All 1122 native and 582 tracked SDK/scenario inputs matched before/after. Five new normalized files were selected by execution timestamp, copied with hashes, and independently compared using canonical JSON serialization against the existing reviewed golden. This comparison preserves boolean/numeric type distinctions. Raw config files and credentials were not copied.

The scenario checks normal public collection execution and scoped worker identities. Its structure-only result projection does not establish result-value disclosure; separate native collection End/classification tests cover that claim. No golden was changed. Pending current-tree complete-suite, nested/C0/resource and release gates remain separate.
