-- Gap G48 (review follow-up): the claim query must leave out the steps a worker is not entitled to
-- receive, or enough of them ahead of the rest fill its bounded candidate window. The names of the
-- secrets a step declares are static (they come from the pinned workflow version), so they are copied
-- onto the step row when it is created. NULL: the step declares none, or it was created before this
-- migration (those are still refused by the claim transaction itself, they are only not left out of
-- the candidate window).
ALTER TABLE steps ADD COLUMN secrets text[];
ALTER TABLE steps ADD CONSTRAINT steps_secrets_bounded
    CHECK (secrets IS NULL OR (cardinality(secrets) BETWEEN 1 AND 16
                               AND octet_length(array_to_string(secrets, ',')) <= 1039));
