DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM workflows WHERE internal) OR EXISTS (SELECT 1 FROM runs WHERE origin IS NOT NULL) THEN
        RAISE EXCEPTION 'cannot downgrade retained private executions';
    END IF;
END $$;
DROP TRIGGER runs_private_definition_scope ON runs;
DROP FUNCTION runs_private_definition_guard();
ALTER TABLE runs DROP COLUMN origin;
DROP TABLE private_execution_definitions;
DROP FUNCTION private_execution_definition_guard();
DROP TRIGGER workflows_internal_immutable ON workflows;
DROP FUNCTION workflows_internal_guard();
ALTER TABLE workflows DROP COLUMN internal;
