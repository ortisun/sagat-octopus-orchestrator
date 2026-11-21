-- Feature 035, D035-139: the credential pin of an attempt is keyed by the role of the secret. A SQL attempt pins its one secret as
-- `primary`; an Iceberg attempt pins two, the catalog's and the storage's, each compared across the two phases of the resolve,
-- pinned and audited on its own.
--
-- Additive: every existing row reads as `primary`, which is what every pin has been so far, and an older engine inserts with the
-- default and reads only rows of that role (a lake attempt cannot exist for it). The primary key gains the role, so the index is
-- rebuilt: the table holds one row per SQL attempt that has not yet been cleaned up, and the lock is held for that rebuild.
-- The grants of the table and its immutability trigger already cover the new column.

ALTER TABLE step_database_credentials ADD COLUMN secret_role text NOT NULL DEFAULT 'primary';
ALTER TABLE step_database_credentials ADD CONSTRAINT step_database_credentials_role_valid
    CHECK (secret_role IN ('primary', 'catalog', 'storage'));
ALTER TABLE step_database_credentials DROP CONSTRAINT step_database_credentials_pkey;
ALTER TABLE step_database_credentials
    ADD PRIMARY KEY (tenant_id, run_id, node_id, attempt, generation, secret_role);
