-- Undoes 20270130000400_flow_control_group_declarations.up.sql. Rollback is an operator/quiesced-service operation. A
-- declaration row is derived from its version's document, which stays; but a version that declares a group is a
-- version an older build refuses to admit, so the rows are evidence the operator must see: rolling back refuses while
-- one exists.
LOCK TABLE flow_control_group_declarations IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM flow_control_group_declarations) THEN
        RAISE EXCEPTION 'published versions declare flow-control groups; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TABLE flow_control_group_declarations;
DROP FUNCTION flow_control_group_declarations_immutable();
