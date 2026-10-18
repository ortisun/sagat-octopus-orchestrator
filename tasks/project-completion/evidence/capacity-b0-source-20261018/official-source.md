# Mode classification source

Consulted2026-10-18: [PostgreSQL16 explicit locking, row locks](https://www.postgresql.org/docs/16/explicit-locking.html#LOCKING-ROWS).
Eligible nonpartial unique-key updates take UPDATE-strength row protection; other updates
use NO KEY UPDATE. The latter is compatible with KEY SHARE, while UPDATE is not.
Applied against actual migration indexes, not merely the presence/absence of explicit SELECT
FOR UPDATE. This corrects the earlier approval cancellation request NK suggestion.
No inference from genericUnavailable to detecteddeadlock. Actualbarriers remain separate.
