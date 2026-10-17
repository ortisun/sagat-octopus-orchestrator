DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM database_connections) OR EXISTS (SELECT 1 FROM run_database_connections) THEN
        RAISE EXCEPTION 'database connections must be exported and removed before downgrade';
    END IF;
END $$;
DROP TABLE run_database_connections;
ALTER TABLE runs DROP CONSTRAINT runs_database_scope_unique;
ALTER TABLE database_connections DROP CONSTRAINT database_connection_head_fk;
DROP TABLE database_connection_revisions;
DROP TABLE database_connections;
DROP FUNCTION sagat_database_revision_immutable();
DROP FUNCTION sagat_database_identity_immutable();
