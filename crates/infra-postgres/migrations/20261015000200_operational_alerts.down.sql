DROP INDEX runs_live_by_status_idx;
DROP INDEX inbox_events_dead_idx;
DROP TABLE operational_alerts;
-- Subscriptions that ask for alerts cannot be kept by the older constraint: they are narrowed to
-- the one kind it knows, or removed if they asked for nothing else.
UPDATE webhook_subscriptions SET kinds = array_remove(array_remove(kinds, 'ops.alert.fired'), 'ops.alert.resolved');
DELETE FROM webhook_subscriptions WHERE cardinality(kinds) = 0;
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_kinds_valid;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_kinds_valid CHECK (
    cardinality(kinds) BETWEEN 1 AND 8 AND kinds <@ ARRAY['run.finished']);
