-- Feature 022 (T022-082): the CHECK 20270213000300 restated `NOT VALID` is validated under `SHARE UPDATE EXCLUSIVE`, so
-- the monitor and the listings go on while it is scanned (the rule of design 043, D043-145, D043-153).
ALTER TABLE operational_alerts VALIDATE CONSTRAINT operational_alerts_kind_valid;
