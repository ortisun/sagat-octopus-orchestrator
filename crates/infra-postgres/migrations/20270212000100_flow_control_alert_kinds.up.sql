-- Feature 022, task T022-082 (decision D022-077 items 4, 6 and 7): the engine's monitor raises four flow-control alerts,
-- `CONCURRENCY_QUEUE_STUCK`, `RATE_LIMIT_GRANT_LAGGING`, `RATE_LIMIT_QUEUE_SATURATED` and `FLOW_CONTROL_WAITER_ORPHANED`
-- (`docs/runbooks/alerts.md`, `docs/runbooks/flow-control.md` sections 8 to 10 and 14). Their subject is the key's group,
-- never a key's value or label, within the 120 characters the table already bounds.
--
-- The only change of the schema is the list of kinds the CHECK holds closed. Additive: an older reader of the alert
-- listing reads the kind as text (`AlertKind::from_str` refuses a name it does not know, so roll the engines and the
-- control planes forward before the monitor of this build raises one: an older control plane's listing of a namespace
-- that holds one answers an error until it is upgraded); no older writer writes them. The CHECK is added `NOT VALID`,
-- so this statement takes no scan under its `ACCESS EXCLUSIVE` lock; `20270212000200` validates it under `SHARE UPDATE
-- EXCLUSIVE` (the rule of design 043, D043-145, D043-153). No table, column, index or grant changes (the runtime role
-- already writes the table), so nothing is classified.
ALTER TABLE operational_alerts DROP CONSTRAINT operational_alerts_kind_valid;
ALTER TABLE operational_alerts ADD CONSTRAINT operational_alerts_kind_valid CHECK (kind IN
    ('OUTBOX_STUCK', 'LEASE_NOT_REAPED', 'STEP_UNCLAIMED', 'REVIEW_PENDING', 'DEAD_ITEMS',
     'CONCURRENCY_QUEUE_STUCK', 'RATE_LIMIT_GRANT_LAGGING', 'RATE_LIMIT_QUEUE_SATURATED',
     'FLOW_CONTROL_WAITER_ORPHANED')) NOT VALID;
