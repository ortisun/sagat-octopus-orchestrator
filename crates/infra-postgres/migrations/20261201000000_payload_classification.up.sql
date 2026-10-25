-- Existing immutable versions retain legacy structural disclosure. No document, digest or
-- version row is updated; new publications explicitly insert the current interpreter rules.
ALTER TABLE workflow_versions
    ADD COLUMN classification_rules smallint NOT NULL DEFAULT 0
    CHECK (classification_rules IN (0, 1));
