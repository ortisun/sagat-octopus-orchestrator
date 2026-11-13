-- Feature 018: the recovery of an outage is planned once and its limits hold across the steps that
-- carry it out (FR-018-008). A step acts on one time, so without a record the next step plans the
-- same backlog again and the ceiling of one recovery (`catchupMode` ONE: one action; ALL: at most
-- `catchup_actions`) is never reached: every slice of the backlog would act again.
--
-- `recovery_until` is the instant the recovery's times end (the time of the pass that planned it:
-- what falls due later belongs to the next recovery) and `recovery_acts` how many of them may still
-- start a run. Both are null for a schedule that is not in the middle of a recovery, which is every
-- schedule that exists. Additive: older code neither reads nor writes them.
ALTER TABLE schedules
    ADD COLUMN recovery_until timestamptz,
    ADD COLUMN recovery_acts  integer;

ALTER TABLE schedules ADD CONSTRAINT schedules_recovery_whole CHECK (
    (recovery_until IS NULL) = (recovery_acts IS NULL)) NOT VALID;
ALTER TABLE schedules VALIDATE CONSTRAINT schedules_recovery_whole;
ALTER TABLE schedules ADD CONSTRAINT schedules_recovery_acts_bounded CHECK (
    recovery_acts IS NULL OR recovery_acts BETWEEN 0 AND 1000) NOT VALID;
ALTER TABLE schedules VALIDATE CONSTRAINT schedules_recovery_acts_bounded;
