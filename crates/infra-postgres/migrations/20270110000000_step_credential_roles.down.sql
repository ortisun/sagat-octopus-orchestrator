-- An older engine reads one pin per attempt: with an Iceberg attempt's second pin in the table it would read the wrong one or
-- none. Roll back only when every pin is the primary one; an operator lets the lake attempts finish and be cleaned up first.
-- Nothing may pin a catalog or storage secret between the check and the drop.
LOCK TABLE step_database_credentials IN SHARE ROW EXCLUSIVE MODE;

DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM step_database_credentials WHERE secret_role <> 'primary') THEN
        RAISE EXCEPTION 'cannot roll back step credential roles while a catalog or storage secret is pinned'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$guard$;

ALTER TABLE step_database_credentials DROP CONSTRAINT step_database_credentials_pkey;
ALTER TABLE step_database_credentials ADD PRIMARY KEY (tenant_id, run_id, node_id, attempt, generation);
ALTER TABLE step_database_credentials DROP CONSTRAINT step_database_credentials_role_valid;
ALTER TABLE step_database_credentials DROP COLUMN secret_role;
