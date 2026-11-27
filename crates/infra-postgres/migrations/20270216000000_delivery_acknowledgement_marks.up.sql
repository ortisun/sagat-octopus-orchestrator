-- Feature 006, delivery acknowledgement (protocol 1.8, ADR-0099; Task 006-044 of specs/006-sdk-multilanguage/tasks.md;
-- `stream-completion.md`, "The marks"): the columns the claim, the acknowledgement and the withdrawals of 1.8 write. Additive
-- and dormant: nothing writes them until Task 006-047, so no row is backfilled, and no `CHECK` constrains a mark (a constraint
-- on a new column would scan `steps` under its lock; every assignment is checked statically instead, Task 006-047). Each
-- `ADD COLUMN` is nullable, or carries a constant default, so it changes the catalog only and rewrites no row. The table-level
-- grants of `sagat_app` on `steps` and `worker_sessions` cover the new columns. This migration touches no closed capability
-- check: the registry's part is Task 006-046's.
--
-- On `steps`:
--   lease_hand_back_generation   the generation of a lease claimed for a session at 1.8 or later: its hand-back is reserved
--   lease_offered_generation     the generation of a lease marked provisional (`acknowledge_before_start`)
--   lease_capability_generation  the generation of a lease to which a capability was served
--   lease_claimed_at             when the lease was claimed, on the database's clock
--   lease_claim_incarnation      `pg_postmaster_start_time()` at the claim: a mark of another server incarnation never counts
--   lease_acknowledged_at        when the host's acknowledgement committed
--   provisional_withdrawn_attempt the attempt whose provisional lease was withdrawn: its next lease is not flagged
--   withdrawn_lease_token_hash   the SHA-256 of the lease token of the last withdrawal of a marked lease (32 bytes, written
--                                only by the server), so a call that follows it is told apart
--   withdrawn_on_hand_back       whether that withdrawal was the host's hand-back
-- On `worker_sessions`:
--   declines_acknowledgement     the session asked for no flagged lease (its fallback); set by each registration
ALTER TABLE steps
    ADD COLUMN lease_hand_back_generation bigint,
    ADD COLUMN lease_offered_generation bigint,
    ADD COLUMN lease_capability_generation bigint,
    ADD COLUMN lease_claimed_at timestamptz,
    ADD COLUMN lease_claim_incarnation timestamptz,
    ADD COLUMN lease_acknowledged_at timestamptz,
    ADD COLUMN provisional_withdrawn_attempt integer,
    ADD COLUMN withdrawn_lease_token_hash bytea,
    ADD COLUMN withdrawn_on_hand_back boolean;

ALTER TABLE worker_sessions
    ADD COLUMN declines_acknowledgement boolean NOT NULL DEFAULT false;
