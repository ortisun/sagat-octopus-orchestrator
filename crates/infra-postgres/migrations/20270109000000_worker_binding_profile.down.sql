-- An older engine does not read the profile, so a binding that is not `database` would stop being restricted to its
-- profile. Roll back only when every binding is the default; an operator re-binds the lake runners first.
-- Nothing may bind a lake runner between the check and the drop.
LOCK TABLE worker_bindings IN SHARE ROW EXCLUSIVE MODE;

DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM worker_bindings WHERE profile <> 'database') THEN
        RAISE EXCEPTION 'cannot roll back worker binding profiles while a binding is not of the database profile'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$guard$;

ALTER TABLE worker_bindings DROP CONSTRAINT worker_bindings_profile_valid;
ALTER TABLE worker_bindings DROP COLUMN profile;
