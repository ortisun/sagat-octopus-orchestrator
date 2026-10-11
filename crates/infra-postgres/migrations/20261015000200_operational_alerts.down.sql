DROP INDEX runs_live_by_status_idx;
DROP INDEX inbox_events_dead_idx;
DROP TABLE operational_alerts;
-- Subscriptions that ask for alerts cannot be kept by the older constraint: those that asked for
-- nothing else are removed first (narrowing them would leave no kind, which the constraint still in
-- force refuses), and the rest are narrowed to the one kind it knows.
-- The notifications of alerts that were not delivered yet go with it: the code that comes back
-- does not know how to deliver them.
DELETE FROM webhook_deliveries WHERE kind IN ('ops.alert.fired', 'ops.alert.resolved');
DELETE FROM webhook_subscriptions WHERE kinds <@ ARRAY['ops.alert.fired', 'ops.alert.resolved'];
UPDATE webhook_subscriptions SET kinds = array_remove(array_remove(kinds, 'ops.alert.fired'), 'ops.alert.resolved');
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_kinds_valid;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_kinds_valid CHECK (
    cardinality(kinds) BETWEEN 1 AND 8 AND kinds <@ ARRAY['run.finished']);
