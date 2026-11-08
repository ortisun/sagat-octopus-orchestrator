-- A pin is what lets a lake attempt read a blob. An older engine leases no lake step, so a pin it cannot read would be an attempt it
-- cannot serve: roll back only when no pin exists. Pins stay as long as their run, and nothing purges a run today, so once a lake step
-- that names blobs has been leased this rollback is closed until an operator, as the owner, deletes the pins of attempts that are over
-- (`DELETE FROM step_lease_blobs`, once no lake attempt is in flight): a pin of an attempt that has ended authorizes nothing, and the
-- reads it allowed are in the audit trail (`blob.read`). Nothing may pin a blob between the check and the drop.
LOCK TABLE step_lease_blobs IN SHARE ROW EXCLUSIVE MODE;

DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM step_lease_blobs) THEN
        RAISE EXCEPTION 'cannot roll back lake blob pins while a pin exists'
            USING ERRCODE = 'object_not_in_prerequisite_state',
                  HINT = 'let the lake attempts finish, then delete the rows of step_lease_blobs as the owner: a pin of an attempt that is over authorizes nothing';
    END IF;
END
$guard$;

DROP TABLE step_lease_blobs;
DROP FUNCTION sagat_lake_blob_pin_immutable();
