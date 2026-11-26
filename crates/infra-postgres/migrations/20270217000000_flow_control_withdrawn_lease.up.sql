-- Feature 022 (decisions D022-138 and D022-139; ADR-0096 decision 6, ADR-0098 decision 3): a lease the server withdraws
-- (`step.lease_withdrawn`, feature 006) leaves its gated step granted and not leased. Its rate grant must then count as
-- unleased again, so the withdrawal's commit clears the lease instant the scheduler may have copied onto it, and the
-- scheduler records the attempt's next lease. The guard of `20270130000100_flow_control_rate_limits` allowed only the one
-- recording of the lease instant; it now also allows that clear, and nothing else: every other column unchanged.
--
-- The clear is written even when the instant is not recorded yet (a write that always makes a new row version): a lease
-- pass that read the row before the withdrawal then finds another version on its recheck and does not write the
-- withdrawn lease (D022-139). A run's grant cannot be cleared: its shape check requires `leased_at = granted_at`.
--
-- Additive: the function is replaced under its name, the triggers that call it are unchanged; no row changes. The down
-- restores the guard of `20270130000100`.
CREATE OR REPLACE FUNCTION rate_limit_rows_guard() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    -- a field is read only from the table that has it: a waiter has no lease instant
    IF TG_TABLE_NAME <> 'rate_limit_grants' THEN
        RAISE EXCEPTION 'a rate-limit waiter is never changed'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF (NEW.tenant_id, NEW.namespace_id, NEW.run_id, NEW.node, NEW.attempt, NEW.pool, NEW.limit_scope,
        NEW.group_name, NEW.key_digest, NEW.declared_limit, NEW.period_ms, NEW.granted_at, NEW.shard)
       IS NOT DISTINCT FROM
       (OLD.tenant_id, OLD.namespace_id, OLD.run_id, OLD.node, OLD.attempt, OLD.pool, OLD.limit_scope,
        OLD.group_name, OLD.key_digest, OLD.declared_limit, OLD.period_ms, OLD.granted_at, OLD.shard)
       -- the recording of the lease instant, once; or its clear by a withdrawal of that lease
       AND ((OLD.leased_at IS NULL AND NEW.leased_at IS NOT NULL) OR NEW.leased_at IS NULL) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'a rate-limit waiter is never changed, and a grant only records its lease instant once or clears it'
        USING ERRCODE = 'restrict_violation';
END
$$;
