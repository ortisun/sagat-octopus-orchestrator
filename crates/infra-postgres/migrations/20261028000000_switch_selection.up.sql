ALTER TABLE steps
    ADD COLUMN switch_case_id text,
    ADD CONSTRAINT steps_switch_case_id_valid CHECK (
        switch_case_id IS NULL
        OR switch_case_id = ''
        OR switch_case_id ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
    );
