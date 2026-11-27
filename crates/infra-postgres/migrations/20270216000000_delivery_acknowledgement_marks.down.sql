-- Rollback of `20270216000000_delivery_acknowledgement_marks`, an operator/quiesced-service operation: refused while a
-- provisional mark could still count (a `LEASED` step whose lease is the one marked provisional), because a binary without
-- the marks would treat that lease as an ordinary one, which its host was told not to start before an acknowledgement it can
-- no longer receive. Otherwise the columns go: nothing else reads them.
LOCK TABLE steps IN SHARE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM steps
                WHERE status = 'LEASED' AND lease_offered_generation IS NOT NULL
                  AND lease_offered_generation = generation) THEN
        RAISE EXCEPTION 'a LEASED step holds a provisional mark that can still count; wait for its lease to end, or roll the binaries back first and let the lease lapse'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE worker_sessions DROP COLUMN declines_acknowledgement;
ALTER TABLE steps
    DROP COLUMN withdrawn_on_hand_back,
    DROP COLUMN withdrawn_lease_token_hash,
    DROP COLUMN provisional_withdrawn_attempt,
    DROP COLUMN lease_acknowledged_at,
    DROP COLUMN lease_claim_incarnation,
    DROP COLUMN lease_claimed_at,
    DROP COLUMN lease_capability_generation,
    DROP COLUMN lease_offered_generation,
    DROP COLUMN lease_hand_back_generation;
