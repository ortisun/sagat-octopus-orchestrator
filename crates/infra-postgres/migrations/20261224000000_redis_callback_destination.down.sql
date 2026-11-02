DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM webhook_subscriptions WHERE destination_kind = 'REDIS') THEN
        RAISE EXCEPTION USING ERRCODE = '55000', MESSAGE = 'cannot roll back Redis callbacks while Redis subscription history exists';
    END IF;
END
$guard$;
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_destination_kind_valid;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_destination_kind_valid
    CHECK (destination_kind IN ('WEBHOOK', 'NATS', 'KAFKA', 'SQS'));
