-- Reverses 20270218000000_requirement_hold. Refused while a design has registered a floor: that design's own down migration
-- removes its row and its function first, so a rollback never leaves a capability whose floor nobody enforces.
DO $refuse$
BEGIN
    IF EXISTS (SELECT 1 FROM platform_requirement_floors) THEN
        RAISE EXCEPTION 'platform_requirement_floors still holds a floor: roll back the migration that registered it first'
            USING ERRCODE = 'restrict_violation';
    END IF;
END
$refuse$;

DROP FUNCTION platform_requirement_hold(text);
DROP TRIGGER platform_requirement_floor ON platform_requirements;
DROP TABLE platform_requirement_floors;
DROP FUNCTION platform_requirement_floor();
DROP FUNCTION platform_requirement_floors_check();
