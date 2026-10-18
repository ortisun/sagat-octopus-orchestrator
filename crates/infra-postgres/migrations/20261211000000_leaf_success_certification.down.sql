-- Match own-run-before-certificate order; all checks/drop operations share one migration TX.
LOCK TABLE runs IN ACCESS EXCLUSIVE MODE;
LOCK TABLE leaf_collection_success_certifications IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM leaf_collection_success_certifications) THEN
        RAISE EXCEPTION 'leaf certifications are retained; keep a compatible reader until original source and receipt retention has a compatible migration path'
            USING ERRCODE='55000';
    END IF;
END $guard$;
DROP TABLE leaf_collection_success_certifications;
DO $retention$
DECLARE source_table text;
BEGIN
    FOREACH source_table IN ARRAY ARRAY['runs','steps','iteration_scopes','collection_scope_items',
        'collection_iterations','iteration_checkpoints','scope_sagas','scope_saga_entries'] LOOP
        EXECUTE format('DROP TRIGGER leaf_source_no_delete ON %I',source_table);
        EXECUTE format('DROP TRIGGER leaf_source_no_truncate ON %I',source_table);
    END LOOP;
END $retention$;
DROP TRIGGER leaf_source_run_identity ON runs;
DROP FUNCTION leaf_source_run_identity_guard();
