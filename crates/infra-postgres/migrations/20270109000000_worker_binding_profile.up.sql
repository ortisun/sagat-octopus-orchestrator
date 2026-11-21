-- Feature 035, D035-057: a dedicated runner is of one profile. The binding carries it, so a runner bound to `*`
-- still registers only as its own profile, and a stolen lake token cannot ask for PostgreSQL credentials.
--
-- Additive: every existing row reads as `database`, which is what every dedicated runner has been so far, and an
-- older engine ignores the column. The default is a constant, so the column is added without rewriting the table;
-- the CHECK reads the (small) table once. All the rows of one identity in one namespace carry the same value:
-- the store replaces them together under the identity's advisory lock. The sagat_app grants of the table already
-- cover the column.

ALTER TABLE worker_bindings ADD COLUMN profile text NOT NULL DEFAULT 'database';
ALTER TABLE worker_bindings ADD CONSTRAINT worker_bindings_profile_valid
    CHECK (profile IN ('database', 'lake'));
