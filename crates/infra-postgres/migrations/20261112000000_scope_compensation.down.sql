DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM scope_sagas) THEN
        RAISE EXCEPTION 'scope compensation is retained by a run';
    END IF;
END $$;

DROP TABLE scope_saga_entries;
DROP TABLE scope_sagas;
DROP FUNCTION scope_saga_after_valid(integer[], text[]);
