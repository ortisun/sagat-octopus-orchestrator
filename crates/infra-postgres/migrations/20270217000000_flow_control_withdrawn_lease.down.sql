-- Restores the guard of `20270130000100_flow_control_rate_limits`: a grant records its lease instant once and is never
-- cleared. No row changes; a grant cleared by a withdrawal stays unleased until the scheduler records its next lease.
CREATE OR REPLACE FUNCTION rate_limit_rows_guard() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    -- a field is read only from the table that has it: a waiter has no lease instant
    IF TG_TABLE_NAME <> 'rate_limit_grants' THEN
        RAISE EXCEPTION 'a rate-limit waiter is never changed'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.leased_at IS NULL AND NEW.leased_at IS NOT NULL
       AND (NEW.tenant_id, NEW.namespace_id, NEW.run_id, NEW.node, NEW.attempt, NEW.pool, NEW.limit_scope,
            NEW.group_name, NEW.key_digest, NEW.declared_limit, NEW.period_ms, NEW.granted_at, NEW.shard)
           IS NOT DISTINCT FROM
           (OLD.tenant_id, OLD.namespace_id, OLD.run_id, OLD.node, OLD.attempt, OLD.pool, OLD.limit_scope,
            OLD.group_name, OLD.key_digest, OLD.declared_limit, OLD.period_ms, OLD.granted_at, OLD.shard) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'a rate-limit waiter is never changed, and a grant only records its lease instant once'
        USING ERRCODE = 'restrict_violation';
END
$$;
