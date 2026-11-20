-- Rolls back 20270204000000_start_release_deferrals. A row only holds a refused release back from the pass until its
-- retry instant; nothing an older build reads is in it, and without the table the pass simply tries the run again at
-- once. Dropping it loses no run data.
DROP TABLE start_release_deferrals;
