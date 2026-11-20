-- Feature 033. Activation follows a cold upgrade of every outbound reader/writer.
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_destination_kind_valid;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_destination_kind_valid
    CHECK (destination_kind IN ('WEBHOOK', 'NATS', 'KAFKA', 'SQS', 'REDIS'));
