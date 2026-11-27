-- Restores the schema before `20270205000000_flow_control_key_listing`. Nothing is lost: `oldest_wait_at` is derived
-- from the waiters, which stay.
CREATE INDEX concurrency_holders_key_idx
    ON concurrency_holders (tenant_id, namespace_id, pool, holder_scope, group_name, key_digest);
DROP INDEX concurrency_holders_key_age_idx;
DROP INDEX concurrency_semaphores_group_listing_idx;
DROP INDEX concurrency_semaphores_listing_idx;

CREATE OR REPLACE FUNCTION concurrency_semaphore_count() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    holder_delta integer := 0;
    waiter_delta integer := 0;
    k_tenant uuid; k_namespace uuid; k_pool text; k_scope text; k_group text; k_digest bytea;
    changed integer;
BEGIN
    IF TG_TABLE_NAME = 'concurrency_holders' THEN
        holder_delta := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    ELSE
        waiter_delta := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    END IF;
    -- a field is read only from the table that has it: PL/pgSQL resolves every field an expression names
    IF TG_OP = 'INSERT' THEN
        k_tenant := NEW.tenant_id; k_namespace := NEW.namespace_id; k_pool := NEW.pool; k_group := NEW.group_name;
        k_digest := NEW.key_digest;
        IF TG_TABLE_NAME = 'concurrency_holders' THEN
            k_scope := NEW.holder_scope;
        ELSE
            k_scope := NEW.permit_scope;
        END IF;
    ELSE
        k_tenant := OLD.tenant_id; k_namespace := OLD.namespace_id; k_pool := OLD.pool; k_group := OLD.group_name;
        k_digest := OLD.key_digest;
        IF TG_TABLE_NAME = 'concurrency_holders' THEN
            k_scope := OLD.holder_scope;
        ELSE
            k_scope := OLD.permit_scope;
        END IF;
    END IF;
    UPDATE public.concurrency_semaphores s
       SET holders = s.holders + holder_delta,
           waiters = s.waiters + waiter_delta,
           updated_at = clock_timestamp()
     WHERE s.tenant_id = k_tenant AND s.namespace_id = k_namespace AND s.pool = k_pool AND s.permit_scope = k_scope
       AND s.group_name = k_group AND s.key_digest = k_digest;
    GET DIAGNOSTICS changed = ROW_COUNT;
    IF changed <> 1 THEN
        RAISE EXCEPTION 'a concurrency holder or waiter has no semaphore row'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION concurrency_semaphore_count() FROM PUBLIC;

ALTER TABLE concurrency_semaphores DROP CONSTRAINT concurrency_semaphores_oldest_wait_iff_waiters;
ALTER TABLE concurrency_semaphores DROP COLUMN oldest_wait_at;
