-- A pin is what lets a lake attempt read a blob. An older engine leases no lake step, so a pin it cannot read would be an attempt it
-- cannot serve: roll back only when no attempt holds one. An operator lets the lake attempts finish and their runs be cleaned up
-- first. Nothing may pin a blob between the check and the drop.
LOCK TABLE step_lease_blobs IN SHARE ROW EXCLUSIVE MODE;

DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM step_lease_blobs) THEN
        RAISE EXCEPTION 'cannot roll back lake blob pins while an attempt holds one'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$guard$;

DROP TABLE step_lease_blobs;
DROP FUNCTION sagat_lake_blob_pin_immutable();
